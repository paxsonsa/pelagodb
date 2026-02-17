//! Background Job Integration Tests
//!
//! Tests job creation, execution, and cursor-based resumption for background jobs.
//!
//! Run with:
//!   LIBRARY_PATH=/usr/local/lib cargo test --test job_integration -- --ignored --nocapture

use pelago_core::schema::{EdgeDef, EdgeTarget, EntitySchema, IndexType, PropertyDef};
use pelago_core::{PropertyType, Value};
use pelago_storage::job_executor::executor_for_job;
use pelago_storage::{
    EdgeStore, IdAllocator, JobStatus, JobStore, JobType, NodeRef, NodeStore, PelagoDb,
    SchemaCache, SchemaRegistry, Subspace,
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
    format!("job_test_{}", ts)
}

/// Helper to set up common test infrastructure
async fn setup() -> (
    PelagoDb,
    String,
    String,
    Arc<SchemaRegistry>,
    Arc<NodeStore>,
    JobStore,
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
        site_id,
    ));
    let job_store = JobStore::new(db.clone());

    (
        db,
        database,
        namespace,
        schema_registry,
        node_store,
        job_store,
    )
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_job_lifecycle() {
    let (_db, database, namespace, _schema_registry, _node_store, job_store) = setup().await;

    // Create a job
    let job = job_store
        .create_job(
            &database,
            &namespace,
            JobType::StripProperty {
                entity_type: "User".to_string(),
                property_name: "deprecated_field".to_string(),
            },
        )
        .await
        .expect("Failed to create job");

    println!("✓ Created job: {}", job.job_id);
    assert_eq!(job.status, JobStatus::Pending);
    assert_eq!(job.database, database);
    assert_eq!(job.namespace, namespace);

    // Retrieve the job
    let retrieved = job_store
        .get_job(&database, &namespace, &job.job_id)
        .await
        .expect("Failed to get job")
        .expect("Job should exist");

    assert_eq!(retrieved.job_id, job.job_id);
    assert_eq!(retrieved.status, JobStatus::Pending);
    println!("✓ Retrieved job by ID");

    // Update the job status
    let mut updated = retrieved;
    updated.status = JobStatus::Running;
    updated.processed_items = 50;
    updated.progress_pct = 0.5;
    job_store
        .update_job(&updated)
        .await
        .expect("Failed to update job");
    println!("✓ Updated job to Running");

    // Verify update persisted
    let reloaded = job_store
        .get_job(&database, &namespace, &job.job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.status, JobStatus::Running);
    assert_eq!(reloaded.processed_items, 50);

    // Find actionable jobs
    let actionable = job_store
        .find_actionable_jobs(&database, &namespace)
        .await
        .expect("Failed to find actionable jobs");

    let found = actionable.iter().any(|j| j.job_id == job.job_id);
    assert!(found, "Running job should be actionable");
    println!("✓ Running job appears in actionable jobs list");

    // Mark completed
    let mut completed = reloaded;
    completed.status = JobStatus::Completed;
    completed.progress_pct = 1.0;
    completed.processed_items = 100;
    job_store.update_job(&completed).await.unwrap();

    // Completed jobs should NOT be actionable
    let actionable2 = job_store
        .find_actionable_jobs(&database, &namespace)
        .await
        .unwrap();
    let found2 = actionable2.iter().any(|j| j.job_id == job.job_id);
    assert!(!found2, "Completed job should not be actionable");
    println!("✓ Completed job removed from actionable list");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_job_cursor_persistence() {
    let (_db, database, namespace, _schema_registry, _node_store, job_store) = setup().await;

    // Create a job and simulate partial progress with a cursor
    let job = job_store
        .create_job(
            &database,
            &namespace,
            JobType::IndexBackfill {
                entity_type: "User".to_string(),
                property_name: "email".to_string(),
                index_type: IndexType::Unique,
            },
        )
        .await
        .unwrap();

    let cursor = b"some_progress_cursor_value".to_vec();
    let mut updated = job;
    updated.status = JobStatus::Running;
    updated.progress_cursor = Some(cursor.clone());
    updated.processed_items = 500;
    updated.progress_pct = 0.25;
    job_store.update_job(&updated).await.unwrap();

    // Reload and verify cursor is preserved
    let reloaded = job_store
        .get_job(&database, &namespace, &updated.job_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reloaded.progress_cursor, Some(cursor));
    assert_eq!(reloaded.processed_items, 500);
    println!("✓ Job cursor persisted correctly for resumption");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_failed_job_preserves_error() {
    let (_db, database, namespace, _schema_registry, _node_store, job_store) = setup().await;

    let job = job_store
        .create_job(
            &database,
            &namespace,
            JobType::CdcRetention {
                max_retention_secs: 3600,
                checkpoint_aware: true,
            },
        )
        .await
        .unwrap();

    let mut failed = job;
    failed.status = JobStatus::Failed;
    failed.error = Some("FDB transaction conflict after 5 retries".to_string());
    job_store.update_job(&failed).await.unwrap();

    let reloaded = job_store
        .get_job(&database, &namespace, &failed.job_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reloaded.status, JobStatus::Failed);
    assert_eq!(
        reloaded.error.as_deref(),
        Some("FDB transaction conflict after 5 retries")
    );
    println!("✓ Failed job preserves error message");
}

// =========================================================================
// Execution-level tests
// =========================================================================

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_index_backfill_job() {
    let (db, database, namespace, schema_registry, node_store, job_store) = setup().await;

    // 1. Register schema WITHOUT index on 'age'
    let schema_v1 = EntitySchema::new("Worker")
        .with_property("name", PropertyDef::new(PropertyType::String).required())
        .with_property("age", PropertyDef::new(PropertyType::Int));

    schema_registry
        .register_schema(&database, &namespace, schema_v1)
        .await
        .expect("Failed to register schema v1");
    println!("✓ Registered schema v1 (no index on age)");

    // 2. Create several nodes with 'age' property
    let node_count = 5;
    let mut node_ids = Vec::new();
    for i in 0..node_count {
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String(format!("Worker_{}", i)));
        props.insert("age".to_string(), Value::Int(20 + i));

        let node = node_store
            .create_node(&database, &namespace, "Worker", props)
            .await
            .expect("Failed to create node");
        node_ids.push(node.id.clone());
    }
    println!("✓ Created {} nodes with age property", node_count);

    // 3. Verify no index entries exist yet (scan the index subspace for Worker/age)
    let subspace = Subspace::namespace(&database, &namespace);
    let idx_subspace = subspace.index();
    let idx_range_start = idx_subspace
        .pack()
        .add_string("Worker")
        .add_string("age")
        .build()
        .to_vec();
    let mut idx_range_end = idx_range_start.clone();
    idx_range_end.push(0xFF);

    let before = db
        .get_range(&idx_range_start, &idx_range_end, 1000)
        .await
        .unwrap();
    assert!(
        before.is_empty(),
        "No index entries should exist before backfill"
    );
    println!("✓ Verified no index entries exist before schema update");

    // 4. Update schema to add Range index on 'age'
    let schema_v2 = EntitySchema::new("Worker")
        .with_property("name", PropertyDef::new(PropertyType::String).required())
        .with_property(
            "age",
            PropertyDef::new(PropertyType::Int).with_index(IndexType::Range),
        );

    let v2 = schema_registry
        .register_schema(&database, &namespace, schema_v2)
        .await
        .expect("Failed to register schema v2");
    assert_eq!(v2, 2);
    println!("✓ Registered schema v2 (Range index on age)");

    // 5. Verify IndexBackfill job was auto-created by schema evolution (M9.4)
    let jobs = job_store
        .find_actionable_jobs(&database, &namespace)
        .await
        .expect("Failed to find jobs");

    let backfill_job = jobs
        .iter()
        .find(|j| {
            matches!(
                &j.job_type,
                JobType::IndexBackfill {
                    entity_type,
                    property_name,
                    ..
                } if entity_type == "Worker" && property_name == "age"
            )
        })
        .expect("IndexBackfill job should have been auto-created");
    println!("✓ IndexBackfill job auto-created: {}", backfill_job.job_id);

    // 6. Run the executor to completion
    let mut job = backfill_job.clone();
    let executor =
        executor_for_job(&job, Arc::clone(&schema_registry)).expect("Failed to create executor");

    job.status = JobStatus::Running;
    loop {
        let more = executor
            .execute_batch(&mut job, &db, 100)
            .await
            .expect("Batch execution failed");
        if !more {
            break;
        }
    }
    job.status = JobStatus::Completed;
    job_store.update_job(&job).await.unwrap();
    println!(
        "✓ IndexBackfill executor completed, processed {} items",
        job.processed_items
    );

    // 7. Verify index entries now exist for all nodes
    let after = db
        .get_range(&idx_range_start, &idx_range_end, 1000)
        .await
        .unwrap();
    assert_eq!(
        after.len(),
        node_count as usize,
        "Should have one index entry per node"
    );
    println!(
        "✓ Verified {} index entries created by backfill",
        after.len()
    );
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_cdc_retention_job() {
    let (db, database, namespace, schema_registry, node_store, job_store) = setup().await;

    // 1. Register schema and create mutations (which generate CDC entries)
    let schema = EntitySchema::new("Ephemeral")
        .with_property("data", PropertyDef::new(PropertyType::String).required());

    schema_registry
        .register_schema(&database, &namespace, schema)
        .await
        .unwrap();

    for i in 0..3 {
        let mut props = HashMap::new();
        props.insert("data".to_string(), Value::String(format!("item_{}", i)));
        node_store
            .create_node(&database, &namespace, "Ephemeral", props)
            .await
            .unwrap();
    }
    println!("✓ Created 3 nodes (generating CDC entries)");

    // 2. Verify CDC entries exist
    let cdc_subspace = Subspace::namespace(&database, &namespace).cdc();
    let cdc_start = cdc_subspace.prefix().to_vec();
    let cdc_end = cdc_subspace.range_end().to_vec();
    let entries_before = db.get_range(&cdc_start, &cdc_end, 1000).await.unwrap();
    assert!(
        !entries_before.is_empty(),
        "CDC entries should exist after mutations"
    );
    println!(
        "✓ Found {} CDC entries before retention",
        entries_before.len()
    );

    // 3. Create CdcRetention job with 0-second retention (delete everything)
    let mut job = job_store
        .create_job(
            &database,
            &namespace,
            JobType::CdcRetention {
                max_retention_secs: 0,
                checkpoint_aware: false,
            },
        )
        .await
        .unwrap();
    println!("✓ Created CdcRetention job with 0s retention");

    // 4. Run the executor
    let executor = executor_for_job(&job, Arc::clone(&schema_registry))
        .expect("Failed to create CdcRetention executor");

    job.status = JobStatus::Running;
    loop {
        let more = executor
            .execute_batch(&mut job, &db, 100)
            .await
            .expect("CDC retention batch failed");
        if !more {
            break;
        }
    }
    println!(
        "✓ CdcRetention executor completed, processed {} items",
        job.processed_items
    );

    // 5. Verify CDC entries are deleted
    let entries_after = db.get_range(&cdc_start, &cdc_end, 1000).await.unwrap();
    assert!(
        entries_after.len() < entries_before.len(),
        "CDC entries should be reduced after retention (before={}, after={})",
        entries_before.len(),
        entries_after.len()
    );
    println!(
        "✓ CDC entries reduced from {} to {} after retention",
        entries_before.len(),
        entries_after.len()
    );
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_job_resumes_after_interruption() {
    let (db, database, namespace, schema_registry, node_store, job_store) = setup().await;

    // 1. Register schema with an index
    let schema = EntitySchema::new("Resumable")
        .with_property("name", PropertyDef::new(PropertyType::String).required())
        .with_property(
            "score",
            PropertyDef::new(PropertyType::Int).with_index(IndexType::Range),
        );

    schema_registry
        .register_schema(&database, &namespace, schema)
        .await
        .unwrap();

    // 2. Create enough nodes for multiple small batches
    let node_count = 10;
    for i in 0..node_count {
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String(format!("node_{}", i)));
        props.insert("score".to_string(), Value::Int(i * 10));
        node_store
            .create_node(&database, &namespace, "Resumable", props)
            .await
            .unwrap();
    }
    println!("✓ Created {} nodes for resumption test", node_count);

    // 3. Create IndexBackfill job manually (simulating schema evolution)
    let mut job = job_store
        .create_job(
            &database,
            &namespace,
            JobType::IndexBackfill {
                entity_type: "Resumable".to_string(),
                property_name: "score".to_string(),
                index_type: IndexType::Range,
            },
        )
        .await
        .unwrap();

    // 4. Run only ONE batch with small batch_size (simulate interruption)
    let executor = executor_for_job(&job, Arc::clone(&schema_registry)).unwrap();

    job.status = JobStatus::Running;
    let more = executor
        .execute_batch(&mut job, &db, 3) // Only process 3 items
        .await
        .expect("First batch failed");

    let items_after_first_batch = job.processed_items;
    let cursor_after_first_batch = job.progress_cursor.clone();
    println!(
        "✓ First batch processed {} items, more_work={}",
        items_after_first_batch, more
    );
    assert!(
        cursor_after_first_batch.is_some(),
        "Cursor should be set after first batch"
    );

    // 5. Save progress (simulating what JobWorker does before crash)
    job_store.update_job(&job).await.unwrap();

    // 6. Simulate restart: reload job from store, create new executor, resume
    let mut resumed_job = job_store
        .get_job(&database, &namespace, &job.job_id)
        .await
        .unwrap()
        .expect("Job should exist after save");

    assert_eq!(resumed_job.processed_items, items_after_first_batch);
    assert_eq!(resumed_job.progress_cursor, cursor_after_first_batch);
    println!("✓ Job reloaded with preserved cursor and progress");

    let executor2 = executor_for_job(&resumed_job, Arc::clone(&schema_registry)).unwrap();

    // Run remaining batches to completion
    loop {
        let more = executor2
            .execute_batch(&mut resumed_job, &db, 3)
            .await
            .expect("Resumed batch failed");
        if !more {
            break;
        }
    }
    println!(
        "✓ Resumed job completed with total {} items processed",
        resumed_job.processed_items
    );

    // 7. Verify all index entries exist
    let subspace = Subspace::namespace(&database, &namespace);
    let idx_subspace = subspace.index();
    let idx_start = idx_subspace
        .pack()
        .add_string("Resumable")
        .add_string("score")
        .build()
        .to_vec();
    let mut idx_end = idx_start.clone();
    idx_end.push(0xFF);

    let index_entries = db.get_range(&idx_start, &idx_end, 1000).await.unwrap();
    assert_eq!(
        index_entries.len(),
        node_count as usize,
        "All nodes should have index entries after resumed backfill"
    );
    println!(
        "✓ All {} index entries verified after interrupted+resumed backfill",
        index_entries.len()
    );
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored --nocapture"]
async fn test_orphaned_edge_cleanup_job() {
    let (db, database, namespace, schema_registry, node_store, job_store) = setup().await;

    let deleted_schema = EntitySchema::new("Deleted")
        .with_property("name", PropertyDef::new(PropertyType::String).required())
        .with_edge(
            "LINKED_TO",
            EdgeDef::new(EdgeTarget::specific("Other")).bidirectional(),
        );
    schema_registry
        .register_schema(&database, &namespace, deleted_schema)
        .await
        .unwrap();

    let other_schema = EntitySchema::new("Other")
        .with_property("name", PropertyDef::new(PropertyType::String).required());
    schema_registry
        .register_schema(&database, &namespace, other_schema)
        .await
        .unwrap();

    let mut deleted_props = HashMap::new();
    deleted_props.insert("name".to_string(), Value::String("gone".to_string()));
    let deleted_node = node_store
        .create_node(&database, &namespace, "Deleted", deleted_props)
        .await
        .unwrap();

    let mut other_props = HashMap::new();
    other_props.insert("name".to_string(), Value::String("kept".to_string()));
    let other_node = node_store
        .create_node(&database, &namespace, "Other", other_props)
        .await
        .unwrap();

    let edge_store = EdgeStore::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::new(IdAllocator::new(db.clone(), 1, 100)),
        Arc::clone(&node_store),
        "1".to_string(),
    );
    edge_store
        .create_edge(
            &database,
            &namespace,
            NodeRef::new(&database, &namespace, "Deleted", &deleted_node.id),
            NodeRef::new(&database, &namespace, "Other", &other_node.id),
            "LINKED_TO",
            HashMap::new(),
        )
        .await
        .unwrap();

    let edge_subspace = Subspace::namespace(&database, &namespace).edge();
    let before = db
        .get_range(
            edge_subspace.prefix(),
            &edge_subspace.range_end().to_vec(),
            1000,
        )
        .await
        .unwrap();
    assert!(
        !before.is_empty(),
        "edge keys should exist before orphan cleanup"
    );

    let mut job = job_store
        .create_job(
            &database,
            &namespace,
            JobType::OrphanedEdgeCleanup {
                deleted_entity_type: "Deleted".to_string(),
            },
        )
        .await
        .unwrap();
    let executor = executor_for_job(&job, Arc::clone(&schema_registry)).unwrap();

    job.status = JobStatus::Running;
    loop {
        let more = executor.execute_batch(&mut job, &db, 64).await.unwrap();
        if !more {
            break;
        }
    }
    assert!(
        job.processed_items > 0,
        "cleanup job should process at least one metadata row"
    );

    let after = db
        .get_range(
            edge_subspace.prefix(),
            &edge_subspace.range_end().to_vec(),
            1000,
        )
        .await
        .unwrap();
    assert!(
        after.is_empty(),
        "all edge keys should be removed by cleanup"
    );
}
