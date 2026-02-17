//! Replication + security integration tests.
//!
//! Run with:
//!   LIBRARY_PATH=/usr/local/lib FDB_CLUSTER_FILE=/usr/local/etc/foundationdb/fdb.cluster \
//!   cargo test -p pelago-storage --test replication_security_integration -- --ignored --nocapture

use pelago_core::schema::{EdgeDef, EdgeTarget, EntitySchema, PropertyDef};
use pelago_core::{PropertyType, Value};
use pelago_storage::{
    append_audit_record, check_permission, claim_site, cleanup_audit_records,
    get_replication_positions, list_sites, query_audit_records, read_cdc_entries,
    update_replication_position, upsert_policy, AuditRecord, AuthPolicy, CdcOperation, EdgeStore,
    IdAllocator, NodeRef, NodeStore, PelagoDb, PolicyPermission, SchemaCache, SchemaRegistry,
    Versionstamp,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn cluster_file() -> String {
    std::env::var("FDB_CLUSTER_FILE")
        .unwrap_or_else(|_| "/usr/local/etc/foundationdb/fdb.cluster".to_string())
}

fn unique_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros()
        .to_string()
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_site_claim_and_replication_position() {
    let db = PelagoDb::connect(&cluster_file())
        .await
        .expect("Failed to connect to FDB");

    let suffix = unique_suffix();
    let site_id = format!("site_{}", suffix);
    let site_name = format!("integration-{}", suffix);

    let claim = claim_site(&db, &site_id, &site_name)
        .await
        .expect("claim_site should succeed");
    assert_eq!(claim.site_id, site_id);
    assert_eq!(claim.site_name, site_name);

    // Idempotent claim with the same owner should succeed.
    let claim2 = claim_site(&db, &site_id, &site_name)
        .await
        .expect("repeat claim_site should be idempotent");
    assert_eq!(claim2.site_id, claim.site_id);

    let sites = list_sites(&db).await.expect("list_sites should succeed");
    assert!(sites.iter().any(|s| s.site_id == site_id));

    let remote_site = format!("remote_{}", suffix);
    let vs = Versionstamp::zero().next();
    let pos = update_replication_position(&db, &remote_site, Some(vs.clone()), 7)
        .await
        .expect("update_replication_position should succeed");
    assert_eq!(pos.remote_site_id, remote_site);
    assert_eq!(pos.last_applied_versionstamp, Some(vs));
    assert_eq!(pos.lag_events, 7);

    let positions = get_replication_positions(&db)
        .await
        .expect("get_replication_positions should succeed");
    assert!(positions.iter().any(|p| p.remote_site_id == remote_site));
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_policy_check_and_audit_query() {
    let db = PelagoDb::connect(&cluster_file())
        .await
        .expect("Failed to connect to FDB");

    let suffix = unique_suffix();
    let principal = format!("principal_{}", suffix);
    let policy = AuthPolicy {
        policy_id: format!("policy_{}", suffix),
        principal_id: principal.clone(),
        permissions: vec![PolicyPermission {
            database: "db1".to_string(),
            namespace: "ns1".to_string(),
            entity_type: "Person".to_string(),
            actions: vec!["read".to_string(), "write".to_string()],
        }],
        created_at: 0,
    };

    upsert_policy(&db, &policy)
        .await
        .expect("upsert_policy should succeed");

    let allow = check_permission(&db, &principal, "read", "db1", "ns1", "Person")
        .await
        .expect("check_permission allow should succeed");
    assert!(allow);

    let deny = check_permission(&db, &principal, "delete", "db1", "ns1", "Person")
        .await
        .expect("check_permission deny should succeed");
    assert!(!deny);

    let audit = append_audit_record(
        &db,
        AuditRecord {
            event_id: String::new(),
            timestamp: 0,
            principal_id: principal.clone(),
            action: "read".to_string(),
            resource: "db1/ns1/Person".to_string(),
            allowed: true,
            reason: "integration".to_string(),
            metadata: HashMap::from([("test".to_string(), "replication_security".to_string())]),
        },
    )
    .await
    .expect("append_audit_record should succeed");

    let audit_rows = query_audit_records(&db, Some(&principal), Some("read"), None, None, 20)
        .await
        .expect("query_audit_records should succeed");
    assert!(audit_rows.iter().any(|r| r.event_id == audit.event_id));
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_ownership_enforcement_for_node_and_edge_mutations() {
    let db = PelagoDb::connect(&cluster_file())
        .await
        .expect("Failed to connect to FDB");

    let suffix = unique_suffix();
    let database = format!("ownership_{}", suffix);
    let namespace = "default".to_string();

    let schema_cache_1 = Arc::new(SchemaCache::new());
    let schema_cache_2 = Arc::new(SchemaCache::new());
    let schema_registry_1 = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&schema_cache_1),
        "1".to_string(),
    ));
    let schema_registry_2 = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&schema_cache_2),
        "2".to_string(),
    ));

    let id_allocator_1 = Arc::new(IdAllocator::new(db.clone(), 1, 100));
    let id_allocator_2 = Arc::new(IdAllocator::new(db.clone(), 2, 100));

    let node_store_1 = Arc::new(NodeStore::new(
        db.clone(),
        Arc::clone(&schema_registry_1),
        Arc::clone(&id_allocator_1),
        "1".to_string(),
    ));
    let node_store_2 = Arc::new(NodeStore::new(
        db.clone(),
        Arc::clone(&schema_registry_2),
        Arc::clone(&id_allocator_2),
        "2".to_string(),
    ));

    let edge_store_2 = EdgeStore::new(
        db.clone(),
        Arc::clone(&schema_registry_2),
        Arc::clone(&id_allocator_2),
        Arc::clone(&node_store_2),
        "2".to_string(),
    );

    schema_registry_1
        .register_schema(
            &database,
            &namespace,
            EntitySchema::new("Person")
                .with_property("name", PropertyDef::new(PropertyType::String).required())
                .with_property("age", PropertyDef::new(PropertyType::Int))
                .with_edge("knows", EdgeDef::new(EdgeTarget::specific("Person"))),
        )
        .await
        .expect("schema registration should succeed");

    let alice = node_store_1
        .create_node(
            &database,
            &namespace,
            "Person",
            HashMap::from([("name".to_string(), Value::String("Alice".to_string()))]),
        )
        .await
        .expect("alice create should succeed");
    let bob = node_store_1
        .create_node(
            &database,
            &namespace,
            "Person",
            HashMap::from([("name".to_string(), Value::String("Bob".to_string()))]),
        )
        .await
        .expect("bob create should succeed");

    let update_err = node_store_2
        .update_node(
            &database,
            &namespace,
            "Person",
            &alice.id,
            HashMap::from([("age".to_string(), Value::Int(42))]),
        )
        .await
        .expect_err("cross-site update should be denied");
    assert!(matches!(
        update_err,
        pelago_core::PelagoError::VersionConflict { .. }
    ));

    let edge_err = edge_store_2
        .create_edge(
            &database,
            &namespace,
            NodeRef::new(&database, &namespace, "Person", &alice.id),
            NodeRef::new(&database, &namespace, "Person", &bob.id),
            "knows",
            HashMap::new(),
        )
        .await
        .expect_err("cross-site edge create should be denied");
    assert!(matches!(
        edge_err,
        pelago_core::PelagoError::VersionConflict { .. }
    ));

    node_store_1
        .transfer_ownership(&database, &namespace, "Person", &alice.id, 2)
        .await
        .expect("ownership transfer should succeed");

    node_store_2
        .update_node(
            &database,
            &namespace,
            "Person",
            &alice.id,
            HashMap::from([("age".to_string(), Value::Int(43))]),
        )
        .await
        .expect("new owner should be able to update");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_replica_conflict_resolution_owner_wins_and_lww() {
    let db = PelagoDb::connect(&cluster_file())
        .await
        .expect("Failed to connect to FDB");

    let suffix = unique_suffix();
    let database = format!("replica_conflict_{}", suffix);
    let namespace = "default".to_string();

    let schema_cache_1 = Arc::new(SchemaCache::new());
    let schema_cache_2 = Arc::new(SchemaCache::new());
    let schema_registry_1 = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&schema_cache_1),
        "1".to_string(),
    ));
    let schema_registry_2 = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&schema_cache_2),
        "2".to_string(),
    ));

    let id_allocator_1 = Arc::new(IdAllocator::new(db.clone(), 1, 100));
    let id_allocator_2 = Arc::new(IdAllocator::new(db.clone(), 2, 100));

    let node_store_1 = Arc::new(NodeStore::new(
        db.clone(),
        Arc::clone(&schema_registry_1),
        Arc::clone(&id_allocator_1),
        "1".to_string(),
    ));
    let node_store_2 = Arc::new(NodeStore::new(
        db.clone(),
        Arc::clone(&schema_registry_2),
        Arc::clone(&id_allocator_2),
        "2".to_string(),
    ));

    schema_registry_1
        .register_schema(
            &database,
            &namespace,
            EntitySchema::new("Person")
                .with_property("name", PropertyDef::new(PropertyType::String).required())
                .with_property("age", PropertyDef::new(PropertyType::Int)),
        )
        .await
        .expect("schema registration should succeed");

    let alice = node_store_1
        .create_node(
            &database,
            &namespace,
            "Person",
            HashMap::from([("name".to_string(), Value::String("Alice".to_string()))]),
        )
        .await
        .expect("node create should succeed");
    let initial = node_store_1
        .get_node(&database, &namespace, "Person", &alice.id)
        .await
        .expect("get should succeed")
        .expect("node should exist");

    let stale_applied = node_store_2
        .apply_replica_node_update(
            &database,
            &namespace,
            "Person",
            &alice.id,
            HashMap::from([("age".to_string(), Value::Int(31))]),
            initial.properties.clone(),
            initial.updated_at.saturating_sub(1),
            "2",
        )
        .await
        .expect("stale update apply should not error");
    assert!(!stale_applied, "stale non-owner update must be rejected");

    let lww_applied = node_store_2
        .apply_replica_node_update(
            &database,
            &namespace,
            "Person",
            &alice.id,
            HashMap::from([("age".to_string(), Value::Int(42))]),
            initial.properties.clone(),
            initial.updated_at + 10,
            "2",
        )
        .await
        .expect("newer update apply should not error");
    assert!(
        lww_applied,
        "newer conflicting event should win via LWW fallback"
    );

    let after_lww = node_store_1
        .get_node(&database, &namespace, "Person", &alice.id)
        .await
        .expect("get should succeed")
        .expect("node should exist");
    assert_eq!(
        after_lww.locality, 2,
        "LWW winner should become effective owner"
    );
    assert_eq!(after_lww.properties.get("age"), Some(&Value::Int(42)));

    let stale_delete = node_store_1
        .apply_replica_node_delete(
            &database,
            &namespace,
            "Person",
            &alice.id,
            after_lww.updated_at.saturating_sub(1),
            "1",
        )
        .await
        .expect("stale delete apply should not error");
    assert!(
        !stale_delete,
        "stale delete from non-owner must be rejected"
    );

    let lww_delete = node_store_1
        .apply_replica_node_delete(
            &database,
            &namespace,
            "Person",
            &alice.id,
            after_lww.updated_at + 10,
            "1",
        )
        .await
        .expect("newer delete apply should not error");
    assert!(lww_delete, "newer delete should win via LWW fallback");

    let deleted = node_store_1
        .get_node(&database, &namespace, "Person", &alice.id)
        .await
        .expect("get after delete should succeed");
    assert!(deleted.is_none());
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_audit_retention_cleanup_deletes_old_records() {
    let db = PelagoDb::connect(&cluster_file())
        .await
        .expect("Failed to connect to FDB");

    let suffix = unique_suffix();
    let principal = format!("audit_retention_{}", suffix);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    let old_ts = now - (3 * 24 * 60 * 60 * 1_000_000);

    let old = append_audit_record(
        &db,
        AuditRecord {
            event_id: String::new(),
            timestamp: old_ts,
            principal_id: principal.clone(),
            action: "authz.denied".to_string(),
            resource: "db/ns/Person".to_string(),
            allowed: false,
            reason: "old".to_string(),
            metadata: HashMap::new(),
        },
    )
    .await
    .expect("old audit append should succeed");
    let recent = append_audit_record(
        &db,
        AuditRecord {
            event_id: String::new(),
            timestamp: now,
            principal_id: principal.clone(),
            action: "authz.allowed".to_string(),
            resource: "db/ns/Person".to_string(),
            allowed: true,
            reason: "new".to_string(),
            metadata: HashMap::new(),
        },
    )
    .await
    .expect("recent audit append should succeed");

    let deleted = cleanup_audit_records(&db, 24 * 60 * 60, 1000)
        .await
        .expect("audit cleanup should succeed");
    assert!(
        deleted >= 1,
        "expected at least one old audit record to be deleted"
    );

    let rows = query_audit_records(&db, Some(&principal), None, None, None, 50)
        .await
        .expect("audit query should succeed");
    assert!(!rows.iter().any(|r| r.event_id == old.event_id));
    assert!(rows.iter().any(|r| r.event_id == recent.event_id));
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_two_site_pull_replication_roundtrip() {
    let db = PelagoDb::connect(&cluster_file())
        .await
        .expect("Failed to connect to FDB");

    let suffix = unique_suffix();
    let source_db = format!("site_a_{}", suffix);
    let target_db = format!("site_b_{}", suffix);
    let namespace = "default".to_string();

    let source_cache = Arc::new(SchemaCache::new());
    let target_cache = Arc::new(SchemaCache::new());
    let source_registry = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&source_cache),
        "1".to_string(),
    ));
    let target_registry = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&target_cache),
        "2".to_string(),
    ));

    let source_ids = Arc::new(IdAllocator::new(db.clone(), 1, 100));
    let target_ids = Arc::new(IdAllocator::new(db.clone(), 2, 100));

    let source_nodes = Arc::new(NodeStore::new(
        db.clone(),
        Arc::clone(&source_registry),
        Arc::clone(&source_ids),
        "1".to_string(),
    ));
    let target_nodes = Arc::new(NodeStore::new(
        db.clone(),
        Arc::clone(&target_registry),
        Arc::clone(&target_ids),
        "2".to_string(),
    ));
    let source_edges = EdgeStore::new(
        db.clone(),
        Arc::clone(&source_registry),
        Arc::clone(&source_ids),
        Arc::clone(&source_nodes),
        "1".to_string(),
    );
    let target_edges = EdgeStore::new(
        db.clone(),
        Arc::clone(&target_registry),
        Arc::clone(&target_ids),
        Arc::clone(&target_nodes),
        "2".to_string(),
    );

    let schema = EntitySchema::new("Person")
        .with_property("name", PropertyDef::new(PropertyType::String).required())
        .with_property("age", PropertyDef::new(PropertyType::Int))
        .with_edge("knows", EdgeDef::new(EdgeTarget::specific("Person")));
    source_registry
        .register_schema(&source_db, &namespace, schema.clone())
        .await
        .expect("source schema registration should succeed");
    target_registry
        .register_schema(&target_db, &namespace, schema)
        .await
        .expect("target schema registration should succeed");

    let alice = source_nodes
        .create_node(
            &source_db,
            &namespace,
            "Person",
            HashMap::from([("name".to_string(), Value::String("Alice".to_string()))]),
        )
        .await
        .expect("source alice create should succeed");
    let bob = source_nodes
        .create_node(
            &source_db,
            &namespace,
            "Person",
            HashMap::from([("name".to_string(), Value::String("Bob".to_string()))]),
        )
        .await
        .expect("source bob create should succeed");
    source_edges
        .create_edge(
            &source_db,
            &namespace,
            NodeRef::new(&source_db, &namespace, "Person", &alice.id),
            NodeRef::new(&source_db, &namespace, "Person", &bob.id),
            "knows",
            HashMap::new(),
        )
        .await
        .expect("source edge create should succeed");

    let source_entries = read_cdc_entries(&db, &source_db, &namespace, None, 10_000)
        .await
        .expect("source cdc read should succeed");
    assert!(
        !source_entries.is_empty(),
        "source site must emit CDC events"
    );

    let mut last_applied = Versionstamp::zero();
    for (vs, entry) in source_entries {
        assert_eq!(entry.site, "1");
        for op in entry.operations {
            match op {
                CdcOperation::NodeCreate {
                    entity_type,
                    node_id,
                    properties,
                    home_site,
                } => {
                    target_nodes
                        .apply_replica_node_create(
                            &target_db,
                            &namespace,
                            &entity_type,
                            &node_id,
                            properties,
                            &home_site,
                            entry.timestamp,
                            &entry.site,
                        )
                        .await
                        .expect("replica node create should succeed");
                }
                CdcOperation::NodeUpdate {
                    entity_type,
                    node_id,
                    changed_properties,
                    old_properties,
                } => {
                    target_nodes
                        .apply_replica_node_update(
                            &target_db,
                            &namespace,
                            &entity_type,
                            &node_id,
                            changed_properties,
                            old_properties,
                            entry.timestamp,
                            &entry.site,
                        )
                        .await
                        .expect("replica node update should succeed");
                }
                CdcOperation::NodeDelete {
                    entity_type,
                    node_id,
                } => {
                    target_nodes
                        .apply_replica_node_delete(
                            &target_db,
                            &namespace,
                            &entity_type,
                            &node_id,
                            entry.timestamp,
                            &entry.site,
                        )
                        .await
                        .expect("replica node delete should succeed");
                }
                CdcOperation::EdgeCreate {
                    source_type,
                    source_id,
                    target_type,
                    target_id,
                    edge_type,
                    edge_id,
                    properties,
                } => {
                    target_edges
                        .apply_replica_edge_create(
                            &target_db,
                            &namespace,
                            NodeRef::new(&target_db, &namespace, &source_type, &source_id),
                            NodeRef::new(&target_db, &namespace, &target_type, &target_id),
                            &edge_type,
                            &edge_id,
                            properties,
                            entry.timestamp,
                            &entry.site,
                        )
                        .await
                        .expect("replica edge create should succeed");
                }
                CdcOperation::EdgeDelete {
                    source_type,
                    source_id,
                    target_type,
                    target_id,
                    edge_type,
                } => {
                    target_edges
                        .apply_replica_edge_delete(
                            &target_db,
                            &namespace,
                            NodeRef::new(&target_db, &namespace, &source_type, &source_id),
                            NodeRef::new(&target_db, &namespace, &target_type, &target_id),
                            &edge_type,
                            entry.timestamp,
                            &entry.site,
                        )
                        .await
                        .expect("replica edge delete should succeed");
                }
                CdcOperation::OwnershipTransfer {
                    entity_type,
                    node_id,
                    previous_site_id,
                    current_site_id,
                } => {
                    target_nodes
                        .apply_replica_ownership_transfer(
                            &target_db,
                            &namespace,
                            &entity_type,
                            &node_id,
                            &previous_site_id,
                            &current_site_id,
                            entry.timestamp,
                            &entry.site,
                        )
                        .await
                        .expect("replica ownership transfer should succeed");
                }
                CdcOperation::SchemaRegister { .. } => {}
            }
        }
        last_applied = vs;
    }

    let replicated_alice = target_nodes
        .get_node(&target_db, &namespace, "Person", &alice.id)
        .await
        .expect("target get alice should succeed");
    assert!(replicated_alice.is_some());
    let replicated_edges = target_edges
        .list_edges(
            &target_db,
            &namespace,
            "Person",
            &alice.id,
            Some("knows"),
            10,
        )
        .await
        .expect("target edge list should succeed");
    assert_eq!(replicated_edges.len(), 1);

    source_nodes
        .update_node(
            &source_db,
            &namespace,
            "Person",
            &alice.id,
            HashMap::from([("age".to_string(), Value::Int(33))]),
        )
        .await
        .expect("source update should succeed");

    let delta = read_cdc_entries(&db, &source_db, &namespace, Some(&last_applied), 10_000)
        .await
        .expect("delta cdc read should succeed");
    assert!(
        !delta.is_empty(),
        "second pull should return only new source-site events"
    );

    for (_vs, entry) in delta {
        for op in entry.operations {
            if let CdcOperation::NodeUpdate {
                entity_type,
                node_id,
                changed_properties,
                old_properties,
            } = op
            {
                target_nodes
                    .apply_replica_node_update(
                        &target_db,
                        &namespace,
                        &entity_type,
                        &node_id,
                        changed_properties,
                        old_properties,
                        entry.timestamp,
                        &entry.site,
                    )
                    .await
                    .expect("delta replica node update should succeed");
            }
        }
    }

    let alice_after_delta = target_nodes
        .get_node(&target_db, &namespace, "Person", &alice.id)
        .await
        .expect("target get alice after delta should succeed")
        .expect("alice should still exist on target");
    assert_eq!(
        alice_after_delta.properties.get("age"),
        Some(&Value::Int(33))
    );
}
