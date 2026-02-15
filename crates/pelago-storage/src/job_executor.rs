//! Job executor trait and implementations
//!
//! Each job type has an executor that processes work in batches. The executor
//! reads from a cursor position, processes `batch_size` items, advances the
//! cursor, and returns whether more work remains. This enables:
//!
//! - **Resumption**: If the worker crashes, the cursor in `JobState` indicates
//!   where to restart. No work is repeated beyond the last uncommitted batch.
//! - **Progress tracking**: `processed_items` and `progress_pct` update as
//!   batches complete.
//! - **Bounded transactions**: Each batch fits within FDB's 10MB transaction
//!   limit by processing a fixed number of items.

use crate::cdc::Versionstamp;
use crate::consumer::fetch_all_checkpoints;
use crate::db::PelagoDb;
use crate::index;
use crate::jobs::{JobState, JobType};
use crate::schema::SchemaRegistry;
use crate::Subspace;
use async_trait::async_trait;
use pelago_core::encoding::decode_cbor;
use pelago_core::PelagoError;
use std::sync::Arc;

/// Trait for job execution with cursor-based batch processing
#[async_trait]
pub trait JobExecutor: Send + Sync {
    /// Execute one batch of work. Returns `true` if more work remains.
    async fn execute_batch(
        &self,
        job: &mut JobState,
        db: &PelagoDb,
        batch_size: usize,
    ) -> Result<bool, PelagoError>;
}

/// Create the appropriate executor for a job type
pub fn executor_for_job(
    job: &JobState,
    schema_registry: Arc<SchemaRegistry>,
) -> Result<Box<dyn JobExecutor>, PelagoError> {
    match &job.job_type {
        JobType::IndexBackfill { .. } => Ok(Box::new(IndexBackfillExecutor {
            schema_registry,
        })),
        JobType::StripProperty { .. } => Ok(Box::new(StripPropertyExecutor)),
        JobType::CdcRetention { .. } => Ok(Box::new(CdcRetentionExecutor)),
        JobType::OrphanedEdgeCleanup { .. } => {
            Err(PelagoError::Internal("OrphanedEdgeCleanup not yet implemented".into()))
        }
    }
}

// ─── IndexBackfill ──────────────────────────────────────────────────────

/// Scans all nodes of an entity type and creates index entries.
///
/// For each node, decodes its properties, computes the required index
/// entries using the schema, and writes them to FDB. The node_id is
/// extracted from the last 9 bytes of the data key.
pub struct IndexBackfillExecutor {
    schema_registry: Arc<SchemaRegistry>,
}

#[async_trait]
impl JobExecutor for IndexBackfillExecutor {
    async fn execute_batch(
        &self,
        job: &mut JobState,
        db: &PelagoDb,
        batch_size: usize,
    ) -> Result<bool, PelagoError> {
        let entity_type = match &job.job_type {
            JobType::IndexBackfill { entity_type, .. } => entity_type.clone(),
            _ => return Err(PelagoError::Internal("Wrong executor for job type".into())),
        };

        let subspace = Subspace::namespace(&job.database, &job.namespace);
        let data_subspace = subspace.data();

        // Build scan range from cursor or beginning
        let range_start = match &job.progress_cursor {
            Some(cursor) => cursor.clone(),
            None => {
                data_subspace
                    .pack()
                    .add_string(&entity_type)
                    .build()
                    .to_vec()
            }
        };

        let range_end = {
            let mut end = data_subspace
                .pack()
                .add_string(&entity_type)
                .build()
                .to_vec();
            end.push(0xFF);
            end
        };

        let results = db.get_range(&range_start, &range_end, batch_size).await?;

        if results.is_empty() {
            return Ok(false);
        }

        let schema = self.schema_registry
            .get_schema(&job.database, &job.namespace, &entity_type)
            .await?
            .ok_or_else(|| PelagoError::UnregisteredType { entity_type: entity_type.clone() })?;

        let trx = db.create_transaction()?;

        for (key, value) in &results {
            // Extract node_id from the last 9 bytes of the data key
            if key.len() < 9 {
                continue;
            }
            let node_id_bytes = &key[key.len() - 9..];

            // Decode node properties
            let properties: std::collections::HashMap<String, pelago_core::Value> =
                decode_cbor(value)?;

            // Compute index entries using the full schema
            let entries = index::compute_index_entries(
                &subspace,
                &entity_type,
                node_id_bytes,
                &schema,
                &properties,
            )?;

            for entry in entries {
                let value = entry.value_bytes.as_ref().map(|b| b.as_ref()).unwrap_or(&[]);
                trx.set(entry.key.as_ref(), value);
            }

            job.processed_items += 1;
        }

        // Update cursor to the key after the last processed
        if let Some((last_key, _)) = results.last() {
            let mut next_cursor = last_key.clone();
            next_cursor.push(0x00);
            job.progress_cursor = Some(next_cursor);
        }

        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Index backfill commit failed: {}", e)))?;

        Ok(results.len() >= batch_size)
    }
}

// ─── StripProperty ──────────────────────────────────────────────────────

/// Removes a property from all nodes of an entity type by re-encoding
/// each node's properties without the target property.
pub struct StripPropertyExecutor;

#[async_trait]
impl JobExecutor for StripPropertyExecutor {
    async fn execute_batch(
        &self,
        job: &mut JobState,
        db: &PelagoDb,
        batch_size: usize,
    ) -> Result<bool, PelagoError> {
        let (entity_type, property_name) = match &job.job_type {
            JobType::StripProperty { entity_type, property_name } => {
                (entity_type.clone(), property_name.clone())
            }
            _ => return Err(PelagoError::Internal("Wrong executor for job type".into())),
        };

        let subspace = Subspace::namespace(&job.database, &job.namespace);
        let data_subspace = subspace.data();

        let range_start = match &job.progress_cursor {
            Some(cursor) => cursor.clone(),
            None => {
                data_subspace
                    .pack()
                    .add_string(&entity_type)
                    .build()
                    .to_vec()
            }
        };

        let range_end = {
            let mut end = data_subspace
                .pack()
                .add_string(&entity_type)
                .build()
                .to_vec();
            end.push(0xFF);
            end
        };

        let results = db.get_range(&range_start, &range_end, batch_size).await?;

        if results.is_empty() {
            return Ok(false);
        }

        let trx = db.create_transaction()?;

        for (key, value) in &results {
            let mut properties: std::collections::HashMap<String, pelago_core::Value> =
                decode_cbor(value)?;

            if properties.remove(&property_name).is_some() {
                let new_value = pelago_core::encoding::encode_cbor(&properties)?;
                trx.set(key, &new_value);
            }

            job.processed_items += 1;
        }

        if let Some((last_key, _)) = results.last() {
            let mut next_cursor = last_key.clone();
            next_cursor.push(0x00);
            job.progress_cursor = Some(next_cursor);
        }

        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Strip property commit failed: {}", e)))?;

        Ok(results.len() >= batch_size)
    }
}

// ─── CdcRetention ───────────────────────────────────────────────────────

/// Deletes expired CDC entries, optionally respecting consumer checkpoints.
///
/// When `checkpoint_aware` is true, the executor finds the minimum checkpoint
/// across all consumers and refuses to delete entries beyond that point. This
/// prevents deleting entries that a consumer hasn't processed yet.
pub struct CdcRetentionExecutor;

#[async_trait]
impl JobExecutor for CdcRetentionExecutor {
    async fn execute_batch(
        &self,
        job: &mut JobState,
        db: &PelagoDb,
        batch_size: usize,
    ) -> Result<bool, PelagoError> {
        let (max_retention_secs, checkpoint_aware) = match &job.job_type {
            JobType::CdcRetention { max_retention_secs, checkpoint_aware } => {
                (*max_retention_secs, *checkpoint_aware)
            }
            _ => return Err(PelagoError::Internal("Wrong executor for job type".into())),
        };

        let subspace = Subspace::namespace(&job.database, &job.namespace).cdc();

        // If checkpoint_aware, find the minimum consumer checkpoint
        let safe_cutoff = if checkpoint_aware {
            let checkpoints = fetch_all_checkpoints(db, &job.database, &job.namespace).await?;
            if checkpoints.is_empty() {
                None
            } else {
                Some(checkpoints.into_iter().map(|(_, vs)| vs).min().unwrap())
            }
        } else {
            None
        };

        let range_start = match &job.progress_cursor {
            Some(cursor) => cursor.clone(),
            None => subspace.prefix().to_vec(),
        };
        let range_end = subspace.range_end().to_vec();

        let results = db.get_range(&range_start, &range_end, batch_size).await?;

        if results.is_empty() {
            return Ok(false);
        }

        let trx = db.create_transaction()?;
        let prefix_len = subspace.prefix().len();
        let mut deleted = 0;

        for (key, value) in &results {
            let vs_bytes = &key[prefix_len..];
            if let Some(vs) = Versionstamp::from_bytes(vs_bytes) {
                // Don't delete past the minimum consumer checkpoint
                if let Some(ref cutoff) = safe_cutoff {
                    if &vs >= cutoff {
                        continue;
                    }
                }

                // Check timestamp for time-based retention
                if let Ok(entry) = decode_cbor::<crate::cdc::CdcEntry>(value) {
                    let now_us = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_micros() as i64;
                    let age_secs = (now_us - entry.timestamp) / 1_000_000;
                    if age_secs < max_retention_secs as i64 {
                        // Not old enough — entries are ordered, so stop
                        break;
                    }
                }

                trx.clear(key);
                deleted += 1;
            }

            job.processed_items += 1;
        }

        if let Some((last_key, _)) = results.last() {
            let mut next_cursor = last_key.clone();
            next_cursor.push(0x00);
            job.progress_cursor = Some(next_cursor);
        }

        if deleted > 0 {
            trx.commit()
                .await
                .map_err(|e| PelagoError::Internal(format!("CDC retention commit failed: {}", e)))?;
        }

        Ok(results.len() >= batch_size)
    }
}
