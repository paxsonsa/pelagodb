//! PelagoDB error types with gRPC status mapping
//!
//! All PelagoDB errors are categorized for appropriate gRPC status code mapping:
//! - VALIDATION: Invalid input (INVALID_ARGUMENT)
//! - REFERENTIAL: Entity not found (NOT_FOUND)
//! - CONSISTENCY: Conflicts (ALREADY_EXISTS, ABORTED)
//! - TIMEOUT: Operation timeout (DEADLINE_EXCEEDED)
//! - RESOURCE: Limits exceeded (RESOURCE_EXHAUSTED)
//! - INTERNAL: Server errors (INTERNAL)

use std::collections::HashMap;

/// Main error type for PelagoDB
#[derive(Debug, thiserror::Error)]
pub enum PelagoError {
    // Validation errors
    #[error("Entity type '{entity_type}' not registered")]
    UnregisteredType { entity_type: String },

    #[error("Required property '{field}' missing for type '{entity_type}'")]
    MissingRequired { entity_type: String, field: String },

    #[error("Type mismatch for field '{field}': expected {expected}, got {actual}")]
    TypeMismatch {
        field: String,
        expected: String,
        actual: String,
    },

    #[error("Extra property '{field}' not allowed (extras_policy=reject)")]
    ExtraProperty { field: String },

    #[error("CEL syntax error in '{expression}': {message}")]
    CelSyntax { expression: String, message: String },

    #[error("CEL type error for field '{field}': {message}")]
    CelType { field: String, message: String },

    #[error("Invalid ID format: {value}")]
    InvalidId { value: String },

    #[error("Invalid value for field '{field}': {reason}")]
    InvalidValue { field: String, reason: String },

    #[error("Invalid schema: {message}")]
    SchemaValidation { message: String },

    // Referential errors
    #[error("Source node not found: {entity_type}/{node_id}")]
    SourceNotFound {
        entity_type: String,
        node_id: String,
    },

    #[error("Target node not found: {entity_type}/{node_id}")]
    TargetNotFound {
        entity_type: String,
        node_id: String,
    },

    #[error("Node not found: {entity_type}/{node_id}")]
    NodeNotFound {
        entity_type: String,
        node_id: String,
    },

    #[error("Edge not found: {source_node} -[{label}]-> {target_node}")]
    EdgeNotFound {
        source_node: String,
        target_node: String,
        label: String,
    },

    #[error("Target type mismatch for edge '{edge_type}': expected {expected}, got {actual}")]
    TargetTypeMismatch {
        edge_type: String,
        expected: String,
        actual: String,
    },

    #[error("Undeclared edge type '{edge_type}' for entity '{entity_type}'")]
    UndeclaredEdgeType {
        entity_type: String,
        edge_type: String,
    },

    // Consistency errors
    #[error("Unique constraint violation: {entity_type}.{field} = {value}")]
    UniqueConstraintViolation {
        entity_type: String,
        field: String,
        value: String,
    },

    #[error("Version conflict for {entity_type}/{node_id}")]
    VersionConflict {
        entity_type: String,
        node_id: String,
    },

    #[error("Schema version mismatch: expected {expected}, got {actual}")]
    SchemaMismatch { expected: u32, actual: u32 },

    // Timeout errors
    #[error("Query timeout: {elapsed_ms}ms exceeded {timeout_ms}ms")]
    QueryTimeout { timeout_ms: u64, elapsed_ms: u64 },

    #[error("Traversal timeout at depth {reached_depth}")]
    TraversalTimeout { max_depth: u32, reached_depth: u32 },

    #[error("FDB transaction too old")]
    TransactionTimeout,

    // Resource errors
    #[error("Traversal result limit exceeded: {current_count} >= {max_results}")]
    TraversalLimit {
        max_results: u32,
        current_count: u32,
    },

    // Watch System errors (Phase 4)
    #[error("Subscription limit reached: {current}/{limit}")]
    WatchSubscriptionLimit { limit: usize, current: usize },

    #[error("Query watch limit reached: {current}/{limit}")]
    WatchQueryLimit { limit: usize, current: usize },

    #[error("Resume position expired (oldest available: {oldest_available})")]
    WatchPositionExpired {
        requested: String,
        oldest_available: String,
    },

    #[error("Invalid watch query '{expression}': {error}")]
    WatchInvalidQuery { expression: String, error: String },

    #[error("Watch query too complex: complexity {complexity} exceeds limit {limit}")]
    WatchQueryTooComplex { complexity: u64, limit: u64 },

    #[error("Subscription not found: {subscription_id}")]
    WatchSubscriptionNotFound { subscription_id: String },

    #[error("Requested TTL {requested_secs}s exceeds maximum {max_secs}s")]
    WatchTtlExceeded { requested_secs: u32, max_secs: u32 },

    // Internal errors
    #[error("FDB unavailable: {0}")]
    FdbUnavailable(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl PelagoError {
    /// Get the error code string
    pub fn code(&self) -> &'static str {
        match self {
            PelagoError::UnregisteredType { .. } => "ERR_VALIDATION_UNREGISTERED_TYPE",
            PelagoError::MissingRequired { .. } => "ERR_VALIDATION_MISSING_REQUIRED",
            PelagoError::TypeMismatch { .. } => "ERR_VALIDATION_TYPE_MISMATCH",
            PelagoError::ExtraProperty { .. } => "ERR_VALIDATION_EXTRA_PROPERTY",
            PelagoError::CelSyntax { .. } => "ERR_VALIDATION_CEL_SYNTAX",
            PelagoError::CelType { .. } => "ERR_VALIDATION_CEL_TYPE",
            PelagoError::InvalidId { .. } => "ERR_VALIDATION_INVALID_ID",
            PelagoError::InvalidValue { .. } => "ERR_VALIDATION_INVALID_VALUE",
            PelagoError::SchemaValidation { .. } => "ERR_VALIDATION_SCHEMA",
            PelagoError::SourceNotFound { .. } => "ERR_REF_SOURCE_NOT_FOUND",
            PelagoError::TargetNotFound { .. } => "ERR_REF_TARGET_NOT_FOUND",
            PelagoError::NodeNotFound { .. } => "ERR_REF_NODE_NOT_FOUND",
            PelagoError::EdgeNotFound { .. } => "ERR_REF_EDGE_NOT_FOUND",
            PelagoError::TargetTypeMismatch { .. } => "ERR_REF_TARGET_TYPE_MISMATCH",
            PelagoError::UndeclaredEdgeType { .. } => "ERR_REF_UNDECLARED_EDGE",
            PelagoError::UniqueConstraintViolation { .. } => "ERR_CONSISTENCY_UNIQUE_VIOLATION",
            PelagoError::VersionConflict { .. } => "ERR_CONSISTENCY_VERSION_CONFLICT",
            PelagoError::SchemaMismatch { .. } => "ERR_CONSISTENCY_SCHEMA_MISMATCH",
            PelagoError::QueryTimeout { .. } => "ERR_TIMEOUT_QUERY",
            PelagoError::TraversalTimeout { .. } => "ERR_TIMEOUT_TRAVERSAL",
            PelagoError::TransactionTimeout => "ERR_TIMEOUT_TRANSACTION",
            PelagoError::TraversalLimit { .. } => "ERR_RESOURCE_TRAVERSAL_LIMIT",
            PelagoError::WatchSubscriptionLimit { .. } => "ERR_WATCH_SUBSCRIPTION_LIMIT",
            PelagoError::WatchQueryLimit { .. } => "ERR_WATCH_QUERY_LIMIT",
            PelagoError::WatchPositionExpired { .. } => "ERR_WATCH_POSITION_EXPIRED",
            PelagoError::WatchInvalidQuery { .. } => "ERR_WATCH_INVALID_QUERY",
            PelagoError::WatchQueryTooComplex { .. } => "ERR_WATCH_QUERY_TOO_COMPLEX",
            PelagoError::WatchSubscriptionNotFound { .. } => "ERR_WATCH_SUBSCRIPTION_NOT_FOUND",
            PelagoError::WatchTtlExceeded { .. } => "ERR_WATCH_TTL_EXCEEDED",
            PelagoError::FdbUnavailable(_) => "ERR_INTERNAL_FDB_UNAVAILABLE",
            PelagoError::Internal(_) => "ERR_INTERNAL_UNEXPECTED",
        }
    }

    /// Get the error category
    pub fn category(&self) -> &'static str {
        match self {
            PelagoError::UnregisteredType { .. }
            | PelagoError::MissingRequired { .. }
            | PelagoError::TypeMismatch { .. }
            | PelagoError::ExtraProperty { .. }
            | PelagoError::CelSyntax { .. }
            | PelagoError::CelType { .. }
            | PelagoError::InvalidId { .. }
            | PelagoError::InvalidValue { .. }
            | PelagoError::SchemaValidation { .. } => "VALIDATION",

            PelagoError::SourceNotFound { .. }
            | PelagoError::TargetNotFound { .. }
            | PelagoError::NodeNotFound { .. }
            | PelagoError::EdgeNotFound { .. }
            | PelagoError::TargetTypeMismatch { .. }
            | PelagoError::UndeclaredEdgeType { .. } => "REFERENTIAL",

            PelagoError::UniqueConstraintViolation { .. }
            | PelagoError::VersionConflict { .. }
            | PelagoError::SchemaMismatch { .. } => "CONSISTENCY",

            PelagoError::QueryTimeout { .. }
            | PelagoError::TraversalTimeout { .. }
            | PelagoError::TransactionTimeout => "TIMEOUT",

            PelagoError::TraversalLimit { .. }
            | PelagoError::WatchSubscriptionLimit { .. }
            | PelagoError::WatchQueryLimit { .. } => "RESOURCE",

            PelagoError::WatchPositionExpired { .. } => "WATCH",

            PelagoError::WatchInvalidQuery { .. }
            | PelagoError::WatchQueryTooComplex { .. }
            | PelagoError::WatchTtlExceeded { .. } => "VALIDATION",

            PelagoError::WatchSubscriptionNotFound { .. } => "REFERENTIAL",

            PelagoError::FdbUnavailable(_) | PelagoError::Internal(_) => "INTERNAL",
        }
    }

    /// Get metadata for error details
    pub fn metadata(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        match self {
            PelagoError::UnregisteredType { entity_type } => {
                m.insert("entity_type".into(), entity_type.clone());
            }
            PelagoError::MissingRequired { entity_type, field } => {
                m.insert("entity_type".into(), entity_type.clone());
                m.insert("field".into(), field.clone());
            }
            PelagoError::TypeMismatch {
                field,
                expected,
                actual,
            } => {
                m.insert("field".into(), field.clone());
                m.insert("expected".into(), expected.clone());
                m.insert("actual".into(), actual.clone());
            }
            PelagoError::ExtraProperty { field } => {
                m.insert("field".into(), field.clone());
            }
            PelagoError::InvalidId { value } => {
                m.insert("value".into(), value.clone());
            }
            PelagoError::InvalidValue { field, reason } => {
                m.insert("field".into(), field.clone());
                m.insert("reason".into(), reason.clone());
            }
            PelagoError::NodeNotFound {
                entity_type,
                node_id,
            } => {
                m.insert("entity_type".into(), entity_type.clone());
                m.insert("node_id".into(), node_id.clone());
            }
            PelagoError::SourceNotFound {
                entity_type,
                node_id,
            } => {
                m.insert("entity_type".into(), entity_type.clone());
                m.insert("node_id".into(), node_id.clone());
            }
            PelagoError::TargetNotFound {
                entity_type,
                node_id,
            } => {
                m.insert("entity_type".into(), entity_type.clone());
                m.insert("node_id".into(), node_id.clone());
            }
            PelagoError::UniqueConstraintViolation {
                entity_type,
                field,
                value,
            } => {
                m.insert("entity_type".into(), entity_type.clone());
                m.insert("field".into(), field.clone());
                m.insert("value".into(), value.clone());
            }
            PelagoError::QueryTimeout {
                timeout_ms,
                elapsed_ms,
            } => {
                m.insert("timeout_ms".into(), timeout_ms.to_string());
                m.insert("elapsed_ms".into(), elapsed_ms.to_string());
            }
            PelagoError::TraversalLimit {
                max_results,
                current_count,
            } => {
                m.insert("max_results".into(), max_results.to_string());
                m.insert("current_count".into(), current_count.to_string());
            }
            PelagoError::WatchSubscriptionLimit { limit, current } => {
                m.insert("limit".into(), limit.to_string());
                m.insert("current".into(), current.to_string());
            }
            PelagoError::WatchQueryLimit { limit, current } => {
                m.insert("limit".into(), limit.to_string());
                m.insert("current".into(), current.to_string());
            }
            PelagoError::WatchPositionExpired {
                requested,
                oldest_available,
            } => {
                m.insert("requested".into(), requested.clone());
                m.insert("oldest_available".into(), oldest_available.clone());
            }
            PelagoError::WatchSubscriptionNotFound { subscription_id } => {
                m.insert("subscription_id".into(), subscription_id.clone());
            }
            _ => {}
        }
        m
    }
}

// gRPC status code mapping implemented in pelago-api crate
