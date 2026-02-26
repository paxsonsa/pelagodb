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

    #[error(
        "Namespace schema ownership conflict for {database}/{namespace}: owner_site_id={owner_site_id}, actor_site_id={actor_site_id}"
    )]
    NamespaceSchemaOwnershipConflict {
        database: String,
        namespace: String,
        owner_site_id: String,
        actor_site_id: String,
    },

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

    #[error(
        "Mutation scope too large for inline execution ({operation}): keys={estimated_mutated_keys}, bytes={estimated_write_bytes}, edges={estimated_cascade_edges}, recommended_mode={recommended_mode}"
    )]
    MutationScopeTooLarge {
        operation: String,
        estimated_mutated_keys: usize,
        estimated_write_bytes: usize,
        estimated_cascade_edges: usize,
        recommended_mode: String,
    },

    #[error("Transaction conflict retry budget exhausted ({operation}) after {attempts} attempts")]
    TxConflictRetryExhausted { operation: String, attempts: u32 },

    #[error("Transaction wall time budget exceeded ({operation}): {elapsed_ms}ms > {budget_ms}ms")]
    TxWallBudgetExceeded {
        operation: String,
        elapsed_ms: u64,
        budget_ms: u64,
    },

    #[error(
        "Strict snapshot budget exceeded ({operation}): scanned_keys={scanned_keys}, result_bytes={result_bytes}, elapsed_ms={elapsed_ms}, budget_ms={budget_ms}"
    )]
    SnapshotBudgetExceeded {
        operation: String,
        scanned_keys: usize,
        result_bytes: usize,
        elapsed_ms: u64,
        budget_ms: u64,
    },

    #[error("Strict snapshot expired ({operation})")]
    SnapshotExpired { operation: String },

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
            PelagoError::NamespaceSchemaOwnershipConflict { .. } => {
                "ERR_CONSISTENCY_NAMESPACE_SCHEMA_OWNERSHIP_CONFLICT"
            }
            PelagoError::QueryTimeout { .. } => "ERR_TIMEOUT_QUERY",
            PelagoError::TraversalTimeout { .. } => "ERR_TIMEOUT_TRAVERSAL",
            PelagoError::TransactionTimeout => "ERR_TIMEOUT_TRANSACTION",
            PelagoError::TraversalLimit { .. } => "ERR_RESOURCE_TRAVERSAL_LIMIT",
            PelagoError::MutationScopeTooLarge { .. } => "ERR_RESOURCE_MUTATION_SCOPE_TOO_LARGE",
            PelagoError::TxConflictRetryExhausted { .. } => {
                "ERR_CONSISTENCY_TX_CONFLICT_RETRY_EXHAUSTED"
            }
            PelagoError::TxWallBudgetExceeded { .. } => "ERR_TIMEOUT_TX_WALL_BUDGET_EXCEEDED",
            PelagoError::SnapshotBudgetExceeded { .. } => {
                "ERR_CONSISTENCY_SNAPSHOT_BUDGET_EXCEEDED"
            }
            PelagoError::SnapshotExpired { .. } => "ERR_CONSISTENCY_SNAPSHOT_EXPIRED",
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
            | PelagoError::SchemaMismatch { .. }
            | PelagoError::NamespaceSchemaOwnershipConflict { .. }
            | PelagoError::TxConflictRetryExhausted { .. }
            | PelagoError::SnapshotBudgetExceeded { .. }
            | PelagoError::SnapshotExpired { .. } => "CONSISTENCY",

            PelagoError::QueryTimeout { .. }
            | PelagoError::TraversalTimeout { .. }
            | PelagoError::TransactionTimeout
            | PelagoError::TxWallBudgetExceeded { .. } => "TIMEOUT",

            PelagoError::TraversalLimit { .. } | PelagoError::MutationScopeTooLarge { .. } => {
                "RESOURCE"
            }

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
            PelagoError::MutationScopeTooLarge {
                operation,
                estimated_mutated_keys,
                estimated_write_bytes,
                estimated_cascade_edges,
                recommended_mode,
            } => {
                m.insert("operation".into(), operation.clone());
                m.insert(
                    "estimated_mutated_keys".into(),
                    estimated_mutated_keys.to_string(),
                );
                m.insert(
                    "estimated_write_bytes".into(),
                    estimated_write_bytes.to_string(),
                );
                m.insert(
                    "estimated_cascade_edges".into(),
                    estimated_cascade_edges.to_string(),
                );
                m.insert("recommended_mode".into(), recommended_mode.clone());
            }
            PelagoError::TxConflictRetryExhausted {
                operation,
                attempts,
            } => {
                m.insert("operation".into(), operation.clone());
                m.insert("attempts".into(), attempts.to_string());
            }
            PelagoError::TxWallBudgetExceeded {
                operation,
                elapsed_ms,
                budget_ms,
            } => {
                m.insert("operation".into(), operation.clone());
                m.insert("elapsed_ms".into(), elapsed_ms.to_string());
                m.insert("budget_ms".into(), budget_ms.to_string());
            }
            PelagoError::SnapshotBudgetExceeded {
                operation,
                scanned_keys,
                result_bytes,
                elapsed_ms,
                budget_ms,
            } => {
                m.insert("operation".into(), operation.clone());
                m.insert("scanned_keys".into(), scanned_keys.to_string());
                m.insert("result_bytes".into(), result_bytes.to_string());
                m.insert("elapsed_ms".into(), elapsed_ms.to_string());
                m.insert("budget_ms".into(), budget_ms.to_string());
            }
            PelagoError::SnapshotExpired { operation } => {
                m.insert("operation".into(), operation.clone());
            }
            PelagoError::NamespaceSchemaOwnershipConflict {
                database,
                namespace,
                owner_site_id,
                actor_site_id,
            } => {
                m.insert("database".into(), database.clone());
                m.insert("namespace".into(), namespace.clone());
                m.insert("owner_site_id".into(), owner_site_id.clone());
                m.insert("actor_site_id".into(), actor_site_id.clone());
            }
            _ => {}
        }
        m
    }
}

// gRPC status code mapping implemented in pelago-api crate
