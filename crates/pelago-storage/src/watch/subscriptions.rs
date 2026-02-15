//! Watch subscription types
//!
//! Three subscription kinds, each with matching logic against CDC operations:
//! - PointWatch: O(1) lookup via node/edge indexes
//! - QueryWatch: CEL/PQL filter evaluation with enter/update/exit tracking
//! - NamespaceWatch: entity type + operation type filtering

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::cdc::{CdcOperation, Versionstamp};
use pelago_core::Value;
use pelago_proto::WatchEvent;

pub type SubscriptionId = String;

// ─── Point Watch ────────────────────────────────────────────────────────

/// Watches specific nodes and/or edges by identity.
/// Dispatched via O(1) index lookup in the dispatcher.
pub struct PointWatchSubscription {
    pub subscription_id: SubscriptionId,
    pub database: String,
    pub namespace: String,

    /// Nodes to watch: (entity_type, node_id)
    pub watched_nodes: HashSet<(String, String)>,
    /// Edges to watch: (source_type, source_id, label)
    pub watched_edges: HashSet<(String, String, String)>,
    /// Property restrictions: (entity_type, node_id) → property names.
    /// If set, only updates to these properties trigger events.
    pub property_filters: HashMap<(String, String), HashSet<String>>,

    pub sender: mpsc::Sender<WatchEvent>,
    pub last_position: Versionstamp,
    pub created_at: Instant,
    pub expires_at: Instant,
}

impl PointWatchSubscription {
    /// Check if this CDC operation matches this subscription.
    pub fn matches(&self, op: &CdcOperation) -> bool {
        match op {
            CdcOperation::NodeCreate {
                entity_type,
                node_id,
                ..
            }
            | CdcOperation::NodeUpdate {
                entity_type,
                node_id,
                ..
            }
            | CdcOperation::NodeDelete {
                entity_type,
                node_id,
                ..
            } => {
                let key = (entity_type.clone(), node_id.clone());
                if !self.watched_nodes.contains(&key) {
                    return false;
                }
                // For updates, optionally filter by changed properties
                if let CdcOperation::NodeUpdate {
                    changed_properties, ..
                } = op
                {
                    if let Some(props) = self.property_filters.get(&key) {
                        return changed_properties.keys().any(|k| props.contains(k));
                    }
                }
                true
            }

            CdcOperation::EdgeCreate {
                source_type,
                source_id,
                edge_type,
                ..
            }
            | CdcOperation::EdgeDelete {
                source_type,
                source_id,
                edge_type,
                ..
            } => self.watched_edges.contains(&(
                source_type.clone(),
                source_id.clone(),
                edge_type.clone(),
            )),

            CdcOperation::SchemaRegister { .. } => false,
        }
    }
}

// ─── Query Watch ────────────────────────────────────────────────────────

/// Watches nodes matching a CEL or PQL filter expression.
/// Emits Enter/Update/Exit events as nodes transition in/out of the result set.
pub struct QueryWatchSubscription {
    pub subscription_id: SubscriptionId,
    pub database: String,
    pub namespace: String,
    pub entity_type: String,

    /// Human-readable expression (for listing/debugging)
    pub expression: String,
    /// Compiled filter: returns true if properties match the query.
    /// Injected by the API layer (CEL or PQL compilation).
    pub filter: Arc<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>,

    /// Track which nodes currently match (for enter/exit detection)
    pub matching_nodes: HashSet<String>,

    pub sender: mpsc::Sender<WatchEvent>,
    pub include_data: bool,
    pub last_position: Versionstamp,
    pub created_at: Instant,
    pub expires_at: Instant,
}

// ─── Namespace Watch ────────────────────────────────────────────────────

/// Watches all changes within a namespace, optionally filtered by
/// entity type and/or operation type.
pub struct NamespaceWatchSubscription {
    pub subscription_id: SubscriptionId,
    pub database: String,
    pub namespace: String,

    /// If non-empty, only events for these entity types pass
    pub entity_type_filter: HashSet<String>,
    /// If non-empty, only these operation types pass
    pub operation_type_filter: HashSet<i32>,

    pub sender: mpsc::Sender<WatchEvent>,
    pub last_position: Versionstamp,
    pub created_at: Instant,
    pub expires_at: Instant,
}

impl NamespaceWatchSubscription {
    pub fn matches(&self, op: &CdcOperation) -> bool {
        // Check operation type filter
        if !self.operation_type_filter.is_empty() {
            let op_type = operation_type_of(op);
            if !self.operation_type_filter.contains(&op_type) {
                return false;
            }
        }
        // Check entity type filter
        if !self.entity_type_filter.is_empty() {
            let et = entity_type_of(op);
            if !self.entity_type_filter.contains(&et) {
                return false;
            }
        }
        true
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────

/// Map a CDC operation to its OperationType proto i32 value.
pub fn operation_type_of(op: &CdcOperation) -> i32 {
    use pelago_proto::OperationType;
    match op {
        CdcOperation::NodeCreate { .. } => OperationType::NodeCreate as i32,
        CdcOperation::NodeUpdate { .. } => OperationType::NodeUpdate as i32,
        CdcOperation::NodeDelete { .. } => OperationType::NodeDelete as i32,
        CdcOperation::EdgeCreate { .. } => OperationType::EdgeCreate as i32,
        CdcOperation::EdgeDelete { .. } => OperationType::EdgeDelete as i32,
        CdcOperation::SchemaRegister { .. } => OperationType::SchemaRegister as i32,
    }
}

/// Extract the primary entity type from a CDC operation.
pub fn entity_type_of(op: &CdcOperation) -> String {
    match op {
        CdcOperation::NodeCreate { entity_type, .. }
        | CdcOperation::NodeUpdate { entity_type, .. }
        | CdcOperation::NodeDelete { entity_type, .. } => entity_type.clone(),
        CdcOperation::EdgeCreate { source_type, .. }
        | CdcOperation::EdgeDelete { source_type, .. } => source_type.clone(),
        CdcOperation::SchemaRegister { entity_type, .. } => entity_type.clone(),
    }
}

/// Extract the node_id from a node-related CDC operation (None for edge/schema ops).
pub fn node_id_of(op: &CdcOperation) -> Option<&str> {
    match op {
        CdcOperation::NodeCreate { node_id, .. }
        | CdcOperation::NodeUpdate { node_id, .. }
        | CdcOperation::NodeDelete { node_id, .. } => Some(node_id),
        _ => None,
    }
}

/// Convert a core Value to a proto Value.
pub fn core_value_to_proto(v: &Value) -> pelago_proto::Value {
    use pelago_proto::value::Kind;
    pelago_proto::Value {
        kind: Some(match v {
            Value::String(s) => Kind::StringValue(s.clone()),
            Value::Int(i) => Kind::IntValue(*i),
            Value::Float(f) => Kind::FloatValue(*f),
            Value::Bool(b) => Kind::BoolValue(*b),
            Value::Timestamp(t) => Kind::TimestampValue(*t),
            Value::Bytes(b) => Kind::BytesValue(b.clone()),
            Value::Null => Kind::NullValue(true),
        }),
    }
}

/// Convert a core property map to proto property map.
pub fn core_props_to_proto(props: &HashMap<String, Value>) -> HashMap<String, pelago_proto::Value> {
    props
        .iter()
        .map(|(k, v)| (k.clone(), core_value_to_proto(v)))
        .collect()
}
