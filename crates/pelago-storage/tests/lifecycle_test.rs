//! Full Lifecycle Integration Test
//!
//! This test exercises the complete PelagoDB workflow:
//! 1. Schema registration
//! 2. Node creation with validation
//! 3. Edge creation (including bidirectional)
//! 4. Node updates and constraint validation
//! 5. Edge and node deletion
//! 6. CDC verification (Phase 2)
//!
//! Run with:
//!   LIBRARY_PATH=/usr/local/lib cargo test --test lifecycle_test -- --ignored --nocapture
//!
//! The --nocapture flag shows all the println! output so you can see what's happening.

use pelago_core::schema::{EdgeDef, EdgeTarget, EntitySchema, IndexType, PropertyDef};
use pelago_core::{PropertyType, Value};
use pelago_storage::{
    CdcConsumer, CdcOperation, ConsumerConfig, EdgeStore, IdAllocator, NodeRef, NodeStore,
    PelagoDb, SchemaCache, SchemaRegistry, Versionstamp,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const SEPARATOR: &str = "════════════════════════════════════════════════════════════";
const THIN_SEP: &str = "────────────────────────────────────────────────────────────";

/// Test database and namespace - unique per run to avoid conflicts
fn test_context() -> (String, String) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    (format!("test_db_{}", ts), "default".to_string())
}

fn cluster_file() -> String {
    std::env::var("FDB_CLUSTER_FILE")
        .unwrap_or_else(|_| "/usr/local/etc/foundationdb/fdb.cluster".to_string())
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_full_lifecycle() {
    println!("\n{}", SEPARATOR);
    println!("  PelagoDB Full Lifecycle Test");
    println!("{}\n", SEPARATOR);

    // =========================================================================
    // SETUP: Connect to FDB and create stores
    // =========================================================================
    println!("📦 Connecting to FoundationDB...");
    let db = PelagoDb::connect(&cluster_file())
        .await
        .expect("Failed to connect to FDB");
    println!("   ✓ Connected to cluster: {}\n", cluster_file());

    let (database, namespace) = test_context();
    println!(
        "📁 Test context: database='{}', namespace='{}'\n",
        database, namespace
    );

    // Create shared components
    let site_id = "1".to_string();
    let schema_cache = Arc::new(SchemaCache::new());
    let schema_registry = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&schema_cache),
        site_id.clone(),
    ));
    let id_allocator = Arc::new(IdAllocator::new(db.clone(), 1, 100)); // site_id=1, batch=100
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

    // =========================================================================
    // PHASE 1: Register Schemas
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 1: Schema Registration");
    println!("{}\n", THIN_SEP);

    // User schema with indexed email
    let user_schema = EntitySchema::new("User")
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
        .with_property(
            "active",
            PropertyDef::new(PropertyType::Bool).with_default(Value::Bool(true)),
        )
        .with_edge("follows", EdgeDef::new(EdgeTarget::specific("User")))
        .with_edge(
            "friends_with",
            EdgeDef::new(EdgeTarget::specific("User")).bidirectional(),
        );

    let user_version = schema_registry
        .register_schema(&database, &namespace, user_schema)
        .await
        .expect("Failed to register User schema");
    println!("   ✓ Registered 'User' schema (version {})", user_version);
    println!("     - email: String, required, unique index");
    println!("     - name: String, required");
    println!("     - age: Int, range index");
    println!("     - active: Bool, default=true");
    println!("     - follows -> User (unidirectional)");
    println!("     - friends_with <-> User (bidirectional)\n");

    // Post schema
    let post_schema = EntitySchema::new("Post")
        .with_property("title", PropertyDef::new(PropertyType::String).required())
        .with_property("content", PropertyDef::new(PropertyType::String))
        .with_property(
            "published_at",
            PropertyDef::new(PropertyType::Timestamp).with_index(IndexType::Range),
        )
        .with_edge("authored_by", EdgeDef::new(EdgeTarget::specific("User")));

    let post_version = schema_registry
        .register_schema(&database, &namespace, post_schema)
        .await
        .expect("Failed to register Post schema");
    println!("   ✓ Registered 'Post' schema (version {})", post_version);
    println!("     - title: String, required");
    println!("     - content: String");
    println!("     - published_at: Timestamp, range index");
    println!("     - authored_by -> User\n");

    // Verify schemas can be retrieved
    let schemas = schema_registry
        .list_schemas(&database, &namespace)
        .await
        .expect("Failed to list schemas");
    println!("   📋 Registered schemas: {:?}\n", schemas);
    assert!(schemas.contains(&"User".to_string()));
    assert!(schemas.contains(&"Post".to_string()));

    // =========================================================================
    // PHASE 2: Create Nodes
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 2: Node Creation");
    println!("{}\n", THIN_SEP);

    // Create users
    let alice = node_store
        .create_node(
            &database,
            &namespace,
            "User",
            HashMap::from([
                (
                    "email".to_string(),
                    Value::String("alice@example.com".to_string()),
                ),
                ("name".to_string(), Value::String("Alice Smith".to_string())),
                ("age".to_string(), Value::Int(30)),
            ]),
        )
        .await
        .expect("Failed to create Alice");
    println!("   ✓ Created User 'Alice' (id: {})", alice.id);
    println!("     email: alice@example.com");
    println!("     age: 30");
    println!(
        "     active: {:?} (default applied)\n",
        alice.properties.get("active").unwrap()
    );

    let bob = node_store
        .create_node(
            &database,
            &namespace,
            "User",
            HashMap::from([
                (
                    "email".to_string(),
                    Value::String("bob@example.com".to_string()),
                ),
                ("name".to_string(), Value::String("Bob Jones".to_string())),
                ("age".to_string(), Value::Int(25)),
            ]),
        )
        .await
        .expect("Failed to create Bob");
    println!("   ✓ Created User 'Bob' (id: {})", bob.id);

    let carol = node_store
        .create_node(
            &database,
            &namespace,
            "User",
            HashMap::from([
                (
                    "email".to_string(),
                    Value::String("carol@example.com".to_string()),
                ),
                ("name".to_string(), Value::String("Carol White".to_string())),
                ("age".to_string(), Value::Int(35)),
                ("active".to_string(), Value::Bool(false)),
            ]),
        )
        .await
        .expect("Failed to create Carol");
    println!("   ✓ Created User 'Carol' (id: {})", carol.id);
    println!("     active: false (explicitly set)\n");

    // Create a post
    let now_micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;

    let post1 = node_store
        .create_node(
            &database,
            &namespace,
            "Post",
            HashMap::from([
                (
                    "title".to_string(),
                    Value::String("Hello World".to_string()),
                ),
                (
                    "content".to_string(),
                    Value::String("This is my first post!".to_string()),
                ),
                ("published_at".to_string(), Value::Timestamp(now_micros)),
            ]),
        )
        .await
        .expect("Failed to create Post");
    println!("   ✓ Created Post 'Hello World' (id: {})\n", post1.id);

    // =========================================================================
    // PHASE 3: Verify Node Retrieval
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 3: Node Retrieval");
    println!("{}\n", THIN_SEP);

    let retrieved = node_store
        .get_node(&database, &namespace, "User", &alice.id)
        .await
        .expect("Failed to get Alice")
        .expect("Alice should exist");
    println!("   ✓ Retrieved Alice by ID");
    println!("     id: {}", retrieved.id);
    println!("     email: {:?}", retrieved.properties.get("email"));
    println!("     created_at: {}\n", retrieved.created_at);

    // =========================================================================
    // PHASE 4: Create Edges
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 4: Edge Creation");
    println!("{}\n", THIN_SEP);

    // Alice follows Bob (unidirectional)
    let alice_ref = NodeRef::new(&database, &namespace, "User", &alice.id);
    let bob_ref = NodeRef::new(&database, &namespace, "User", &bob.id);
    let carol_ref = NodeRef::new(&database, &namespace, "User", &carol.id);
    let post_ref = NodeRef::new(&database, &namespace, "Post", &post1.id);

    let follows_edge = edge_store
        .create_edge(
            &database,
            &namespace,
            alice_ref.clone(),
            bob_ref.clone(),
            "follows",
            HashMap::new(),
        )
        .await
        .expect("Failed to create follows edge");
    println!(
        "   ✓ Alice --[follows]--> Bob (edge_id: {})",
        follows_edge.edge_id
    );

    // Alice follows Carol
    let follows_edge2 = edge_store
        .create_edge(
            &database,
            &namespace,
            alice_ref.clone(),
            carol_ref.clone(),
            "follows",
            HashMap::new(),
        )
        .await
        .expect("Failed to create follows edge 2");
    println!(
        "   ✓ Alice --[follows]--> Carol (edge_id: {})",
        follows_edge2.edge_id
    );

    // Bob and Carol are friends (bidirectional - creates edges in both directions)
    let friends_edge = edge_store
        .create_edge(
            &database,
            &namespace,
            bob_ref.clone(),
            carol_ref.clone(),
            "friends_with",
            HashMap::from([("since".to_string(), Value::String("2024-01-15".to_string()))]),
        )
        .await
        .expect("Failed to create friends edge");
    println!(
        "   ✓ Bob <--[friends_with]--> Carol (edge_id: {})",
        friends_edge.edge_id
    );
    println!("     (bidirectional: reverse edge also created)\n");

    // Post authored by Alice
    let authored_edge = edge_store
        .create_edge(
            &database,
            &namespace,
            post_ref.clone(),
            alice_ref.clone(),
            "authored_by",
            HashMap::new(),
        )
        .await
        .expect("Failed to create authored_by edge");
    println!(
        "   ✓ Post --[authored_by]--> Alice (edge_id: {})\n",
        authored_edge.edge_id
    );

    // =========================================================================
    // PHASE 5: List Edges
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 5: Edge Listing");
    println!("{}\n", THIN_SEP);

    // List Alice's outgoing "follows" edges
    let alice_follows = edge_store
        .list_edges(
            &database,
            &namespace,
            "User",
            &alice.id,
            Some("follows"),
            100,
        )
        .await
        .expect("Failed to list Alice's follows");
    println!("   Alice's 'follows' edges: {} found", alice_follows.len());
    for edge in &alice_follows {
        println!(
            "     -> {} ({})",
            edge.target.node_id, edge.target.entity_type
        );
    }
    println!();

    // List all of Bob's edges (any label)
    let bob_edges = edge_store
        .list_edges(&database, &namespace, "User", &bob.id, None, 100)
        .await
        .expect("Failed to list Bob's edges");
    println!("   Bob's edges (all labels): {} found", bob_edges.len());
    for edge in &bob_edges {
        println!(
            "     --[{}]--> {} ({})",
            edge.label, edge.target.node_id, edge.target.entity_type
        );
    }
    println!();

    // =========================================================================
    // PHASE 6: Update Node
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 6: Node Updates");
    println!("{}\n", THIN_SEP);

    let updated_alice = node_store
        .update_node(
            &database,
            &namespace,
            "User",
            &alice.id,
            HashMap::from([
                ("age".to_string(), Value::Int(31)), // Birthday!
            ]),
        )
        .await
        .expect("Failed to update Alice");
    println!("   ✓ Updated Alice's age: 30 -> 31");
    println!("     updated_at: {}", updated_alice.updated_at);
    println!("     (was created_at: {})\n", updated_alice.created_at);

    // =========================================================================
    // PHASE 7: Test Unique Constraint
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 7: Constraint Validation");
    println!("{}\n", THIN_SEP);

    // Try to create a user with duplicate email (should fail)
    let duplicate_result = node_store
        .create_node(
            &database,
            &namespace,
            "User",
            HashMap::from([
                (
                    "email".to_string(),
                    Value::String("alice@example.com".to_string()),
                ), // Already exists!
                ("name".to_string(), Value::String("Fake Alice".to_string())),
            ]),
        )
        .await;

    match duplicate_result {
        Err(e) => println!("   ✓ Duplicate email correctly rejected: {}\n", e),
        Ok(_) => panic!("Should have rejected duplicate email!"),
    }

    // Try to create a user missing required field (should fail)
    let missing_field_result = node_store
        .create_node(
            &database,
            &namespace,
            "User",
            HashMap::from([
                (
                    "email".to_string(),
                    Value::String("dave@example.com".to_string()),
                ),
                // Missing required 'name' field!
            ]),
        )
        .await;

    match missing_field_result {
        Err(e) => println!("   ✓ Missing required field correctly rejected: {}\n", e),
        Ok(_) => panic!("Should have rejected missing required field!"),
    }

    // =========================================================================
    // PHASE 8: Delete Edge
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 8: Edge Deletion");
    println!("{}\n", THIN_SEP);

    let deleted = edge_store
        .delete_edge(
            &database,
            &namespace,
            alice_ref.clone(),
            carol_ref.clone(),
            "follows",
        )
        .await
        .expect("Failed to delete edge");
    println!("   ✓ Deleted Alice --[follows]--> Carol: {}\n", deleted);

    // Verify it's gone
    let alice_follows_after = edge_store
        .list_edges(
            &database,
            &namespace,
            "User",
            &alice.id,
            Some("follows"),
            100,
        )
        .await
        .expect("Failed to list edges");
    println!(
        "   Alice's 'follows' edges after deletion: {} (was 2)\n",
        alice_follows_after.len()
    );
    assert_eq!(alice_follows_after.len(), 1);

    // =========================================================================
    // PHASE 9: Delete Node
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 9: Node Deletion");
    println!("{}\n", THIN_SEP);

    let deleted = node_store
        .delete_node(&database, &namespace, "User", &carol.id)
        .await
        .expect("Failed to delete Carol");
    println!("   ✓ Deleted Carol: {}", deleted);

    // Verify Carol is gone
    let carol_check = node_store
        .get_node(&database, &namespace, "User", &carol.id)
        .await
        .expect("Failed to check Carol");
    println!(
        "   Carol exists after deletion: {}\n",
        carol_check.is_some()
    );
    assert!(carol_check.is_none());

    // =========================================================================
    // PHASE 10: CDC Verification
    // =========================================================================
    println!("{}", THIN_SEP);
    println!("  PHASE 10: CDC Verification");
    println!("{}\n", THIN_SEP);

    // Create a consumer to read all CDC entries produced during the test
    let consumer_config = ConsumerConfig::new("lifecycle_test", &database, &namespace);
    let mut consumer = CdcConsumer::new(db.clone(), consumer_config)
        .await
        .expect("Failed to create CDC consumer");

    let entries = consumer
        .poll_batch()
        .await
        .expect("Failed to poll CDC entries");

    println!("   CDC entries found: {}", entries.len());
    assert!(!entries.is_empty(), "Expected CDC entries from mutations");

    // Count operations by type
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

    println!("   CDC operation counts:");
    for (op, count) in &op_counts {
        println!("     {}: {}", op, count);
    }
    println!();

    // Expected operations from the lifecycle test:
    // - 2 SchemaRegister (User, Post)
    // - 4 NodeCreate (Alice, Bob, Carol, Post1)
    // - 1 NodeUpdate (Alice age 30→31)
    // - 4+ EdgeCreate (follows x2, friends_with (bidirectional creates 2), authored_by)
    // - 1 EdgeDelete (Alice --follows--> Carol)
    // - 1 NodeDelete (Carol)
    assert!(
        op_counts.get("SchemaRegister").unwrap_or(&0) >= &2,
        "Expected >= 2 SchemaRegister CDC ops"
    );
    assert!(
        op_counts.get("NodeCreate").unwrap_or(&0) >= &4,
        "Expected >= 4 NodeCreate CDC ops"
    );
    assert!(
        op_counts.get("NodeUpdate").unwrap_or(&0) >= &1,
        "Expected >= 1 NodeUpdate CDC ops"
    );
    assert!(
        op_counts.get("EdgeCreate").unwrap_or(&0) >= &4,
        "Expected >= 4 EdgeCreate CDC ops"
    );
    assert!(
        op_counts.get("EdgeDelete").unwrap_or(&0) >= &1,
        "Expected >= 1 EdgeDelete CDC ops"
    );
    assert!(
        op_counts.get("NodeDelete").unwrap_or(&0) >= &1,
        "Expected >= 1 NodeDelete CDC ops"
    );
    println!("   ✓ All expected CDC operation types present");

    // Verify versionstamp ordering is monotonic
    let mut prev_vs: Option<&Versionstamp> = None;
    for (vs, _) in &entries {
        if let Some(prev) = prev_vs {
            assert!(vs > prev, "CDC entries should be in versionstamp order");
        }
        prev_vs = Some(vs);
    }
    println!("   ✓ All CDC entries in monotonically increasing versionstamp order");

    consumer
        .ack_batch(&entries)
        .await
        .expect("Failed to ack CDC entries");

    // Checkpoint and verify resume works
    consumer
        .checkpoint()
        .await
        .expect("Failed to save CDC checkpoint");
    let empty_batch = consumer
        .poll_batch()
        .await
        .expect("Failed to poll after checkpoint");
    assert!(
        empty_batch.is_empty(),
        "Expected no new entries after checkpoint"
    );
    println!("   ✓ Consumer checkpoint + resume works\n");

    // =========================================================================
    // SUMMARY
    // =========================================================================
    println!("{}", SEPARATOR);
    println!("  TEST COMPLETE - All assertions passed!");
    println!("{}", SEPARATOR);
    println!();
    println!("  Summary:");
    println!("    • 2 schemas registered (User, Post)");
    println!("    • 4 nodes created (3 Users, 1 Post)");
    println!("    • 4 edges created (including 1 bidirectional)");
    println!("    • 1 node updated");
    println!("    • 1 edge deleted");
    println!("    • 1 node deleted");
    println!("    • Unique constraint enforced");
    println!("    • Required field validation enforced");
    println!("    • CDC entries verified for all operations");
    println!("    • CDC versionstamp ordering verified");
    println!("    • CDC consumer checkpoint + resume verified");
    println!();
}
