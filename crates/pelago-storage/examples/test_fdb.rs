//! Simple FDB connection test with detailed error output
//!
//! Run with:
//!   LIBRARY_PATH=/usr/local/lib cargo run --example test_fdb

use foundationdb::api::FdbApiBuilder;
use foundationdb::Database;
use std::env;

#[tokio::main]
async fn main() {
    println!("FDB Connection Test (Direct)");
    println!("============================");

    let cluster_file = env::var("FDB_CLUSTER_FILE").unwrap_or_else(|_| "fdb.cluster".to_string());
    println!("Using cluster file: {}", cluster_file);

    // Read and display cluster file contents
    match std::fs::read_to_string(&cluster_file) {
        Ok(contents) => println!("Cluster file contents: {}", contents.trim()),
        Err(e) => {
            println!("ERROR: Cannot read cluster file: {}", e);
            return;
        }
    }

    println!("\nInitializing FDB network (API version 730)...");

    let network = match FdbApiBuilder::default()
        .set_runtime_version(730)
        .build()
    {
        Ok(n) => n,
        Err(e) => {
            println!("ERROR: Failed to build FDB API: {:?}", e);
            return;
        }
    };

    println!("FDB API built, booting network...");

    let _network_stop = unsafe {
        match network.boot() {
            Ok(stop) => {
                println!("Network booted successfully!");
                stop
            }
            Err(e) => {
                println!("ERROR: Failed to boot network: {:?}", e);
                return;
            }
        }
    };

    // Give the network a moment to fully initialize
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    println!("\nOpening database...");
    let db = match Database::new(Some(&cluster_file)) {
        Ok(db) => {
            println!("Database opened successfully!");
            db
        }
        Err(e) => {
            println!("ERROR: Failed to open database: {:?}", e);
            return;
        }
    };

    println!("\nCreating transaction...");
    let trx = match db.create_trx() {
        Ok(trx) => {
            println!("Transaction created!");
            trx
        }
        Err(e) => {
            println!("ERROR: Failed to create transaction: {:?}", e);
            return;
        }
    };

    println!("\nSetting key...");
    trx.set(b"pelago_test_key", b"pelago_test_value");
    println!("Set operation queued.");

    println!("\nCommitting transaction...");
    match trx.commit().await {
        Ok(_) => println!("Commit successful!"),
        Err(e) => {
            println!("ERROR: Commit failed!");
            println!("  Error code: {}", e.code());
            println!("  Error message: {}", e.message());
            return;
        }
    }

    println!("\nReading key...");
    let trx2 = db.create_trx().expect("create read trx");
    match trx2.get(b"pelago_test_key", false).await {
        Ok(Some(v)) => {
            println!("Read successful!");
            println!("Value: {:?}", String::from_utf8_lossy(&v));
        }
        Ok(None) => println!("ERROR: Key not found after write!"),
        Err(e) => {
            println!("ERROR: Read failed!");
            println!("  Error code: {}", e.code());
            println!("  Error message: {}", e.message());
            return;
        }
    }

    println!("\nCleaning up...");
    let trx3 = db.create_trx().expect("create cleanup trx");
    trx3.clear(b"pelago_test_key");
    match trx3.commit().await {
        Ok(_) => println!("Cleanup successful!"),
        Err(e) => {
            println!("ERROR: Cleanup failed!");
            println!("  Error code: {}", e.code());
            println!("  Error message: {}", e.message());
        }
    }

    println!("\nAll tests passed!");
}
