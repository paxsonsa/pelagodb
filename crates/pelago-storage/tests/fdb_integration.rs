//! Integration tests for FDB database operations
//!
//! These tests require a running FDB instance with native ARM64 binaries.
//! They are ignored by default due to x86_64 emulation issues on Apple Silicon.
//!
//! To run these tests:
//!   1. Ensure you have native FDB client library installed
//!   2. Start FDB: ./scripts/start-fdb.sh
//!   3. Run: LIBRARY_PATH=/usr/local/lib cargo test --test fdb_integration -- --ignored
//!
//! On CI (x86_64), these tests run automatically without --ignored.
//!
//! The tests use the cluster file at ./fdb.cluster

use pelago_storage::{PelagoDb, PelagoTxn};
use std::time::{SystemTime, UNIX_EPOCH};

/// Get a unique test prefix to avoid test interference
fn test_prefix() -> Vec<u8> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("test_{}_", ts).into_bytes()
}

/// Get the cluster file path
fn cluster_file() -> String {
    std::env::var("FDB_CLUSTER_FILE").unwrap_or_else(|_| "fdb.cluster".to_string())
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_connect_to_fdb() {
    let db = PelagoDb::connect(&cluster_file()).await;
    assert!(db.is_ok(), "Should connect to FDB: {:?}", db.err());
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_basic_get_set() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    let key: Vec<u8> = [&prefix[..], b"test_key"].concat();
    let value = b"test_value";

    // Set a value
    db.set(&key, value).await.expect("set");

    // Get the value back
    let result = db.get(&key).await.expect("get");
    assert_eq!(result, Some(value.to_vec()));

    // Clean up
    db.clear(&key).await.expect("clear");

    // Verify deletion
    let result = db.get(&key).await.expect("get after clear");
    assert_eq!(result, None);
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_get_nonexistent_key() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    let key: Vec<u8> = [&prefix[..], b"nonexistent"].concat();
    let result = db.get(&key).await.expect("get");
    assert_eq!(result, None);
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_overwrite_value() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    let key: Vec<u8> = [&prefix[..], b"overwrite_key"].concat();

    // Set initial value
    db.set(&key, b"value1").await.expect("set 1");

    // Overwrite
    db.set(&key, b"value2").await.expect("set 2");

    // Verify new value
    let result = db.get(&key).await.expect("get");
    assert_eq!(result, Some(b"value2".to_vec()));

    // Clean up
    db.clear(&key).await.expect("clear");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_get_range() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    // Insert multiple keys
    for i in 0u8..5 {
        let key: Vec<u8> = [&prefix[..], &[b'k', i]].concat();
        let value = vec![b'v', i];
        db.set(&key, &value).await.expect("set");
    }

    // Range query
    let begin: Vec<u8> = [&prefix[..], b"k"].concat();
    let end: Vec<u8> = [&prefix[..], b"l"].concat(); // After 'k' in sort order

    let results = db.get_range(&begin, &end, 10).await.expect("get_range");
    assert_eq!(results.len(), 5, "Should get all 5 keys");

    // Verify order
    for (i, (key, value)) in results.iter().enumerate() {
        let expected_suffix = [b'k', i as u8];
        assert!(key.ends_with(&expected_suffix), "Key should end with correct suffix");
        assert_eq!(value, &vec![b'v', i as u8]);
    }

    // Clean up
    db.clear_range(&begin, &end).await.expect("clear_range");

    // Verify deletion
    let results = db.get_range(&begin, &end, 10).await.expect("get_range after clear");
    assert_eq!(results.len(), 0);
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_range_with_limit() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    // Insert 10 keys
    for i in 0u8..10 {
        let key: Vec<u8> = [&prefix[..], &[b'm', i]].concat();
        db.set(&key, &[i]).await.expect("set");
    }

    // Query with limit
    let begin: Vec<u8> = [&prefix[..], b"m"].concat();
    let end: Vec<u8> = [&prefix[..], b"n"].concat();

    let results = db.get_range(&begin, &end, 3).await.expect("get_range");
    assert_eq!(results.len(), 3, "Should respect limit");

    // Clean up
    db.clear_range(&begin, &end).await.expect("clear_range");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_transaction_wrapper() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    let key1: Vec<u8> = [&prefix[..], b"txn_key1"].concat();
    let key2: Vec<u8> = [&prefix[..], b"txn_key2"].concat();

    // Create a transaction and do multiple operations
    let trx = db.create_transaction().expect("create_trx");
    let txn = PelagoTxn::new(trx);

    txn.set(&key1, b"value1");
    txn.set(&key2, b"value2");

    txn.commit().await.expect("commit");

    // Verify both keys were set
    let v1 = db.get(&key1).await.expect("get 1");
    let v2 = db.get(&key2).await.expect("get 2");

    assert_eq!(v1, Some(b"value1".to_vec()));
    assert_eq!(v2, Some(b"value2".to_vec()));

    // Clean up
    db.clear(&key1).await.expect("clear 1");
    db.clear(&key2).await.expect("clear 2");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_transaction_read_within() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    let key: Vec<u8> = [&prefix[..], b"read_within_txn"].concat();

    // Pre-set a value
    db.set(&key, b"existing").await.expect("pre-set");

    // Read within a transaction
    let trx = db.create_transaction().expect("create_trx");
    let txn = PelagoTxn::new(trx);

    let value = txn.get(&key).await.expect("get in txn");
    assert_eq!(value, Some(b"existing".to_vec()));

    // Update within same transaction
    txn.set(&key, b"updated");

    // Read again should see update (snapshot read within txn)
    // Note: FDB read-your-writes is enabled by default
    let value = txn.get(&key).await.expect("get after set in txn");
    assert_eq!(value, Some(b"updated".to_vec()));

    txn.commit().await.expect("commit");

    // Verify committed value
    let value = db.get(&key).await.expect("get after commit");
    assert_eq!(value, Some(b"updated".to_vec()));

    // Clean up
    db.clear(&key).await.expect("clear");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_binary_keys_and_values() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    // Test with binary data including nulls
    let key: Vec<u8> = [&prefix[..], &[0x00, 0x01, 0xFF, 0x00, 0x02]].concat();
    let value: Vec<u8> = vec![0x00, 0xFF, 0x00, 0xAB, 0xCD];

    db.set(&key, &value).await.expect("set binary");

    let result = db.get(&key).await.expect("get binary");
    assert_eq!(result, Some(value.clone()));

    // Clean up
    db.clear(&key).await.expect("clear");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_empty_value() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    let key: Vec<u8> = [&prefix[..], b"empty_value_key"].concat();
    let empty_value: Vec<u8> = vec![];

    db.set(&key, &empty_value).await.expect("set empty");

    let result = db.get(&key).await.expect("get empty");
    assert_eq!(result, Some(vec![]));

    // Clean up
    db.clear(&key).await.expect("clear");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_large_value() {
    let db = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let prefix = test_prefix();

    let key: Vec<u8> = [&prefix[..], b"large_value_key"].concat();

    // FDB supports values up to 100KB, but let's test with 10KB
    let large_value: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();

    db.set(&key, &large_value).await.expect("set large");

    let result = db.get(&key).await.expect("get large");
    assert_eq!(result, Some(large_value));

    // Clean up
    db.clear(&key).await.expect("clear");
}

#[tokio::test]
#[ignore = "requires native FDB - run with --ignored"]
async fn test_db_clone() {
    let db1 = PelagoDb::connect(&cluster_file()).await.expect("connect");
    let db2 = db1.clone();
    let prefix = test_prefix();

    let key: Vec<u8> = [&prefix[..], b"clone_test"].concat();

    // Write with db1
    db1.set(&key, b"from_db1").await.expect("set");

    // Read with db2
    let result = db2.get(&key).await.expect("get");
    assert_eq!(result, Some(b"from_db1".to_vec()));

    // Clean up with either
    db2.clear(&key).await.expect("clear");
}
