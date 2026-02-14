//! Error mapping from PelagoError to tonic::Status

use pelago_core::PelagoError;
use tonic::{Code, Status};

/// Convert a PelagoError to a tonic::Status
pub fn to_status(err: PelagoError) -> Status {
    let code = match &err {
        // Validation errors -> INVALID_ARGUMENT
        PelagoError::UnregisteredType { .. }
        | PelagoError::MissingRequired { .. }
        | PelagoError::TypeMismatch { .. }
        | PelagoError::ExtraProperty { .. }
        | PelagoError::CelSyntax { .. }
        | PelagoError::CelType { .. }
        | PelagoError::InvalidId { .. }
        | PelagoError::InvalidValue { .. }
        | PelagoError::SchemaValidation { .. } => Code::InvalidArgument,

        // Referential errors -> NOT_FOUND
        PelagoError::SourceNotFound { .. }
        | PelagoError::TargetNotFound { .. }
        | PelagoError::NodeNotFound { .. }
        | PelagoError::EdgeNotFound { .. }
        | PelagoError::UndeclaredEdgeType { .. } => Code::NotFound,

        // Type mismatch on target -> FAILED_PRECONDITION
        PelagoError::TargetTypeMismatch { .. } => Code::FailedPrecondition,

        // Consistency errors
        PelagoError::UniqueConstraintViolation { .. } => Code::AlreadyExists,
        PelagoError::VersionConflict { .. } | PelagoError::SchemaMismatch { .. } => Code::Aborted,

        // Timeout errors -> DEADLINE_EXCEEDED
        PelagoError::QueryTimeout { .. }
        | PelagoError::TraversalTimeout { .. }
        | PelagoError::TransactionTimeout => Code::DeadlineExceeded,

        // Resource errors -> RESOURCE_EXHAUSTED
        PelagoError::TraversalLimit { .. } => Code::ResourceExhausted,

        // Internal errors -> INTERNAL / UNAVAILABLE
        PelagoError::FdbUnavailable(_) => Code::Unavailable,
        PelagoError::Internal(_) => Code::Internal,
    };

    Status::new(code, err.to_string())
}

/// Extension trait to convert Result<T, PelagoError> to Result<T, Status>
pub trait IntoStatus<T> {
    fn into_status(self) -> Result<T, Status>;
}

impl<T> IntoStatus<T> for Result<T, PelagoError> {
    fn into_status(self) -> Result<T, Status> {
        self.map_err(to_status)
    }
}
