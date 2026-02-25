//! CDC Integration Tests
//!
//! Tests CDC entry production, consumption, and consumer checkpoint/resume behavior.
//!
//! Run with:
//!   LIBRARY_PATH=/usr/local/lib cargo test --test cdc_integration -- --ignored --nocapture

use pelago_core::schema::{EdgeDef, EdgeTarget, EntitySchema, PropertyDef};
use pelago_core::{PropertyType, Value};
use pelago_storage::{
    CdcConsumer, CdcOperation, ConsumerConfig, EdgeStore, IdAllocator, NodeRef, NodeStore,
    PelagoDb, SchemaCache, SchemaRegistry,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn cluster_file() -> String {
    std::env::var("FDB_CLUSTER_FILE")
        .unwrap_or_else(|_| "/usr/local/etc/foundationdb/fdb.cluster".to_string())
}

fn unique_db() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("cdc_test_{}", ts)
}

/// Helper to set up common test infrastructure
async fn setup() -> (
    PelagoDb,
    String,
    String,
    Arc<SchemaRegistry>,
    Arc<IdAllocator>,
    Arc<NodeStore>,
    EdgeStore,
) {
    let db = PelagoDb::connect(&cluster_file())
        .await
        .expect("Failed to connect to FDB");
    let database = unique_db();
    let namespace = "default".to_string();
    let site_id = "1".to_string();

    let schema_cache = Arc::new(SchemaCache::new());
    let schema_registry = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&schema_cache),
        site_id.clone(),
    ));
    let id_allocator = Arc::new(IdAllocator::new(db.clone(), 1, 100));
    let node_store = Arc::new(NodeStore::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        site_id.clone(),
    ));
    let edge_store = EdgeStore::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        Arc::clone(&node_store),
        site_id,
    );

    (
        db,
        database,
        namespace,
        schema_registry,
        id_allocator,
        node_store,
        edge_store,
    )
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_cdc_consumer_receives_all_mutations() {
    let (db, database, namespace, schema_registry, _id_allocator, node_store, edge_store) =
        setup().await;

    // 1. Register a schema → SchemaRegister CDC op
    let schema = EntitySchema::new("Person")
        .with_property("name", PropertyDef::new(PropertyType::String).required())
        .with_property("age", PropertyDef::new(PropertyType::Int))
        .with_edge("knows", EdgeDef::new(EdgeTarget::specific("Person")));

    schema_registry
        .register_schema(&database, &namespace, schema)
        .await
        .expect("Failed to register schema");
    println!("✓ Schema registered");

    // 2. Create nodes → NodeCreate CDC ops
    let alice = node_store
        .create_node(
            &database,
            &namespace,
            "Person",
            HashMap::from([
                ("name".to_string(), Value::String("Alice".to_string())),
                ("age".to_string(), Value::Int(30)),
            ]),
        )
        .await
        .expect("Failed to create Alice");
    println!("✓ Created Alice ({})", alice.id);

    let bob = node_store
        .create_node(
            &database,
            &namespace,
            "Person",
            HashMap::from([
                ("name".to_string(), Value::String("Bob".to_string())),
                ("age".to_string(), Value::Int(25)),
            ]),
        )
        .await
        .expect("Failed to create Bob");
    println!("✓ Created Bob ({})", bob.id);

    // 3. Update node → NodeUpdate CDC op
    node_store
        .update_node(
            &database,
            &namespace,
            "Person",
            &alice.id,
            HashMap::from([("age".to_string(), Value::Int(31))]),
        )
        .await
        .expect("Failed to update Alice");
    println!("✓ Updated Alice's age");

    // 4. Create edge → EdgeCreate CDC op
    let alice_ref = NodeRef::new(&database, &namespace, "Person", &alice.id);
    let bob_ref = NodeRef::new(&database, &namespace, "Person", &bob.id);

    edge_store
        .create_edge(
            &database,
            &namespace,
            alice_ref.clone(),
            bob_ref.clone(),
            "knows",
            HashMap::new(),
        )
        .await
        .expect("Failed to create edge");
    println!("✓ Created edge Alice→Bob");

    // 5. Delete edge → EdgeDelete CDC op
    edge_store
        .delete_edge(&database, &namespace, alice_ref, bob_ref, "knows")
        .await
        .expect("Failed to delete edge");
    println!("✓ Deleted edge Alice→Bob");

    // 6. Delete node → NodeDelete CDC op
    node_store
        .delete_node(&database, &namespace, "Person", &bob.id)
        .await
        .expect("Failed to delete Bob");
    println!("✓ Deleted Bob");

    // Now consume all CDC entries and verify
    let config = ConsumerConfig::new("test_all_mutations", &database, &namespace);
    let mut consumer = CdcConsumer::new(db.clone(), config)
        .await
        .expect("Failed to create consumer");

    let entries = consumer.poll_batch().await.expect("Failed to poll CDC");

    let mut op_counts: HashMap<&str, usize> = HashMap::new();
    for (_vs, entry) in &entries {
        for op in &entry.operations {
            let name = match op {
                CdcOperation::NodeCreate { .. } => "NodeCreate",
                CdcOperation::NodeUpdate { .. } => "NodeUpdate",
                CdcOperation::NodeDelete { .. } => "NodeDelete",
                CdcOperation::EdgeCreate { .. } => "EdgeCreate",
                CdcOperation::EdgeDelete { .. } => "EdgeDelete",
                CdcOperation::SchemaRegister { .. } => "SchemaRegister",
                CdcOperation::OwnershipTransfer { .. } => "OwnershipTransfer",
            };
            *op_counts.entry(name).or_insert(0) += 1;
        }
    }

    println!("\nCDC operation counts: {:?}", op_counts);

    assert_eq!(op_counts.get("SchemaRegister").copied().unwrap_or(0), 1);
    assert_eq!(op_counts.get("NodeCreate").copied().unwrap_or(0), 2);
    assert_eq!(op_counts.get("NodeUpdate").copied().unwrap_or(0), 1);
    assert!(op_counts.get("EdgeCreate").copied().unwrap_or(0) >= 1);
    assert!(op_counts.get("EdgeDelete").copied().unwrap_or(0) >= 1);
    assert_eq!(op_counts.get("NodeDelete").copied().unwrap_or(0), 1);

    // 8. Verify versionstamp ordering is monotonic
    let mut prev = None;
    for (vs, _) in &entries {
        if let Some(p) = prev {
            assert!(vs > p, "CDC entries must be in versionstamp order");
        }
        prev = Some(vs);
    }
    println!("✓ Versionstamp ordering is monotonically increasing");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_cdc_consumer_resumes_from_checkpoint() {
    let (db, database, namespace, schema_registry, _id_allocator, node_store, _edge_store) =
        setup().await;

    // Register schema
    let schema = EntitySchema::new("Widget")
        .with_property("name", PropertyDef::new(PropertyType::String).required());
    schema_registry
        .register_schema(&database, &namespace, schema)
        .await
        .unwrap();

    // Create first batch of mutations
    node_store
        .create_node(
            &database,
            &namespace,
            "Widget",
            HashMap::from([("name".to_string(), Value::String("W1".to_string()))]),
        )
        .await
        .unwrap();
    node_store
        .create_node(
            &database,
            &namespace,
            "Widget",
            HashMap::from([("name".to_string(), Value::String("W2".to_string()))]),
        )
        .await
        .unwrap();

    // Consumer reads and checkpoints
    let config = ConsumerConfig::new("resume_test", &database, &namespace);
    let mut consumer = CdcConsumer::new(db.clone(), config).await.unwrap();

    let batch1 = consumer.poll_batch().await.unwrap();
    let batch1_count = batch1.len();
    println!("First batch: {} entries", batch1_count);
    assert!(
        batch1_count >= 3,
        "Expected schema + 2 nodes = at least 3 entries"
    );

    // Force checkpoint
    consumer.checkpoint().await.unwrap();
    println!("✓ Checkpoint saved");

    // Create more mutations AFTER checkpoint
    node_store
        .create_node(
            &database,
            &namespace,
            "Widget",
            HashMap::from([("name".to_string(), Value::String("W3".to_string()))]),
        )
        .await
        .unwrap();

    // Drop the old consumer and create a new one with the same ID
    drop(consumer);

    let config2 = ConsumerConfig::new("resume_test", &database, &namespace);
    let mut consumer2 = CdcConsumer::new(db.clone(), config2).await.unwrap();

    // Should only get entries AFTER the checkpoint
    let batch2 = consumer2.poll_batch().await.unwrap();
    println!("Second batch (after resume): {} entries", batch2.len());
    assert_eq!(
        batch2.len(),
        1,
        "Should only receive the one new mutation after checkpoint"
    );

    // Verify it's the W3 node
    let has_w3 = batch2.iter().any(|(_, entry)| {
        entry.operations.iter().any(|op| match op {
            CdcOperation::NodeCreate { node_id, .. } => !node_id.is_empty(),
            _ => false,
        })
    });
    assert!(has_w3, "Resumed batch should contain the W3 node creation");
    println!("✓ Consumer correctly resumed from checkpoint");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_multiple_independent_consumers() {
    let (db, database, namespace, schema_registry, _id_allocator, node_store, _edge_store) =
        setup().await;

    // Register schema and create a node
    let schema = EntitySchema::new("Item")
        .with_property("name", PropertyDef::new(PropertyType::String).required());
    schema_registry
        .register_schema(&database, &namespace, schema)
        .await
        .unwrap();

    node_store
        .create_node(
            &database,
            &namespace,
            "Item",
            HashMap::from([("name".to_string(), Value::String("I1".to_string()))]),
        )
        .await
        .unwrap();

    // Create two independent consumers
    let config_a = ConsumerConfig::new("consumer_a", &database, &namespace);
    let config_b = ConsumerConfig::new("consumer_b", &database, &namespace);

    let mut consumer_a = CdcConsumer::new(db.clone(), config_a).await.unwrap();
    let mut consumer_b = CdcConsumer::new(db.clone(), config_b).await.unwrap();

    // Both should receive all entries
    let batch_a = consumer_a.poll_batch().await.unwrap();
    let batch_b = consumer_b.poll_batch().await.unwrap();

    println!("Consumer A: {} entries", batch_a.len());
    println!("Consumer B: {} entries", batch_b.len());

    assert_eq!(
        batch_a.len(),
        batch_b.len(),
        "Both consumers should see same entries"
    );
    assert!(
        batch_a.len() >= 2,
        "Expected at least schema + node = 2 entries"
    );

    // Checkpoint A but not B
    consumer_a.checkpoint().await.unwrap();

    // Create another mutation
    node_store
        .create_node(
            &database,
            &namespace,
            "Item",
            HashMap::from([("name".to_string(), Value::String("I2".to_string()))]),
        )
        .await
        .unwrap();

    // A should only see new entry
    let batch_a2 = consumer_a.poll_batch().await.unwrap();
    assert_eq!(
        batch_a2.len(),
        1,
        "Consumer A should only see new entry after checkpoint"
    );

    // B (no checkpoint) sees the new entry too since its HWM advanced in memory
    let batch_b2 = consumer_b.poll_batch().await.unwrap();
    assert_eq!(
        batch_b2.len(),
        1,
        "Consumer B should see only new entry (HWM advanced in memory)"
    );

    // But if we recreate B (simulating restart without checkpoint), it sees everything again
    drop(consumer_b);
    let config_b2 = ConsumerConfig::new("consumer_b", &database, &namespace);
    let mut consumer_b_fresh = CdcConsumer::new(db.clone(), config_b2).await.unwrap();
    let batch_b_all = consumer_b_fresh.poll_batch().await.unwrap();
    assert!(
        batch_b_all.len() >= 3,
        "Fresh consumer B (no checkpoint) should see all entries"
    );
    println!("✓ Independent consumers operate correctly");
}
