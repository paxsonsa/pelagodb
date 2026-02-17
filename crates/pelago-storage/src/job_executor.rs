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
use crate::subspace::edge_markers;
use crate::Subspace;
use async_trait::async_trait;
use pelago_core::encoding::{decode_cbor, encode_cbor, TupleBuilder};
use pelago_core::{PelagoError, Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
        JobType::IndexBackfill { .. } => Ok(Box::new(IndexBackfillExecutor { schema_registry })),
        JobType::StripProperty { .. } => Ok(Box::new(StripPropertyExecutor)),
        JobType::CdcRetention { .. } => Ok(Box::new(CdcRetentionExecutor)),
        JobType::OrphanedEdgeCleanup { .. } => Ok(Box::new(OrphanedEdgeCleanupExecutor)),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeDataEnvelope {
    properties: HashMap<String, Value>,
    locality: u8,
    created_at: i64,
    updated_at: i64,
}

enum NodePayload {
    Envelope(NodeDataEnvelope),
    LegacyMap(HashMap<String, Value>),
}

impl NodePayload {
    fn properties(&self) -> &HashMap<String, Value> {
        match self {
            NodePayload::Envelope(data) => &data.properties,
            NodePayload::LegacyMap(properties) => properties,
        }
    }

    fn properties_mut(&mut self) -> &mut HashMap<String, Value> {
        match self {
            NodePayload::Envelope(data) => &mut data.properties,
            NodePayload::LegacyMap(properties) => properties,
        }
    }

    fn encode(&self) -> Result<Vec<u8>, PelagoError> {
        match self {
            NodePayload::Envelope(data) => encode_cbor(data),
            NodePayload::LegacyMap(properties) => encode_cbor(properties),
        }
    }
}

fn decode_node_payload(bytes: &[u8]) -> Result<NodePayload, PelagoError> {
    if let Ok(data) = decode_cbor::<NodeDataEnvelope>(bytes) {
        return Ok(NodePayload::Envelope(data));
    }

    let properties = decode_cbor::<HashMap<String, Value>>(bytes)?;
    Ok(NodePayload::LegacyMap(properties))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EdgeMetaEnvelope {
    #[allow(dead_code)]
    edge_id: String,
    #[allow(dead_code)]
    properties: HashMap<String, Value>,
    #[allow(dead_code)]
    created_at: i64,
    #[serde(default)]
    source: Option<crate::edge::NodeRef>,
    #[serde(default)]
    target: Option<crate::edge::NodeRef>,
    #[serde(default)]
    label: Option<String>,
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
            None => data_subspace
                .pack()
                .add_string(&entity_type)
                .build()
                .to_vec(),
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

        let schema = self
            .schema_registry
            .get_schema(&job.database, &job.namespace, &entity_type)
            .await?
            .ok_or_else(|| PelagoError::UnregisteredType {
                entity_type: entity_type.clone(),
            })?;

        let trx = db.create_transaction()?;

        for (key, value) in &results {
            // Extract node_id from the last 9 bytes of the data key
            if key.len() < 9 {
                continue;
            }
            let node_id_bytes = &key[key.len() - 9..];

            let payload = decode_node_payload(value)?;

            // Compute index entries using the full schema
            let entries = index::compute_index_entries(
                &subspace,
                &entity_type,
                node_id_bytes,
                &schema,
                payload.properties(),
            )?;

            for entry in entries {
                let value = entry
                    .value_bytes
                    .as_ref()
                    .map(|b| b.as_ref())
                    .unwrap_or(&[]);
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
            JobType::StripProperty {
                entity_type,
                property_name,
            } => (entity_type.clone(), property_name.clone()),
            _ => return Err(PelagoError::Internal("Wrong executor for job type".into())),
        };

        let subspace = Subspace::namespace(&job.database, &job.namespace);
        let data_subspace = subspace.data();

        let range_start = match &job.progress_cursor {
            Some(cursor) => cursor.clone(),
            None => data_subspace
                .pack()
                .add_string(&entity_type)
                .build()
                .to_vec(),
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
            let mut payload = decode_node_payload(value)?;

            if payload.properties_mut().remove(&property_name).is_some() {
                let new_value = payload.encode()?;
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

// ─── OrphanedEdgeCleanup ────────────────────────────────────────────────

/// Removes edges that reference a deleted entity type.
pub struct OrphanedEdgeCleanupExecutor;

#[async_trait]
impl JobExecutor for OrphanedEdgeCleanupExecutor {
    async fn execute_batch(
        &self,
        job: &mut JobState,
        db: &PelagoDb,
        batch_size: usize,
    ) -> Result<bool, PelagoError> {
        let deleted_entity_type = match &job.job_type {
            JobType::OrphanedEdgeCleanup {
                deleted_entity_type,
            } => deleted_entity_type.clone(),
            _ => return Err(PelagoError::Internal("Wrong executor for job type".into())),
        };

        let edge_subspace = Subspace::namespace(&job.database, &job.namespace).edge();
        let meta_prefix = edge_subspace
            .pack()
            .add_marker(edge_markers::FORWARD_META)
            .build()
            .to_vec();

        let range_start = match &job.progress_cursor {
            Some(cursor) => cursor.clone(),
            None => meta_prefix.clone(),
        };
        let range_end = {
            let mut end = meta_prefix.clone();
            end.push(0xFF);
            end
        };

        let results = db.get_range(&range_start, &range_end, batch_size).await?;
        if results.is_empty() {
            return Ok(false);
        }

        let marker_pos = edge_subspace.prefix().len();
        let mut reverse_direction_forward_keys: HashSet<Vec<u8>> = HashSet::new();
        let mut has_clears = false;
        let trx = db.create_transaction()?;

        for (meta_key, meta_value) in &results {
            let edge: EdgeMetaEnvelope = match decode_cbor(meta_value) {
                Ok(edge) => edge,
                Err(_) => {
                    job.processed_items += 1;
                    continue;
                }
            };

            let (source, target, label) = match (
                edge.source.as_ref(),
                edge.target.as_ref(),
                edge.label.as_deref(),
            ) {
                (Some(source), Some(target), Some(label)) if !label.is_empty() => {
                    (source, target, label)
                }
                _ => {
                    job.processed_items += 1;
                    continue;
                }
            };

            let touches_deleted_type = source.entity_type == deleted_entity_type
                || target.entity_type == deleted_entity_type;
            if !touches_deleted_type {
                job.processed_items += 1;
                continue;
            }

            trx.clear(meta_key);
            has_clears = true;

            let mut forward_key = meta_key.clone();
            if forward_key.len() > marker_pos {
                forward_key[marker_pos] = edge_markers::FORWARD;
                trx.clear(&forward_key);
            }

            let reverse_key = edge_subspace
                .pack()
                .add_marker(edge_markers::REVERSE)
                .add_string(&target.database)
                .add_string(&target.namespace)
                .add_string(&target.entity_type)
                .add_string(&target.node_id)
                .add_string(label)
                .add_string(&source.entity_type)
                .add_string(&source.node_id)
                .build();
            trx.clear(reverse_key.as_ref());

            let reverse_direction_reverse_key = edge_subspace
                .pack()
                .add_marker(edge_markers::REVERSE)
                .add_string(&source.database)
                .add_string(&source.namespace)
                .add_string(&source.entity_type)
                .add_string(&source.node_id)
                .add_string(label)
                .add_string(&target.entity_type)
                .add_string(&target.node_id)
                .build();
            trx.clear(reverse_direction_reverse_key.as_ref());

            for key in find_reverse_direction_forward_keys(
                db,
                &edge_subspace,
                source,
                target,
                label,
                batch_size.max(128),
            )
            .await?
            {
                reverse_direction_forward_keys.insert(key);
            }

            job.processed_items += 1;
        }

        for key in reverse_direction_forward_keys {
            trx.clear(&key);
            has_clears = true;
        }

        if let Some((last_key, _)) = results.last() {
            let mut next_cursor = last_key.clone();
            next_cursor.push(0x00);
            job.progress_cursor = Some(next_cursor);
        }

        if has_clears {
            trx.commit().await.map_err(|e| {
                PelagoError::Internal(format!("Orphaned edge cleanup commit failed: {}", e))
            })?;
        }

        Ok(results.len() >= batch_size)
    }
}

async fn find_reverse_direction_forward_keys(
    db: &PelagoDb,
    edge_subspace: &Subspace,
    source: &crate::edge::NodeRef,
    target: &crate::edge::NodeRef,
    label: &str,
    scan_limit: usize,
) -> Result<Vec<Vec<u8>>, PelagoError> {
    let prefix = edge_subspace
        .pack()
        .add_marker(edge_markers::FORWARD)
        .add_string(&target.entity_type)
        .add_string(&target.node_id)
        .add_string(label)
        .build()
        .to_vec();
    let range_end = {
        let mut end = prefix.clone();
        end.push(0xFF);
        end
    };

    let source_suffix = TupleBuilder::new()
        .add_string(&source.database)
        .add_string(&source.namespace)
        .add_string(&source.entity_type)
        .add_string(&source.node_id)
        .build()
        .to_vec();

    let mut range_start = prefix.clone();
    let mut matches = Vec::new();
    let limit = scan_limit.max(1);

    loop {
        let batch = db.get_range(&range_start, &range_end, limit).await?;
        if batch.is_empty() {
            break;
        }

        for (key, _) in &batch {
            if key.ends_with(&source_suffix) {
                matches.push(key.clone());
            }
        }

        if batch.len() < limit {
            break;
        }

        if let Some((last_key, _)) = batch.last() {
            let mut next_start = last_key.clone();
            next_start.push(0x00);
            range_start = next_start;
        } else {
            break;
        }
    }

    Ok(matches)
}

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
            JobType::CdcRetention {
                max_retention_secs,
                checkpoint_aware,
            } => (*max_retention_secs, *checkpoint_aware),
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
            trx.commit().await.map_err(|e| {
                PelagoError::Internal(format!("CDC retention commit failed: {}", e))
            })?;
        }

        Ok(results.len() >= batch_size)
    }
}
