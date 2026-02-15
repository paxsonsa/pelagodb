//! Background Job Integration Tests
//!
//! Tests job creation, execution, and cursor-based resumption for background jobs.
//!
//! Run with:
//!   LIBRARY_PATH=/usr/local/lib cargo test --test job_integration -- --ignored --nocapture

use pelago_core::schema::{EntitySchema, IndexType, PropertyDef};
use pelago_core::{PropertyType, Value};
use pelago_storage::{
    IdAllocator, JobState, JobStatus, JobStore, JobType, NodeStore, PelagoDb, SchemaCache,
    SchemaRegistry,
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

    (db, database, namespace, schema_registry, node_store, job_store)
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
