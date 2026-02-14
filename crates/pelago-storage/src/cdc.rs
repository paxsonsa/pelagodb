//! Change Data Capture (CDC) entry emission
//!
//! CDC entries are written with versionstamp keys for zero-contention ordering.
//! The CDC consumer framework is implemented in Phase 2.
//!
//! FDB Key Layout:
//! ```text
//! (db, ns, _cdc, versionstamp) → CBOR CdcEntry
//! ```

use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::encoding::encode_cbor;
use pelago_core::{PelagoError, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// CDC operation type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CdcOperation {
    Create,
    Update,
    Delete,
}

/// CDC entry for a node mutation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CdcEntry {
    /// Operation type
    pub operation: CdcOperation,
    /// Entity type
    pub entity_type: String,
    /// Node ID
    pub node_id: String,
    /// Namespace
    pub namespace: String,
    /// Database
    pub database: String,
    /// Old property values (for update/delete)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_values: Option<HashMap<String, Value>>,
    /// New property values (for create/update)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_values: Option<HashMap<String, Value>>,
    /// Timestamp of the operation
    pub timestamp: i64,
}

impl CdcEntry {
    pub fn create(
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        values: HashMap<String, Value>,
    ) -> Self {
        Self {
            operation: CdcOperation::Create,
            entity_type: entity_type.to_string(),
            node_id: node_id.to_string(),
            namespace: ns.to_string(),
            database: db.to_string(),
            old_values: None,
            new_values: Some(values),
            timestamp: now(),
        }
    }

    pub fn update(
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        old_values: HashMap<String, Value>,
        new_values: HashMap<String, Value>,
    ) -> Self {
        Self {
            operation: CdcOperation::Update,
            entity_type: entity_type.to_string(),
            node_id: node_id.to_string(),
            namespace: ns.to_string(),
            database: db.to_string(),
            old_values: Some(old_values),
            new_values: Some(new_values),
            timestamp: now(),
        }
    }

    pub fn delete(
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        old_values: HashMap<String, Value>,
    ) -> Self {
        Self {
            operation: CdcOperation::Delete,
            entity_type: entity_type.to_string(),
            node_id: node_id.to_string(),
            namespace: ns.to_string(),
            database: db.to_string(),
            old_values: Some(old_values),
            new_values: None,
            timestamp: now(),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64
}

/// CDC writer for persisting change entries
pub struct CdcWriter {
    db: PelagoDb,
}

impl CdcWriter {
    pub fn new(db: PelagoDb) -> Self {
        Self { db }
    }

    /// Write a create CDC entry
    pub async fn write_create(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        properties: &HashMap<String, Value>,
    ) -> Result<(), PelagoError> {
        let entry = CdcEntry::create(database, namespace, entity_type, node_id, properties.clone());
        self.write_entry(database, namespace, entry).await
    }

    /// Write an update CDC entry
    pub async fn write_update(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        old_properties: &HashMap<String, Value>,
        new_properties: &HashMap<String, Value>,
    ) -> Result<(), PelagoError> {
        let entry = CdcEntry::update(
            database,
            namespace,
            entity_type,
            node_id,
            old_properties.clone(),
            new_properties.clone(),
        );
        self.write_entry(database, namespace, entry).await
    }

    /// Write a delete CDC entry
    pub async fn write_delete(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        old_properties: &HashMap<String, Value>,
    ) -> Result<(), PelagoError> {
        let entry =
            CdcEntry::delete(database, namespace, entity_type, node_id, old_properties.clone());
        self.write_entry(database, namespace, entry).await
    }

    /// Write a CDC entry to FDB
    async fn write_entry(
        &self,
        database: &str,
        namespace: &str,
        entry: CdcEntry,
    ) -> Result<(), PelagoError> {
        let subspace = Subspace::namespace(database, namespace).cdc();

        // Use timestamp + random suffix as key for ordering
        // Note: In production, this should use FDB versionstamps for true ordering
        let timestamp = entry.timestamp;
        let random_suffix = rand_suffix();

        let key = subspace
            .pack()
            .add_int(timestamp)
            .add_int(random_suffix as i64)
            .build();

        let entry_bytes = encode_cbor(&entry)?;

        self.db.set(key.as_ref(), &entry_bytes).await?;

        Ok(())
    }

    /// Read CDC entries in a time range
    pub async fn read_entries(
        &self,
        database: &str,
        namespace: &str,
        start_timestamp: Option<i64>,
        limit: usize,
    ) -> Result<Vec<CdcEntry>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace).cdc();

        let range_start = match start_timestamp {
            Some(ts) => subspace.pack().add_int(ts).build(),
            None => subspace.prefix().to_vec().into(),
        };
        let range_end = subspace.range_end();

        let results = self
            .db
            .get_range(range_start.as_ref(), range_end.as_ref(), limit)
            .await?;

        let mut entries = Vec::new();
        for (_key, value) in results {
            let entry: CdcEntry = pelago_core::encoding::decode_cbor(&value)?;
            entries.push(entry);
        }

        Ok(entries)
    }
}

fn rand_suffix() -> u32 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    nanos ^ (std::process::id() << 16)
}
