mod common;

use common::{
    apply_fault_profile, cluster_file, load_replay_trace, maybe_save_trace, trace_path,
    unique_context,
};
use pelago_core::schema::{EdgeDef, EdgeTarget, EntitySchema, IndexType, PropertyDef};
use pelago_core::{PropertyType, Value};
use pelago_storage::{
    read_cdc_entries, CdcConsumer, CdcOperation, ConsumerConfig, EdgeStore, FaultMode,
    FaultProfile, IdAllocator, NodeRef, NodeStore, ScenarioEvent, ScenarioEventStatus,
    ScenarioOperation, SchemaCache, SchemaRegistry, SimulationConfig, SimulationTrace,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

fn parse_seed(default: u64) -> u64 {
    std::env::var("PELAGO_SIM_SEED")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn parse_steps(default: u32) -> u32 {
    std::env::var("PELAGO_SIM_STEPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(default)
}

fn parse_workers(default: u16) -> u16 {
    std::env::var("PELAGO_SIM_WORKERS")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(default)
}

fn sim_config(scenario: &str, default_seed: u64, steps: u32, workers: u16) -> SimulationConfig {
    SimulationConfig::new(
        parse_seed(default_seed),
        parse_steps(steps),
        parse_workers(workers),
        scenario,
    )
}

fn build_storage_fault_profile() -> FaultProfile {
    let mut failpoints = HashMap::new();
    failpoints.insert(
        "tx.commit.before_action.create_node".to_string(),
        FaultMode::ReturnError {
            max_triggers: Some(1),
        },
    );
    FaultProfile { failpoints }
}

fn replay_trace_from_env() -> Option<SimulationTrace> {
    let path = std::env::var("PELAGO_SIM_REPLAY_TRACE").ok()?;
    Some(load_replay_trace(path.as_ref()))
}

fn generate_operations(config: &SimulationConfig) -> Vec<ScenarioOperation> {
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut ops = Vec::with_capacity(config.steps as usize);
    let slots = (config.steps.max(8) as usize).min(128);
    let workers = config.workers.max(1);

    for step in 0..config.steps {
        let worker = rng.gen_range(0..workers);
        let op = match rng.gen_range(0..5) {
            0 => ScenarioOperation::CreateNode {
                worker,
                label: format!("u{}", step),
                age: rng.gen_range(18..80),
            },
            1 => ScenarioOperation::UpdateNode {
                worker,
                slot: rng.gen_range(0..slots),
                age: rng.gen_range(18..90),
            },
            2 => ScenarioOperation::DeleteNode {
                worker,
                slot: rng.gen_range(0..slots),
            },
            3 => ScenarioOperation::CreateEdge {
                worker,
                source_slot: rng.gen_range(0..slots),
                target_slot: rng.gen_range(0..slots),
            },
            _ => ScenarioOperation::DeleteEdge {
                worker,
                source_slot: rng.gen_range(0..slots),
                target_slot: rng.gen_range(0..slots),
            },
        };
        ops.push(op);
    }
    ops
}

#[test]
fn test_sim_seed_reproducible_operation_stream() {
    let cfg = SimulationConfig::new(123_456, 64, 3, "determinism");
    let a = generate_operations(&cfg);
    let b = generate_operations(&cfg);
    assert_eq!(a, b, "same seed must generate identical operation stream");

    let trace = SimulationTrace {
        config: cfg,
        operations: a.clone(),
        events: Vec::new(),
    };
    let encoded = serde_json::to_vec(&trace).expect("trace should serialize");
    let decoded: SimulationTrace =
        serde_json::from_slice(&encoded).expect("trace should deserialize");
    assert_eq!(decoded.operations, a, "trace replay payload must be stable");
}

fn push_event(
    events: &mut Vec<ScenarioEvent>,
    step: u32,
    worker: u16,
    operation: impl Into<String>,
    status: ScenarioEventStatus,
    detail: impl Into<String>,
) {
    events.push(ScenarioEvent {
        step,
        worker,
        operation: operation.into(),
        status,
        detail: detail.into(),
    });
}

async fn setup_storage(
    database: &str,
    namespace: &str,
    site_id: &str,
    register_schema: bool,
) -> (
    pelago_storage::PelagoDb,
    Arc<SchemaRegistry>,
    Arc<NodeStore>,
    EdgeStore,
) {
    let db = pelago_storage::PelagoDb::connect(&cluster_file())
        .await
        .expect("FDB should connect");
    let schema_cache = Arc::new(SchemaCache::new());
    let schema_registry = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&schema_cache),
        site_id.to_string(),
    ));
    let id_allocator = Arc::new(IdAllocator::new(
        db.clone(),
        site_id.parse::<u8>().expect("site id should parse"),
        100,
    ));
    let node_store = Arc::new(NodeStore::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        site_id.to_string(),
    ));
    let edge_store = EdgeStore::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        Arc::clone(&node_store),
        site_id.to_string(),
    );

    if register_schema {
        let person_schema = EntitySchema::new("Person")
            .with_property(
                "email",
                PropertyDef::new(PropertyType::String)
                    .required()
                    .with_index(IndexType::Unique),
            )
            .with_property("name", PropertyDef::new(PropertyType::String).required())
            .with_property(
                "age",
                PropertyDef::new(PropertyType::Int).with_index(IndexType::Range),
            )
            .with_edge("knows", EdgeDef::new(EdgeTarget::specific("Person")));
        schema_registry
            .register_schema(database, namespace, person_schema)
            .await
            .expect("schema should register");
    }

    (db, schema_registry, node_store, edge_store)
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_sim_storage_consistency_seeded() {
    let mut config = sim_config("storage_consistency", 17, 300, 4);
    config.fail_profile = build_storage_fault_profile();

    let replay = replay_trace_from_env();
    let operations = replay
        .as_ref()
        .map(|t| t.operations.clone())
        .unwrap_or_else(|| generate_operations(&config));
    if let Some(trace) = replay {
        config = trace.config;
    }

    apply_fault_profile(&config.fail_profile);

    let (database, namespace) = unique_context("sim_storage", config.seed);
    let (_db, _schema_registry, node_store, edge_store) =
        setup_storage(&database, &namespace, "1", true).await;

    let mut events = Vec::new();
    let mut slots: Vec<Option<String>> = vec![None; (config.steps as usize).max(8).min(128)];
    let mut known_ids = HashSet::new();
    let mut node_age = HashMap::new();
    let mut edges: HashSet<(String, String)> = HashSet::new();

    for (step, op) in operations.iter().enumerate() {
        match op {
            ScenarioOperation::CreateNode { worker, label, age } => {
                let unique_label = format!("{}_{}_{}", label, config.seed, step);
                let props = HashMap::from([
                    (
                        "email".to_string(),
                        Value::String(format!("{}@example.test", unique_label)),
                    ),
                    ("name".to_string(), Value::String(unique_label.clone())),
                    ("age".to_string(), Value::Int(*age)),
                ]);
                match node_store
                    .create_node(&database, &namespace, "Person", props)
                    .await
                {
                    Ok(node) => {
                        let slot = step % slots.len();
                        slots[slot] = Some(node.id.clone());
                        known_ids.insert(node.id.clone());
                        node_age.insert(node.id.clone(), *age);
                        push_event(
                            &mut events,
                            step as u32,
                            *worker,
                            "create_node",
                            ScenarioEventStatus::Applied,
                            node.id,
                        );
                    }
                    Err(err) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "create_node",
                        ScenarioEventStatus::Failed,
                        err.to_string(),
                    ),
                }
            }
            ScenarioOperation::UpdateNode { worker, slot, age } => {
                let Some(id) = slots.get(*slot % slots.len()).and_then(|v| v.clone()) else {
                    continue;
                };
                match node_store
                    .update_node(
                        &database,
                        &namespace,
                        "Person",
                        &id,
                        HashMap::from([("age".to_string(), Value::Int(*age))]),
                    )
                    .await
                {
                    Ok(_) => {
                        node_age.insert(id.clone(), *age);
                        push_event(
                            &mut events,
                            step as u32,
                            *worker,
                            "update_node",
                            ScenarioEventStatus::Applied,
                            id,
                        );
                    }
                    Err(err) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "update_node",
                        ScenarioEventStatus::Failed,
                        err.to_string(),
                    ),
                }
            }
            ScenarioOperation::DeleteNode { worker, slot } => {
                let Some(id) = slots.get(*slot % slots.len()).and_then(|v| v.clone()) else {
                    continue;
                };
                match node_store
                    .delete_node(&database, &namespace, "Person", &id)
                    .await
                {
                    Ok(true) => {
                        for v in &mut slots {
                            if v.as_deref() == Some(id.as_str()) {
                                *v = None;
                            }
                        }
                        node_age.remove(&id);
                        edges.retain(|(s, t)| s != &id && t != &id);
                        push_event(
                            &mut events,
                            step as u32,
                            *worker,
                            "delete_node",
                            ScenarioEventStatus::Applied,
                            id,
                        );
                    }
                    Ok(false) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "delete_node",
                        ScenarioEventStatus::Rejected,
                        "missing",
                    ),
                    Err(err) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "delete_node",
                        ScenarioEventStatus::Failed,
                        err.to_string(),
                    ),
                }
            }
            ScenarioOperation::CreateEdge {
                worker,
                source_slot,
                target_slot,
            } => {
                let Some(source_id) = slots
                    .get(*source_slot % slots.len())
                    .and_then(|v| v.as_ref())
                    .cloned()
                else {
                    continue;
                };
                let Some(target_id) = slots
                    .get(*target_slot % slots.len())
                    .and_then(|v| v.as_ref())
                    .cloned()
                else {
                    continue;
                };
                if source_id == target_id {
                    continue;
                }
                match edge_store
                    .create_edge(
                        &database,
                        &namespace,
                        NodeRef::new(&database, &namespace, "Person", &source_id),
                        NodeRef::new(&database, &namespace, "Person", &target_id),
                        "knows",
                        HashMap::new(),
                    )
                    .await
                {
                    Ok(_) => {
                        edges.insert((source_id.clone(), target_id.clone()));
                        push_event(
                            &mut events,
                            step as u32,
                            *worker,
                            "create_edge",
                            ScenarioEventStatus::Applied,
                            format!("{}->{}", source_id, target_id),
                        );
                    }
                    Err(err) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "create_edge",
                        ScenarioEventStatus::Failed,
                        err.to_string(),
                    ),
                }
            }
            ScenarioOperation::DeleteEdge {
                worker,
                source_slot,
                target_slot,
            } => {
                let Some(source_id) = slots
                    .get(*source_slot % slots.len())
                    .and_then(|v| v.as_ref())
                    .cloned()
                else {
                    continue;
                };
                let Some(target_id) = slots
                    .get(*target_slot % slots.len())
                    .and_then(|v| v.as_ref())
                    .cloned()
                else {
                    continue;
                };
                match edge_store
                    .delete_edge(
                        &database,
                        &namespace,
                        NodeRef::new(&database, &namespace, "Person", &source_id),
                        NodeRef::new(&database, &namespace, "Person", &target_id),
                        "knows",
                    )
                    .await
                {
                    Ok(true) => {
                        edges.remove(&(source_id.clone(), target_id.clone()));
                        push_event(
                            &mut events,
                            step as u32,
                            *worker,
                            "delete_edge",
                            ScenarioEventStatus::Applied,
                            format!("{}->{}", source_id, target_id),
                        );
                    }
                    Ok(false) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "delete_edge",
                        ScenarioEventStatus::Rejected,
                        "missing",
                    ),
                    Err(err) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "delete_edge",
                        ScenarioEventStatus::Failed,
                        err.to_string(),
                    ),
                }
            }
        }
    }

    // Invariant I1 + I3: model vs durable state.
    for id in &known_ids {
        let node = node_store
            .get_node(&database, &namespace, "Person", id)
            .await
            .expect("node lookup should succeed");
        let expected_live = node_age.contains_key(id);
        assert_eq!(
            node.is_some(),
            expected_live,
            "liveness mismatch for id={id}"
        );
        if let Some(stored) = node {
            let age = stored
                .properties
                .get("age")
                .and_then(|v| v.as_int())
                .expect("age should be int");
            assert_eq!(Some(age), node_age.get(id).copied());
        }
    }

    for (source_id, target_id) in &edges {
        let outgoing = edge_store
            .list_edges(
                &database,
                &namespace,
                "Person",
                source_id,
                Some("knows"),
                1000,
            )
            .await
            .expect("edge scan should succeed");
        assert!(
            outgoing.iter().any(|e| e.target.node_id == *target_id),
            "expected edge missing: {}->{}",
            source_id,
            target_id
        );
    }

    let trace = SimulationTrace {
        config: config.clone(),
        operations,
        events: events.clone(),
    };
    maybe_save_trace(&trace_path(&config.scenario, config.seed), &trace);
    apply_fault_profile(&FaultProfile::default());
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_sim_cdc_correctness_seeded() {
    let config = sim_config("cdc_correctness", 29, 120, 2);
    apply_fault_profile(&FaultProfile::default());

    let operations = replay_trace_from_env()
        .map(|t| t.operations)
        .unwrap_or_else(|| {
            // Restrict to node mutations so expected CDC op count stays exact.
            let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
            let mut ops = Vec::with_capacity(config.steps as usize);
            for step in 0..config.steps {
                let worker = (step as u16) % config.workers.max(1);
                let op = if rng.gen_bool(0.5) {
                    ScenarioOperation::CreateNode {
                        worker,
                        label: format!("cdc_{}", step),
                        age: rng.gen_range(18..70),
                    }
                } else if rng.gen_bool(0.5) {
                    ScenarioOperation::UpdateNode {
                        worker,
                        slot: rng.gen_range(0..32),
                        age: rng.gen_range(18..70),
                    }
                } else {
                    ScenarioOperation::DeleteNode {
                        worker,
                        slot: rng.gen_range(0..32),
                    }
                };
                ops.push(op);
            }
            ops
        });

    let (database, namespace) = unique_context("sim_cdc", config.seed);
    let (db, _schema_registry, node_store, _edge_store) =
        setup_storage(&database, &namespace, "1", true).await;
    let mut slots: Vec<Option<String>> = vec![None; 32];
    let mut events = Vec::new();

    for (step, op) in operations.iter().enumerate() {
        match op {
            ScenarioOperation::CreateNode { worker, label, age } => {
                let props = HashMap::from([
                    (
                        "email".to_string(),
                        Value::String(format!("{}-{}@example.test", label, step)),
                    ),
                    ("name".to_string(), Value::String(label.clone())),
                    ("age".to_string(), Value::Int(*age)),
                ]);
                match node_store
                    .create_node(&database, &namespace, "Person", props)
                    .await
                {
                    Ok(node) => {
                        let slot_ix = step % slots.len();
                        slots[slot_ix] = Some(node.id.clone());
                        push_event(
                            &mut events,
                            step as u32,
                            *worker,
                            "create_node",
                            ScenarioEventStatus::Applied,
                            node.id,
                        );
                    }
                    Err(err) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "create_node",
                        ScenarioEventStatus::Failed,
                        err.to_string(),
                    ),
                }
            }
            ScenarioOperation::UpdateNode { worker, slot, age } => {
                let Some(id) = slots.get(*slot % slots.len()).and_then(|v| v.clone()) else {
                    continue;
                };
                match node_store
                    .update_node(
                        &database,
                        &namespace,
                        "Person",
                        &id,
                        HashMap::from([("age".to_string(), Value::Int(*age))]),
                    )
                    .await
                {
                    Ok(_) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "update_node",
                        ScenarioEventStatus::Applied,
                        id,
                    ),
                    Err(err) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "update_node",
                        ScenarioEventStatus::Failed,
                        err.to_string(),
                    ),
                }
            }
            ScenarioOperation::DeleteNode { worker, slot } => {
                let Some(id) = slots.get(*slot % slots.len()).and_then(|v| v.clone()) else {
                    continue;
                };
                match node_store
                    .delete_node(&database, &namespace, "Person", &id)
                    .await
                {
                    Ok(true) => {
                        let slot_ix = *slot % slots.len();
                        slots[slot_ix] = None;
                        push_event(
                            &mut events,
                            step as u32,
                            *worker,
                            "delete_node",
                            ScenarioEventStatus::Applied,
                            id,
                        );
                    }
                    Ok(false) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "delete_node",
                        ScenarioEventStatus::Rejected,
                        "missing",
                    ),
                    Err(err) => push_event(
                        &mut events,
                        step as u32,
                        *worker,
                        "delete_node",
                        ScenarioEventStatus::Failed,
                        err.to_string(),
                    ),
                }
            }
            _ => {}
        }
    }

    let entries = read_cdc_entries(&db, &database, &namespace, None, 20_000)
        .await
        .expect("cdc read should succeed");
    for pair in entries.windows(2) {
        assert!(
            pair[0].0 < pair[1].0,
            "CDC versionstamps must be strictly ordered"
        );
    }

    let expected = events
        .iter()
        .filter(|evt| evt.status == ScenarioEventStatus::Applied)
        .filter(|evt| {
            matches!(
                evt.operation.as_str(),
                "create_node" | "update_node" | "delete_node"
            )
        })
        .count();
    let actual_mutations = entries
        .iter()
        .flat_map(|(_, entry)| entry.operations.iter())
        .filter(|op| {
            matches!(
                op,
                CdcOperation::NodeCreate { .. }
                    | CdcOperation::NodeUpdate { .. }
                    | CdcOperation::NodeDelete { .. }
            )
        })
        .count();
    assert_eq!(actual_mutations, expected, "CDC mutation count mismatch");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_sim_replication_conflict_idempotency() {
    let config = sim_config("replication_conflict", 43, 0, 1);
    apply_fault_profile(&FaultProfile::default());

    let (database, namespace) = unique_context("sim_repl", config.seed);
    let (_db, schema_registry_1, node_store_1, _edge_store_1) =
        setup_storage(&database, &namespace, "1", true).await;
    let (_db2, _schema_registry_2, node_store_2, _edge_store_2) =
        setup_storage(&database, &namespace, "2", false).await;

    // Ensure schema exists for both registries.
    assert!(schema_registry_1
        .get_schema(&database, &namespace, "Person")
        .await
        .expect("schema lookup should work")
        .is_some());

    let alice = node_store_1
        .create_node(
            &database,
            &namespace,
            "Person",
            HashMap::from([
                (
                    "email".to_string(),
                    Value::String("alice@site1.test".to_string()),
                ),
                ("name".to_string(), Value::String("Alice".to_string())),
                ("age".to_string(), Value::Int(30)),
            ]),
        )
        .await
        .expect("seed create should succeed");

    let old_props = alice.properties.clone();
    let stale = node_store_2
        .apply_replica_node_update(
            &database,
            &namespace,
            "Person",
            &alice.id,
            HashMap::from([("age".to_string(), Value::Int(31))]),
            old_props.clone(),
            alice.updated_at - 1,
            "2",
        )
        .await
        .expect("stale apply should return bool");
    assert!(!stale, "stale conflicting event should be rejected");

    let newer = node_store_2
        .apply_replica_node_update(
            &database,
            &namespace,
            "Person",
            &alice.id,
            HashMap::from([("age".to_string(), Value::Int(44))]),
            old_props,
            alice.updated_at + 100,
            "2",
        )
        .await
        .expect("newer apply should return bool");
    assert!(newer, "newer event should win via LWW fallback");

    let _idem = node_store_2
        .apply_replica_node_update(
            &database,
            &namespace,
            "Person",
            &alice.id,
            HashMap::from([("age".to_string(), Value::Int(44))]),
            HashMap::from([
                (
                    "email".to_string(),
                    Value::String("alice@site1.test".to_string()),
                ),
                ("name".to_string(), Value::String("Alice".to_string())),
                ("age".to_string(), Value::Int(44)),
            ]),
            alice.updated_at + 100,
            "2",
        )
        .await
        .expect("idempotent apply should return bool");

    let reloaded = node_store_1
        .get_node(&database, &namespace, "Person", &alice.id)
        .await
        .expect("lookup should succeed")
        .expect("node should exist");
    assert_eq!(
        reloaded.properties.get("age"),
        Some(&Value::Int(44)),
        "final replicated age should be 44"
    );
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_sim_consumer_resume_checkpoint_with_injected_failure() {
    let mut config = sim_config("resume_checkpoint", 71, 12, 1);
    let mut failpoints = HashMap::new();
    failpoints.insert(
        "consumer.checkpoint.before_write".to_string(),
        FaultMode::ReturnError {
            max_triggers: Some(1),
        },
    );
    config.fail_profile = FaultProfile { failpoints };
    apply_fault_profile(&config.fail_profile);

    let (database, namespace) = unique_context("sim_resume", config.seed);
    let (db, _schema_registry, node_store, _edge_store) =
        setup_storage(&database, &namespace, "1", true).await;

    for i in 0..4 {
        node_store
            .create_node(
                &database,
                &namespace,
                "Person",
                HashMap::from([
                    (
                        "email".to_string(),
                        Value::String(format!("resume_{i}@example.test")),
                    ),
                    ("name".to_string(), Value::String(format!("Resume{i}"))),
                    ("age".to_string(), Value::Int(20 + i as i64)),
                ]),
            )
            .await
            .expect("seed nodes should be created");
    }

    let cfg = ConsumerConfig::new("sim_resume_consumer", &database, &namespace)
        .with_batch_size(256)
        .with_checkpoint_interval(Duration::from_millis(0));
    let mut consumer = CdcConsumer::new(db.clone(), cfg.clone())
        .await
        .expect("consumer should start");
    let batch = consumer.poll_batch().await.expect("batch should load");
    assert!(!batch.is_empty(), "seed batch should not be empty");
    if cfg!(feature = "failpoints") {
        let ack_err = consumer
            .ack_batch(&batch)
            .await
            .expect_err("first ack should fail due to injected checkpoint failpoint");
        assert!(
            ack_err
                .to_string()
                .contains("Injected failpoint triggered: consumer.checkpoint.before_write"),
            "unexpected ack error: {ack_err}"
        );
    } else {
        consumer
            .ack_batch(&batch)
            .await
            .expect("ack should succeed when failpoints feature is disabled");
    }

    // Failpoint fires once, this explicit checkpoint should now persist.
    consumer
        .checkpoint()
        .await
        .expect("checkpoint retry should succeed");
    let saved_hwm = consumer.hwm().clone();

    // Restart consumer and ensure it resumes from saved checkpoint.
    let mut resumed = CdcConsumer::new(db.clone(), cfg)
        .await
        .expect("resumed consumer should start");
    assert_eq!(
        resumed.hwm(),
        &saved_hwm,
        "resume HWM should match checkpoint"
    );

    let extra = node_store
        .create_node(
            &database,
            &namespace,
            "Person",
            HashMap::from([
                (
                    "email".to_string(),
                    Value::String("resume_new@example.test".to_string()),
                ),
                ("name".to_string(), Value::String("ResumeNew".to_string())),
                ("age".to_string(), Value::Int(88)),
            ]),
        )
        .await
        .expect("new node should be created");

    let next_batch = resumed.poll_batch().await.expect("next batch should load");
    let has_new = next_batch.iter().any(|(_, entry)| {
        entry.operations.iter().any(|op| {
            matches!(
                op,
                CdcOperation::NodeCreate { node_id, .. } if node_id == &extra.id
            )
        })
    });
    assert!(
        has_new,
        "resumed consumer should observe only post-checkpoint history"
    );
    apply_fault_profile(&FaultProfile::default());
}
