use crate::auth_service::AuthPrincipal;
use crate::authz::authorize;
use pelago_proto::RequestContext;
use pelago_storage::PelagoDb;
use tonic::Status;

pub fn require_field<T>(value: Option<T>, field: &'static str) -> Result<T, Status> {
    value.ok_or_else(|| Status::invalid_argument(format!("missing {}", field)))
}

pub async fn authorize_with_context(
    db: &PelagoDb,
    principal: Option<&AuthPrincipal>,
    action: &str,
    context: &RequestContext,
    entity_type: &str,
) -> Result<(), Status> {
    authorize(
        db,
        principal,
        action,
        &context.database,
        &context.namespace,
        entity_type,
    )
    .await
}
