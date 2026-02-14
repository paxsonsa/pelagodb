//! Change Data Capture (CDC) entry emission
//!
//! CDC entries are written with versionstamp keys for zero-contention ordering.
//! The CDC consumer framework is implemented in Phase 2.

use serde::{Deserialize, Serialize};

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
    pub old_values: Option<std::collections::HashMap<String, pelago_core::Value>>,
    /// New property values (for create/update)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_values: Option<std::collections::HashMap<String, pelago_core::Value>>,
}

impl CdcEntry {
    pub fn create(
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        values: std::collections::HashMap<String, pelago_core::Value>,
    ) -> Self {
        Self {
            operation: CdcOperation::Create,
            entity_type: entity_type.to_string(),
            node_id: node_id.to_string(),
            namespace: ns.to_string(),
            database: db.to_string(),
            old_values: None,
            new_values: Some(values),
        }
    }

    pub fn update(
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        old_values: std::collections::HashMap<String, pelago_core::Value>,
        new_values: std::collections::HashMap<String, pelago_core::Value>,
    ) -> Self {
        Self {
            operation: CdcOperation::Update,
            entity_type: entity_type.to_string(),
            node_id: node_id.to_string(),
            namespace: ns.to_string(),
            database: db.to_string(),
            old_values: Some(old_values),
            new_values: Some(new_values),
        }
    }

    pub fn delete(
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        old_values: std::collections::HashMap<String, pelago_core::Value>,
    ) -> Self {
        Self {
            operation: CdcOperation::Delete,
            entity_type: entity_type.to_string(),
            node_id: node_id.to_string(),
            namespace: ns.to_string(),
            database: db.to_string(),
            old_values: Some(old_values),
            new_values: None,
        }
    }
}
