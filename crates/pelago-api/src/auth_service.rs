//! AuthService gRPC handler.

use crate::authz::{authorize, principal_from_request};
use pelago_proto::{
    auth_service_server::AuthService, AuthenticateRequest, AuthenticateResponse,
    CheckPermissionRequest, CheckPermissionResponse, CreatePolicyRequest, CreatePolicyResponse,
    DeletePolicyRequest, DeletePolicyResponse, GetPolicyRequest, GetPolicyResponse,
    ListPoliciesRequest, ListPoliciesResponse, Permission, Policy, Principal, RefreshTokenRequest,
    RefreshTokenResponse, ValidateTokenRequest, ValidateTokenResponse,
};
use pelago_storage::{
    append_audit_record, check_permission, delete_policy, get_policy, list_policies, upsert_policy,
    AuditRecord, AuthPolicy, PelagoDb, PolicyPermission,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct TokenSession {
    principal_id: String,
    principal_type: String,
    roles: Vec<String>,
    expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct AuthPrincipal {
    pub principal_id: String,
    pub principal_type: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AuthRuntime {
    sessions: Arc<RwLock<HashMap<String, TokenSession>>>,
    refresh_tokens: Arc<RwLock<HashMap<String, String>>>,
    api_keys: Arc<HashMap<String, String>>,
}

impl AuthRuntime {
    pub fn new() -> Self {
        let mut api_keys = HashMap::new();
        if let Ok(raw) = std::env::var("PELAGO_API_KEYS") {
            for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if let Some((key, principal)) = entry.split_once(':') {
                    api_keys.insert(key.trim().to_string(), principal.trim().to_string());
                } else {
                    api_keys.insert(entry.to_string(), "api-key-user".to_string());
                }
            }
        }
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(api_keys),
        }
    }

    async fn validate_bearer_token(&self, token: &str) -> Option<TokenSession> {
        let sessions = self.sessions.read().await;
        sessions
            .get(token)
            .cloned()
            .filter(|s| s.expires_at > now_secs())
    }

    pub fn validate_bearer_token_sync(&self, token: &str) -> Option<AuthPrincipal> {
        let sessions = self.sessions.try_read().ok()?;
        let session = sessions.get(token)?;
        if session.expires_at <= now_secs() {
            return None;
        }
        Some(AuthPrincipal {
            principal_id: session.principal_id.clone(),
            principal_type: session.principal_type.clone(),
            roles: session.roles.clone(),
        })
    }

    pub fn principal_for_api_key(&self, api_key: &str) -> Option<AuthPrincipal> {
        self.api_keys.get(api_key).map(|principal| AuthPrincipal {
            principal_id: principal.clone(),
            principal_type: "api_key".to_string(),
            roles: vec!["admin".to_string()],
        })
    }

    async fn issue_tokens(
        &self,
        principal_id: String,
        principal_type: String,
        roles: Vec<String>,
    ) -> (String, String, i64) {
        let access = Uuid::now_v7().to_string();
        let refresh = Uuid::now_v7().to_string();
        let expires_at = now_secs() + 3600;

        let session = TokenSession {
            principal_id,
            principal_type,
            roles,
            expires_at,
        };
        self.sessions.write().await.insert(access.clone(), session);
        self.refresh_tokens
            .write()
            .await
            .insert(refresh.clone(), access.clone());
        (access, refresh, expires_at)
    }
}

pub struct AuthServiceImpl {
    db: PelagoDb,
    runtime: AuthRuntime,
}

impl AuthServiceImpl {
    pub fn new(db: PelagoDb, runtime: AuthRuntime) -> Self {
        Self { db, runtime }
    }

    pub fn runtime(&self) -> AuthRuntime {
        self.runtime.clone()
    }
}

#[tonic::async_trait]
impl AuthService for AuthServiceImpl {
    async fn authenticate(
        &self,
        request: Request<AuthenticateRequest>,
    ) -> Result<Response<AuthenticateResponse>, Status> {
        let req = request.into_inner();
        let principal_id = if !req.api_key.is_empty() {
            self.runtime
                .api_keys
                .get(&req.api_key)
                .cloned()
                .ok_or_else(|| Status::unauthenticated("invalid api key"))?
        } else if !req.username.is_empty() {
            req.username
        } else {
            return Err(Status::invalid_argument("missing credentials"));
        };

        let (access_token, refresh_token, expires_at) = self
            .runtime
            .issue_tokens(
                principal_id.clone(),
                "user".to_string(),
                vec!["admin".to_string()],
            )
            .await;

        let _ = append_audit_record(
            &self.db,
            AuditRecord {
                event_id: String::new(),
                timestamp: 0,
                principal_id: principal_id.clone(),
                action: "auth.authenticate".to_string(),
                resource: "*".to_string(),
                allowed: true,
                reason: "authenticated".to_string(),
                metadata: HashMap::new(),
            },
        )
        .await;

        Ok(Response::new(AuthenticateResponse {
            access_token,
            refresh_token,
            expires_at,
            principal: Some(Principal {
                principal_id,
                principal_type: "user".to_string(),
                roles: vec!["admin".to_string()],
            }),
        }))
    }

    async fn refresh_token(
        &self,
        request: Request<RefreshTokenRequest>,
    ) -> Result<Response<RefreshTokenResponse>, Status> {
        let req = request.into_inner();
        let access = self
            .runtime
            .refresh_tokens
            .read()
            .await
            .get(&req.refresh_token)
            .cloned()
            .ok_or_else(|| Status::unauthenticated("invalid refresh token"))?;

        let session = self
            .runtime
            .sessions
            .read()
            .await
            .get(&access)
            .cloned()
            .ok_or_else(|| Status::unauthenticated("refresh session not found"))?;

        let (new_access, _refresh, expires_at) = self
            .runtime
            .issue_tokens(session.principal_id, session.principal_type, session.roles)
            .await;
        Ok(Response::new(RefreshTokenResponse {
            access_token: new_access,
            expires_at,
        }))
    }

    async fn validate_token(
        &self,
        request: Request<ValidateTokenRequest>,
    ) -> Result<Response<ValidateTokenResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .runtime
            .validate_bearer_token(&req.token)
            .await
            .ok_or_else(|| Status::unauthenticated("invalid token"))?;
        Ok(Response::new(ValidateTokenResponse {
            valid: true,
            principal: Some(Principal {
                principal_id: session.principal_id,
                principal_type: session.principal_type,
                roles: session.roles,
            }),
            expires_at: session.expires_at,
        }))
    }

    async fn create_policy(
        &self,
        request: Request<CreatePolicyRequest>,
    ) -> Result<Response<CreatePolicyResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "auth.policy.write",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let policy = req
            .policy
            .ok_or_else(|| Status::invalid_argument("missing policy"))?;
        let model = proto_policy_to_model(policy)?;
        upsert_policy(&self.db, &model)
            .await
            .map_err(|e| Status::internal(format!("create_policy failed: {}", e)))?;
        let _ = append_audit_record(
            &self.db,
            AuditRecord {
                event_id: String::new(),
                timestamp: 0,
                principal_id: principal
                    .as_ref()
                    .map(|p| p.principal_id.clone())
                    .unwrap_or_else(|| "system".to_string()),
                action: "auth.policy.create".to_string(),
                resource: model.policy_id.clone(),
                allowed: true,
                reason: "policy created".to_string(),
                metadata: HashMap::new(),
            },
        )
        .await;
        Ok(Response::new(CreatePolicyResponse {
            policy: Some(model_policy_to_proto(&model)),
        }))
    }

    async fn get_policy(
        &self,
        request: Request<GetPolicyRequest>,
    ) -> Result<Response<GetPolicyResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "auth.policy.read",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let policy = get_policy(&self.db, &req.policy_id)
            .await
            .map_err(|e| Status::internal(format!("get_policy failed: {}", e)))?
            .ok_or_else(|| Status::not_found("policy not found"))?;
        Ok(Response::new(GetPolicyResponse {
            policy: Some(model_policy_to_proto(&policy)),
        }))
    }

    async fn list_policies(
        &self,
        request: Request<ListPoliciesRequest>,
    ) -> Result<Response<ListPoliciesResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "auth.policy.read",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let policies = list_policies(
            &self.db,
            if req.principal_id.is_empty() {
                None
            } else {
                Some(req.principal_id.as_str())
            },
        )
        .await
        .map_err(|e| Status::internal(format!("list_policies failed: {}", e)))?;

        Ok(Response::new(ListPoliciesResponse {
            policies: policies.iter().map(model_policy_to_proto).collect(),
        }))
    }

    async fn delete_policy(
        &self,
        request: Request<DeletePolicyRequest>,
    ) -> Result<Response<DeletePolicyResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "auth.policy.write",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let deleted = delete_policy(&self.db, &req.policy_id)
            .await
            .map_err(|e| Status::internal(format!("delete_policy failed: {}", e)))?;
        let _ = append_audit_record(
            &self.db,
            AuditRecord {
                event_id: String::new(),
                timestamp: 0,
                principal_id: principal
                    .as_ref()
                    .map(|p| p.principal_id.clone())
                    .unwrap_or_else(|| "system".to_string()),
                action: "auth.policy.delete".to_string(),
                resource: req.policy_id.clone(),
                allowed: deleted,
                reason: if deleted {
                    "policy deleted".to_string()
                } else {
                    "policy not found".to_string()
                },
                metadata: HashMap::new(),
            },
        )
        .await;
        Ok(Response::new(DeletePolicyResponse { deleted }))
    }

    async fn check_permission(
        &self,
        request: Request<CheckPermissionRequest>,
    ) -> Result<Response<CheckPermissionResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "auth.policy.check",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let allowed = check_permission(
            &self.db,
            &req.principal_id,
            &req.action,
            &req.database,
            &req.namespace,
            &req.entity_type,
        )
        .await
        .map_err(|e| Status::internal(format!("check_permission failed: {}", e)))?;

        let _ = append_audit_record(
            &self.db,
            AuditRecord {
                event_id: String::new(),
                timestamp: 0,
                principal_id: req.principal_id.clone(),
                action: req.action.clone(),
                resource: format!("{}/{}/{}", req.database, req.namespace, req.entity_type),
                allowed,
                reason: if allowed {
                    "allowed".to_string()
                } else {
                    "denied".to_string()
                },
                metadata: HashMap::new(),
            },
        )
        .await;

        Ok(Response::new(CheckPermissionResponse {
            allowed,
            reason: if allowed {
                "policy match".to_string()
            } else {
                "no matching policy".to_string()
            },
        }))
    }
}

fn proto_policy_to_model(policy: Policy) -> Result<AuthPolicy, Status> {
    if policy.policy_id.is_empty() {
        return Err(Status::invalid_argument("policy_id cannot be empty"));
    }
    if policy.principal_id.is_empty() {
        return Err(Status::invalid_argument("principal_id cannot be empty"));
    }

    Ok(AuthPolicy {
        policy_id: policy.policy_id,
        principal_id: policy.principal_id,
        permissions: policy
            .permissions
            .into_iter()
            .map(|p| PolicyPermission {
                database: p.database,
                namespace: p.namespace,
                entity_type: p.entity_type,
                actions: p.actions,
            })
            .collect(),
        created_at: if policy.created_at == 0 {
            now_secs()
        } else {
            policy.created_at
        },
    })
}

fn model_policy_to_proto(policy: &AuthPolicy) -> Policy {
    Policy {
        policy_id: policy.policy_id.clone(),
        principal_id: policy.principal_id.clone(),
        permissions: policy
            .permissions
            .iter()
            .map(|p| Permission {
                database: p.database.clone(),
                namespace: p.namespace.clone(),
                entity_type: p.entity_type.clone(),
                actions: p.actions.clone(),
            })
            .collect(),
        created_at: policy.created_at,
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proto_policy_validation() {
        let err = proto_policy_to_model(Policy {
            policy_id: String::new(),
            principal_id: "p1".to_string(),
            permissions: Vec::new(),
            created_at: 0,
        })
        .expect_err("missing policy_id should fail");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        let err = proto_policy_to_model(Policy {
            policy_id: "pol1".to_string(),
            principal_id: String::new(),
            permissions: Vec::new(),
            created_at: 0,
        })
        .expect_err("missing principal_id should fail");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn test_policy_model_proto_roundtrip_shape() {
        let policy = Policy {
            policy_id: "pol1".to_string(),
            principal_id: "principal-a".to_string(),
            permissions: vec![Permission {
                database: "db1".to_string(),
                namespace: "ns1".to_string(),
                entity_type: "Person".to_string(),
                actions: vec!["read".to_string(), "write".to_string()],
            }],
            created_at: 123,
        };

        let model = proto_policy_to_model(policy).expect("policy should convert");
        let back = model_policy_to_proto(&model);
        assert_eq!(back.policy_id, "pol1");
        assert_eq!(back.principal_id, "principal-a");
        assert_eq!(back.permissions.len(), 1);
        assert_eq!(back.permissions[0].entity_type, "Person");
        assert_eq!(back.permissions[0].actions, vec!["read", "write"]);
        assert_eq!(back.created_at, 123);
    }
}
