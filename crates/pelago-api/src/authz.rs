use crate::auth_service::AuthPrincipal;
use pelago_storage::{append_audit_record, check_permission, AuditRecord, PelagoDb};
use tonic::{Request, Status};

pub fn principal_from_request<T>(request: &Request<T>) -> Option<AuthPrincipal> {
    request.extensions().get::<AuthPrincipal>().cloned()
}

pub async fn authorize(
    db: &PelagoDb,
    principal: Option<&AuthPrincipal>,
    action: &str,
    database: &str,
    namespace: &str,
    entity_type: &str,
) -> Result<(), Status> {
    // Development mode: no authenticated principal in request context.
    let Some(principal) = principal else {
        return Ok(());
    };

    let resource = format!("{}/{}/{}", database, namespace, entity_type);
    let allowed = if has_admin_role(&principal.roles) {
        true
    } else if has_namespace_admin_role(&principal.roles, database, namespace) {
        true
    } else if has_read_only_role(&principal.roles) {
        is_read_action(action)
    } else {
        check_permission(
            db,
            &principal.principal_id,
            action,
            database,
            namespace,
            entity_type,
        )
        .await
        .map_err(|e| Status::internal(format!("authorization check failed: {}", e)))?
    };

    let reason = if allowed {
        "allowed"
    } else {
        "permission denied"
    };
    let _ = append_audit_record(
        db,
        AuditRecord {
            event_id: String::new(),
            timestamp: 0,
            principal_id: principal.principal_id.clone(),
            action: if allowed {
                "authz.allowed".to_string()
            } else {
                "authz.denied".to_string()
            },
            resource,
            allowed,
            reason: reason.to_string(),
            metadata: std::collections::HashMap::from([
                ("requested_action".to_string(), action.to_string()),
                (
                    "principal_type".to_string(),
                    principal.principal_type.clone(),
                ),
            ]),
        },
    )
    .await;

    if allowed {
        Ok(())
    } else {
        Err(Status::permission_denied(format!(
            "principal '{}' is not authorized for '{}'",
            principal.principal_id, action
        )))
    }
}

fn has_admin_role(roles: &[String]) -> bool {
    roles.iter().any(|r| r.eq_ignore_ascii_case("admin"))
}

fn has_read_only_role(roles: &[String]) -> bool {
    roles.iter().any(|r| r.eq_ignore_ascii_case("read-only"))
}

fn has_namespace_admin_role(roles: &[String], database: &str, namespace: &str) -> bool {
    for role in roles {
        if role.eq_ignore_ascii_case("namespace-admin") {
            return true;
        }
        if let Some(rest) = role.strip_prefix("namespace-admin:") {
            let mut parts = rest.split(':');
            let db = parts.next().unwrap_or_default();
            let ns = parts.next().unwrap_or_default();
            if db == database && ns == namespace {
                return true;
            }
        }
    }
    false
}

fn is_read_action(action: &str) -> bool {
    action.starts_with("read.")
        || action.starts_with("query.")
        || action.starts_with("watch.")
        || action == "schema.read"
        || action == "node.read"
        || action == "edge.read"
        || action == "admin.read"
        || action == "auth.policy.read"
        || action == "auth.policy.check"
        || action == "replication.pull"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_admin_scoped_role() {
        let roles = vec!["namespace-admin:db1:ns1".to_string()];
        assert!(has_namespace_admin_role(&roles, "db1", "ns1"));
        assert!(!has_namespace_admin_role(&roles, "db1", "ns2"));
    }

    #[test]
    fn test_read_only_role_allows_read_actions_only() {
        let roles = vec!["read-only".to_string()];
        assert!(has_read_only_role(&roles));
        assert!(is_read_action("query.find"));
        assert!(is_read_action("watch.subscribe"));
        assert!(!is_read_action("node.write"));
        assert!(!is_read_action("admin.drop_index"));
    }

    #[test]
    fn test_admin_role_detected_case_insensitive() {
        let roles = vec!["AdMiN".to_string()];
        assert!(has_admin_role(&roles));
    }
}
