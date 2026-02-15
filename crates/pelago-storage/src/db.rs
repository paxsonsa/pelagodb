//! Database abstraction for FDB operations
//!
//! This module provides a high-level interface for interacting with FoundationDB,
//! handling connection management, transactions, and common operations.

use foundationdb::api::{FdbApiBuilder, NetworkAutoStop};
use foundationdb::{Database, FdbBindingError, MaybeCommitted, RangeOption, RetryableTransaction, Transaction};
use pelago_core::PelagoError;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::OnceCell;

/// Global FDB network initialization
///
/// The NetworkAutoStop guard must be kept alive for the entire lifetime of the process.
/// When dropped, it stops the FDB network thread, causing all operations to fail with
/// "Broken promise" errors.
static FDB_NETWORK: OnceCell<NetworkAutoStop> = OnceCell::const_new();

/// Initialize the FDB network (must be called once before any FDB operations)
///
/// Uses API version 730 for compatibility with FDB 7.3.x servers.
/// The returned guard is stored in a static and kept alive for the process lifetime.
pub async fn init_fdb_network() -> Result<(), PelagoError> {
    FDB_NETWORK
        .get_or_try_init(|| async {
            // Use API version 730 for FDB 7.3.x compatibility
            let network_builder = FdbApiBuilder::default()
                .set_runtime_version(730)
                .build()
                .map_err(|e| PelagoError::FdbUnavailable(format!("Failed to build FDB API: {}", e)))?;

            // Safety: boot() must only be called once per process, which we ensure
            // via the OnceCell. The FDB network thread will run for the lifetime
            // of the process. The returned NetworkAutoStop guard is stored in the
            // OnceCell to keep the network alive.
            let guard = unsafe {
                network_builder.boot().map_err(|e| {
                    PelagoError::FdbUnavailable(format!("Failed to start FDB network: {}", e))
                })?
            };

            Ok(guard)
        })
        .await?;
    Ok(())
}

/// PelagoDB database handle wrapping FDB
///
/// This type is Clone via the internal Arc, allowing multiple handles
/// to share the same FDB connection.
#[derive(Clone)]
pub struct PelagoDb {
    fdb: Arc<Database>,
}

impl PelagoDb {
    /// Connect to FDB using the specified cluster file
    pub async fn connect(cluster_file: &str) -> Result<Self, PelagoError> {
        // Ensure network is initialized
        init_fdb_network().await?;

        let fdb = Database::new(Some(cluster_file))
            .map_err(|e| PelagoError::FdbUnavailable(format!("Failed to connect: {}", e)))?;

        Ok(Self { fdb: Arc::new(fdb) })
    }

    /// Connect using default cluster file location
    pub async fn connect_default() -> Result<Self, PelagoError> {
        Self::connect("/etc/foundationdb/fdb.cluster").await
    }

    /// Run a transactional operation with automatic retry
    ///
    /// The closure receives a `RetryableTransaction` (which can be converted to a regular
    /// `Transaction`) and a `MaybeCommitted` for accessing previously committed values.
    /// FDB will automatically retry on retryable errors.
    pub async fn transact<F, Fut, R>(&self, f: F) -> Result<R, PelagoError>
    where
        F: Fn(RetryableTransaction, MaybeCommitted) -> Fut,
        Fut: Future<Output = Result<R, FdbBindingError>>,
    {
        self.fdb
            .run(|trx, maybe_committed| f(trx, maybe_committed))
            .await
            .map_err(|e| PelagoError::Internal(format!("Transaction failed: {}", e)))
    }

    /// Run an async transactional operation
    ///
    /// Unlike `transact`, this creates a single transaction without automatic retry.
    /// The caller is responsible for handling conflicts if needed.
    pub async fn transact_boxed<F, Fut, R>(&self, f: F) -> Result<R, PelagoError>
    where
        F: FnOnce(Transaction) -> Fut,
        Fut: std::future::Future<Output = Result<R, PelagoError>>,
    {
        let trx = self.fdb.create_trx().map_err(|e| {
            PelagoError::FdbUnavailable(format!("Failed to create transaction: {}", e))
        })?;

        let result = f(trx).await?;

        Ok(result)
    }

    /// Create a new transaction for manual control
    pub fn create_transaction(&self) -> Result<Transaction, PelagoError> {
        self.fdb.create_trx().map_err(|e| {
            PelagoError::FdbUnavailable(format!("Failed to create transaction: {}", e))
        })
    }

    /// Get a value by key
    pub async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, PelagoError> {
        let trx = self.fdb.create_trx().map_err(|e| {
            PelagoError::FdbUnavailable(format!("Failed to create transaction: {}", e))
        })?;

        let result = trx.get(key, false).await.map_err(|e| {
            PelagoError::Internal(format!("Get failed: {}", e))
        })?;

        Ok(result.map(|v| v.to_vec()))
    }

    /// Set a key-value pair
    pub async fn set(&self, key: &[u8], value: &[u8]) -> Result<(), PelagoError> {
        let trx = self.fdb.create_trx().map_err(|e| {
            PelagoError::FdbUnavailable(format!("Failed to create transaction: {}", e))
        })?;

        trx.set(key, value);

        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Commit failed: {}", e)))?;

        Ok(())
    }

    /// Delete a key
    pub async fn clear(&self, key: &[u8]) -> Result<(), PelagoError> {
        let trx = self.fdb.create_trx().map_err(|e| {
            PelagoError::FdbUnavailable(format!("Failed to create transaction: {}", e))
        })?;

        trx.clear(key);

        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Commit failed: {}", e)))?;

        Ok(())
    }

    /// Clear a range of keys
    pub async fn clear_range(&self, begin: &[u8], end: &[u8]) -> Result<(), PelagoError> {
        let trx = self.fdb.create_trx().map_err(|e| {
            PelagoError::FdbUnavailable(format!("Failed to create transaction: {}", e))
        })?;

        trx.clear_range(begin, end);

        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Commit failed: {}", e)))?;

        Ok(())
    }

    /// Get a range of key-value pairs
    pub async fn get_range(
        &self,
        begin: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, PelagoError> {
        let trx = self.fdb.create_trx().map_err(|e| {
            PelagoError::FdbUnavailable(format!("Failed to create transaction: {}", e))
        })?;

        let range_option = RangeOption {
            limit: Some(limit),
            ..RangeOption::from((begin, end))
        };

        // iteration must be >= 1 (it's used for streaming pagination)
        let result = trx.get_range(&range_option, 1, false).await.map_err(|e| {
            PelagoError::Internal(format!("Get range failed: {}", e))
        })?;

        Ok(result
            .iter()
            .map(|kv| (kv.key().to_vec(), kv.value().to_vec()))
            .collect())
    }

    /// Get underlying FDB database for advanced operations
    pub fn fdb(&self) -> &Database {
        &self.fdb
    }

    /// Read version for the current snapshot window.
    pub async fn get_read_version(&self) -> Result<i64, PelagoError> {
        let trx = self.fdb.create_trx().map_err(|e| {
            PelagoError::FdbUnavailable(format!("Failed to create transaction: {}", e))
        })?;
        trx.get_read_version()
            .await
            .map_err(|e| PelagoError::Internal(format!("Get read version failed: {}", e)))
    }
}

/// Transaction wrapper for multi-operation transactions
///
/// This provides a higher-level API over FDB transactions with
/// methods that return futures for async operations.
pub struct PelagoTxn {
    trx: Transaction,
}

impl PelagoTxn {
    pub fn new(trx: Transaction) -> Self {
        Self { trx }
    }

    /// Get a value within this transaction
    pub async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, PelagoError> {
        let result = self.trx.get(key, false).await.map_err(|e| {
            PelagoError::Internal(format!("Get failed: {}", e))
        })?;
        Ok(result.map(|v| v.to_vec()))
    }

    /// Set a value within this transaction
    pub fn set(&self, key: &[u8], value: &[u8]) {
        self.trx.set(key, value);
    }

    /// Clear a key within this transaction
    pub fn clear(&self, key: &[u8]) {
        self.trx.clear(key);
    }

    /// Clear a range within this transaction
    pub fn clear_range(&self, begin: &[u8], end: &[u8]) {
        self.trx.clear_range(begin, end);
    }

    /// Commit this transaction
    pub async fn commit(self) -> Result<(), PelagoError> {
        self.trx
            .commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Commit failed: {}", e)))?;
        Ok(())
    }

    /// Get a range of key-value pairs within this transaction
    pub async fn get_range(
        &self,
        begin: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, PelagoError> {
        let range_option = RangeOption {
            limit: Some(limit),
            ..RangeOption::from((begin, end))
        };

        // iteration must be >= 1 (it's used for streaming pagination)
        let result = self.trx.get_range(&range_option, 1, false).await.map_err(|e| {
            PelagoError::Internal(format!("Get range failed: {}", e))
        })?;

        Ok(result
            .iter()
            .map(|kv| (kv.key().to_vec(), kv.value().to_vec()))
            .collect())
    }

    /// Get access to underlying transaction for advanced operations
    pub fn inner(&self) -> &Transaction {
        &self.trx
    }
}
