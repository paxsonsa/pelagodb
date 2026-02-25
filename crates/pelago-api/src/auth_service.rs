//! AuthService gRPC handler.

use crate::authz::principal_from_request;
use crate::service_common::{authorize_with_context, require_field};
use pelago_proto::{
    auth_service_server::AuthService, AuthenticateRequest, AuthenticateResponse,
    CheckPermissionRequest, CheckPermissionResponse, CreatePolicyRequest, CreatePolicyResponse,
    DeletePolicyRequest, DeletePolicyResponse, GetPolicyRequest, GetPolicyResponse,
    IntrospectTokenRequest, IntrospectTokenResponse, IssueServiceTokenRequest,
    IssueServiceTokenResponse, ListPoliciesRequest, ListPoliciesResponse, Permission, Policy,
    Principal, RefreshTokenRequest, RefreshTokenResponse, RevokeTokenRequest, RevokeTokenResponse,
    ValidateTokenRequest, ValidateTokenResponse,
};
use pelago_storage::{
    append_audit_record, check_permission, delete_policy, get_policy, get_token_session_by_access,
    get_token_session_by_refresh, list_policies, revoke_access_token, revoke_refresh_token,
    upsert_policy, upsert_token_session, AuditRecord, AuthPolicy, AuthTokenSession, PelagoDb,
    PolicyPermission,
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

#[derive(Clone, Default)]
pub struct AuthRuntime {
    sessions: Arc<RwLock<HashMap<String, TokenSession>>>,
    refresh_tokens: Arc<RwLock<HashMap<String, String>>>,
    api_keys: Arc<HashMap<String, String>>,
    mtls_subjects: Arc<HashMap<String, String>>,
    mtls_fingerprints: Arc<HashMap<String, String>>,
    mtls_enabled: bool,
    mtls_default_role: String,
    db: Option<PelagoDb>,
    default_ttl_secs: i64,
    max_ttl_secs: i64,
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
        let mut mtls_subjects = HashMap::new();
        if let Ok(raw) = std::env::var("PELAGO_MTLS_SUBJECTS") {
            for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if let Some((subject, principal)) = entry.split_once('=') {
                    mtls_subjects.insert(subject.trim().to_string(), principal.trim().to_string());
                } else {
                    mtls_subjects.insert(entry.to_string(), entry.to_string());
                }
            }
        }
        let mut mtls_fingerprints = HashMap::new();
        if let Ok(raw) = std::env::var("PELAGO_MTLS_FINGERPRINTS") {
            for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if let Some((fingerprint, principal)) = entry.split_once('=') {
                    mtls_fingerprints.insert(
                        normalize_mtls_fingerprint(fingerprint),
                        principal.trim().to_string(),
                    );
                } else {
                    let normalized = normalize_mtls_fingerprint(entry);
                    mtls_fingerprints.insert(normalized.clone(), normalized);
                }
            }
        }
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(api_keys),
            mtls_subjects: Arc::new(mtls_subjects),
            mtls_fingerprints: Arc::new(mtls_fingerprints),
            mtls_enabled: env_bool("PELAGO_MTLS_ENABLED", false),
            mtls_default_role: std::env::var("PELAGO_MTLS_DEFAULT_ROLE")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "service".to_string()),
            db: None,
            default_ttl_secs: env_i64("PELAGO_AUTH_TOKEN_TTL_SECS", 3600).max(60),
            max_ttl_secs: env_i64("PELAGO_AUTH_MAX_TOKEN_TTL_SECS", 86_400).max(300),
        }
    }

    pub fn with_db(db: PelagoDb) -> Self {
        let mut runtime = Self::new();
        runtime.db = Some(db);
        runtime
    }

    async fn validate_bearer_token(&self, token: &str) -> Option<TokenSession> {
        if let Some(session) = self
            .sessions
            .read()
            .await
            .get(token)
            .cloned()
            .filter(|s| s.expires_at > now_secs())
        {
            return Some(session);
        }

        let db = self.db.clone()?;
        let persisted = get_token_session_by_access(&db, token)
            .await
            .ok()
            .flatten()?;
        if !is_active_persistent_session(&persisted) {
            return None;
        }

        let session = TokenSession {
            principal_id: persisted.principal_id.clone(),
            principal_type: persisted.principal_type.clone(),
            roles: persisted.roles.clone(),
            expires_at: persisted.expires_at,
        };

        self.sessions
            .write()
            .await
            .insert(token.to_string(), session.clone());
        self.refresh_tokens
            .write()
            .await
            .insert(persisted.refresh_token, token.to_string());

        Some(session)
    }

    pub async fn validate_bearer_token_principal(&self, token: &str) -> Option<AuthPrincipal> {
        let session = self.validate_bearer_token(token).await?;
        Some(AuthPrincipal {
            principal_id: session.principal_id,
            principal_type: session.principal_type,
            roles: session.roles,
        })
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

    pub fn validate_bearer_token_blocking(&self, token: &str) -> Option<AuthPrincipal> {
        if let Some(principal) = self.validate_bearer_token_sync(token) {
            return Some(principal);
        }

        if self.db.is_none() {
            return None;
        }

        let handle = tokio::runtime::Handle::try_current().ok()?;
        tokio::task::block_in_place(|| handle.block_on(self.validate_bearer_token_principal(token)))
    }

    pub fn principal_for_api_key(&self, api_key: &str) -> Option<AuthPrincipal> {
        self.api_keys.get(api_key).map(|principal| AuthPrincipal {
            principal_id: principal.clone(),
            principal_type: "api_key".to_string(),
            roles: vec!["admin".to_string()],
        })
    }

    pub fn principal_for_mtls_subject(&self, subject: &str) -> Option<AuthPrincipal> {
        if !self.mtls_enabled {
            return None;
        }
        let subject = subject.trim();
        if subject.is_empty() {
            return None;
        }
        let principal_id = self
            .mtls_subjects
            .get(subject)
            .cloned()
            .unwrap_or_else(|| subject.to_string());
        Some(AuthPrincipal {
            principal_id,
            principal_type: "mtls".to_string(),
            roles: vec![self.mtls_default_role.clone()],
        })
    }

    pub fn principal_for_mtls_fingerprint(&self, fingerprint: &str) -> Option<AuthPrincipal> {
        if !self.mtls_enabled {
            return None;
        }

        let normalized = normalize_mtls_fingerprint(fingerprint);
        if normalized == "sha256:" {
            return None;
        }

        let principal_id = if self.mtls_fingerprints.is_empty() {
            normalized
        } else {
            self.mtls_fingerprints.get(&normalized).cloned()?
        };

        Some(AuthPrincipal {
            principal_id,
            principal_type: "mtls".to_string(),
            roles: vec![self.mtls_default_role.clone()],
        })
    }

    async fn issue_tokens(
        &self,
        principal_id: String,
        principal_type: String,
        roles: Vec<String>,
        ttl_secs: Option<i64>,
    ) -> Result<(String, String, i64), Status> {
        let access = Uuid::now_v7().to_string();
        let refresh = Uuid::now_v7().to_string();
        let ttl = ttl_secs
            .unwrap_or(self.default_ttl_secs)
            .clamp(60, self.max_ttl_secs);
        let issued_at = now_secs();
        let expires_at = issued_at + ttl;

        let session = TokenSession {
            principal_id: principal_id.clone(),
            principal_type: principal_type.clone(),
            roles: roles.clone(),
            expires_at,
        };
        self.sessions.write().await.insert(access.clone(), session);
        self.refresh_tokens
            .write()
            .await
            .insert(refresh.clone(), access.clone());

        if let Some(db) = &self.db {
            let persistent = AuthTokenSession {
                access_token: access.clone(),
                refresh_token: refresh.clone(),
                principal_id,
                principal_type,
                roles,
                issued_at,
                expires_at,
                revoked_at: None,
            };
            upsert_token_session(db, &persistent)
                .await
                .map_err(|e| Status::internal(format!("token persistence failed: {}", e)))?;
        }

        Ok((access, refresh, expires_at))
    }

    async fn session_from_refresh_token(&self, refresh_token: &str) -> Option<TokenSession> {
        if let Some(access) = self.refresh_tokens.read().await.get(refresh_token).cloned() {
            if let Some(session) = self
                .sessions
                .read()
                .await
                .get(&access)
                .cloned()
                .filter(|s| s.expires_at > now_secs())
            {
                return Some(session);
            }
        }

        let db = self.db.clone()?;
        let persisted = get_token_session_by_refresh(&db, refresh_token)
            .await
            .ok()
            .flatten()?;
        if !is_active_persistent_session(&persisted) {
            return None;
        }

        let session = TokenSession {
            principal_id: persisted.principal_id.clone(),
            principal_type: persisted.principal_type.clone(),
            roles: persisted.roles.clone(),
            expires_at: persisted.expires_at,
        };
        self.sessions
            .write()
            .await
            .insert(persisted.access_token.clone(), session.clone());
        self.refresh_tokens
            .write()
            .await
            .insert(refresh_token.to_string(), persisted.access_token);
        Some(session)
    }

    async fn revoke_tokens(
        &self,
        access_token: Option<&str>,
        refresh_token: Option<&str>,
    ) -> Result<bool, Status> {
        let mut revoked = false;

        if let Some(access_token) = access_token {
            if self.sessions.write().await.remove(access_token).is_some() {
                revoked = true;
            }
            self.refresh_tokens
                .write()
                .await
                .retain(|_, access| access != access_token);
        }

        if let Some(refresh_token) = refresh_token {
            if self
                .refresh_tokens
                .write()
                .await
                .remove(refresh_token)
                .is_some()
            {
                revoked = true;
            }
        }

        if let Some(db) = &self.db {
            if let Some(access_token) = access_token {
                revoked |= revoke_access_token(db, access_token)
                    .await
                    .map_err(|e| Status::internal(format!("token revoke failed: {}", e)))?;
            }
            if let Some(refresh_token) = refresh_token {
                revoked |= revoke_refresh_token(db, refresh_token)
                    .await
                    .map_err(|e| Status::internal(format!("refresh revoke failed: {}", e)))?;
            }
        }

        Ok(revoked)
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

    async fn append_auth_audit(
        &self,
        principal_id: String,
        action: &str,
        resource: String,
        allowed: bool,
        reason: &str,
    ) {
        let _ = append_audit_record(
            &self.db,
            AuditRecord {
                event_id: String::new(),
                timestamp: 0,
                principal_id,
                action: action.to_string(),
                resource,
                allowed,
                reason: reason.to_string(),
                metadata: HashMap::new(),
            },
        )
        .await;
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
                None,
            )
            .await?;

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
        let session = self
            .runtime
            .session_from_refresh_token(&req.refresh_token)
            .await
            .ok_or_else(|| Status::unauthenticated("refresh session not found"))?;
        let refreshed_principal_id = session.principal_id.clone();

        let (new_access, new_refresh, expires_at) = self
            .runtime
            .issue_tokens(
                session.principal_id,
                session.principal_type,
                session.roles,
                None,
            )
            .await?;

        let _ = self
            .runtime
            .revoke_tokens(None, Some(&req.refresh_token))
            .await;

        self.append_auth_audit(
            refreshed_principal_id,
            "auth.token.refresh",
            new_access.clone(),
            true,
            "token refreshed",
        )
        .await;

        Ok(Response::new(RefreshTokenResponse {
            access_token: new_access,
            expires_at,
            refresh_token: new_refresh,
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

        self.append_auth_audit(
            session.principal_id.clone(),
            "auth.token.validate",
            req.token,
            true,
            "token validated",
        )
        .await;

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

    async fn issue_service_token(
        &self,
        request: Request<IssueServiceTokenRequest>,
    ) -> Result<Response<IssueServiceTokenResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = require_field(req.context, "context")?;
        authorize_with_context(&self.db, principal.as_ref(), "auth.token.write", &ctx, "*").await?;

        if req.principal_id.is_empty() {
            return Err(Status::invalid_argument("principal_id cannot be empty"));
        }

        let roles = if req.roles.is_empty() {
            vec!["service".to_string()]
        } else {
            req.roles
        };
        let ttl = if req.ttl_secs <= 0 {
            None
        } else {
            Some(req.ttl_secs)
        };

        let (access_token, refresh_token, expires_at) = self
            .runtime
            .issue_tokens(
                req.principal_id.clone(),
                "service".to_string(),
                roles.clone(),
                ttl,
            )
            .await?;

        let _ = append_audit_record(
            &self.db,
            AuditRecord {
                event_id: String::new(),
                timestamp: 0,
                principal_id: principal
                    .as_ref()
                    .map(|p| p.principal_id.clone())
                    .unwrap_or_else(|| "system".to_string()),
                action: "auth.token.issue".to_string(),
                resource: req.principal_id.clone(),
                allowed: true,
                reason: "service token issued".to_string(),
                metadata: HashMap::new(),
            },
        )
        .await;

        Ok(Response::new(IssueServiceTokenResponse {
            access_token,
            refresh_token,
            expires_at,
            principal: Some(Principal {
                principal_id: req.principal_id,
                principal_type: "service".to_string(),
                roles,
            }),
        }))
    }

    async fn revoke_token(
        &self,
        request: Request<RevokeTokenRequest>,
    ) -> Result<Response<RevokeTokenResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = require_field(req.context, "context")?;
        authorize_with_context(&self.db, principal.as_ref(), "auth.token.write", &ctx, "*").await?;

        if req.access_token.is_empty() && req.refresh_token.is_empty() {
            return Err(Status::invalid_argument(
                "either access_token or refresh_token must be provided",
            ));
        }

        let revoked = self
            .runtime
            .revoke_tokens(
                if req.access_token.is_empty() {
                    None
                } else {
                    Some(req.access_token.as_str())
                },
                if req.refresh_token.is_empty() {
                    None
                } else {
                    Some(req.refresh_token.as_str())
                },
            )
            .await?;

        let _ = append_audit_record(
            &self.db,
            AuditRecord {
                event_id: String::new(),
                timestamp: 0,
                principal_id: principal
                    .as_ref()
                    .map(|p| p.principal_id.clone())
                    .unwrap_or_else(|| "system".to_string()),
                action: "auth.token.revoke".to_string(),
                resource: if req.access_token.is_empty() {
                    req.refresh_token.clone()
                } else {
                    req.access_token.clone()
                },
                allowed: revoked,
                reason: if revoked {
                    "token revoked".to_string()
                } else {
                    "token not found".to_string()
                },
                metadata: HashMap::new(),
            },
        )
        .await;

        Ok(Response::new(RevokeTokenResponse { revoked }))
    }

    async fn introspect_token(
        &self,
        request: Request<IntrospectTokenRequest>,
    ) -> Result<Response<IntrospectTokenResponse>, Status> {
        let requester_principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = require_field(req.context, "context")?;
        authorize_with_context(
            &self.db,
            requester_principal.as_ref(),
            "auth.token.read",
            &ctx,
            "*",
        )
        .await?;

        if req.token.is_empty() {
            return Err(Status::invalid_argument("token cannot be empty"));
        }

        let session = self.runtime.validate_bearer_token(&req.token).await;
        let (active, principal, expires_at, reason) = if let Some(session) = session {
            (
                true,
                Some(Principal {
                    principal_id: session.principal_id,
                    principal_type: session.principal_type,
                    roles: session.roles,
                }),
                session.expires_at,
                "token active".to_string(),
            )
        } else {
            (
                false,
                None,
                0,
                "token not found, expired, or revoked".to_string(),
            )
        };

        self.append_auth_audit(
            requester_principal
                .as_ref()
                .map(|p| p.principal_id.clone())
                .unwrap_or_else(|| "system".to_string()),
            "auth.token.introspect",
            req.token.clone(),
            active,
            &reason,
        )
        .await;

        Ok(Response::new(IntrospectTokenResponse {
            active,
            principal,
            expires_at,
            reason,
        }))
    }

    async fn create_policy(
        &self,
        request: Request<CreatePolicyRequest>,
    ) -> Result<Response<CreatePolicyResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = require_field(req.context, "context")?;
        authorize_with_context(&self.db, principal.as_ref(), "auth.policy.write", &ctx, "*")
            .await?;
        let policy = require_field(req.policy, "policy")?;
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
        let ctx = require_field(req.context, "context")?;
        authorize_with_context(&self.db, principal.as_ref(), "auth.policy.read", &ctx, "*").await?;
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
        let ctx = require_field(req.context, "context")?;
        authorize_with_context(&self.db, principal.as_ref(), "auth.policy.read", &ctx, "*").await?;
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
        let ctx = require_field(req.context, "context")?;
        authorize_with_context(&self.db, principal.as_ref(), "auth.policy.write", &ctx, "*")
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
        let ctx = require_field(req.context, "context")?;
        authorize_with_context(&self.db, principal.as_ref(), "auth.policy.check", &ctx, "*")
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
                path: p.path,
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
                path: p.path.clone(),
            })
            .collect(),
        created_at: policy.created_at,
    }
}

fn is_active_persistent_session(session: &AuthTokenSession) -> bool {
    session.revoked_at.is_none() && session.expires_at > now_secs()
}

fn normalize_mtls_fingerprint(raw: &str) -> String {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("sha256:") {
        trimmed
    } else {
        format!("sha256:{}", trimmed)
    }
}

fn env_i64(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or(default)
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|raw| match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
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
                path: "db1/ns1/data/Person".to_string(),
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
        assert_eq!(back.permissions[0].path, "db1/ns1/data/Person");
        assert_eq!(back.created_at, 123);
    }

    #[test]
    fn test_mtls_subject_principal_mapping() {
        let mut subjects = HashMap::new();
        subjects.insert("CN=svc-a".to_string(), "svc-a".to_string());
        let runtime = AuthRuntime {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(HashMap::new()),
            mtls_subjects: Arc::new(subjects),
            mtls_fingerprints: Arc::new(HashMap::new()),
            mtls_enabled: true,
            mtls_default_role: "service".to_string(),
            db: None,
            default_ttl_secs: 3600,
            max_ttl_secs: 86_400,
        };

        let principal = runtime
            .principal_for_mtls_subject("CN=svc-a")
            .expect("mapped subject should resolve");
        assert_eq!(principal.principal_id, "svc-a");
        assert_eq!(principal.principal_type, "mtls");
        assert_eq!(principal.roles, vec!["service"]);
    }

    #[test]
    fn test_mtls_subject_principal_uses_subject_when_unmapped() {
        let runtime = AuthRuntime {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(HashMap::new()),
            mtls_subjects: Arc::new(HashMap::new()),
            mtls_fingerprints: Arc::new(HashMap::new()),
            mtls_enabled: true,
            mtls_default_role: "service".to_string(),
            db: None,
            default_ttl_secs: 3600,
            max_ttl_secs: 86_400,
        };

        let principal = runtime
            .principal_for_mtls_subject("CN=svc-b")
            .expect("subject should map to itself by default");
        assert_eq!(principal.principal_id, "CN=svc-b");
        assert_eq!(principal.principal_type, "mtls");
        assert_eq!(principal.roles, vec!["service"]);
    }

    #[test]
    fn test_mtls_fingerprint_principal_mapping() {
        let runtime = AuthRuntime {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(HashMap::new()),
            mtls_subjects: Arc::new(HashMap::new()),
            mtls_fingerprints: Arc::new(HashMap::from([(
                "sha256:abc123".to_string(),
                "svc-fp-a".to_string(),
            )])),
            mtls_enabled: true,
            mtls_default_role: "service".to_string(),
            db: None,
            default_ttl_secs: 3600,
            max_ttl_secs: 86_400,
        };

        let principal = runtime
            .principal_for_mtls_fingerprint("ABC123")
            .expect("fingerprint should resolve");
        assert_eq!(principal.principal_id, "svc-fp-a");
        assert_eq!(principal.principal_type, "mtls");
        assert_eq!(principal.roles, vec!["service"]);
    }

    #[test]
    fn test_mtls_fingerprint_requires_mapping_when_configured() {
        let runtime = AuthRuntime {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(HashMap::new()),
            mtls_subjects: Arc::new(HashMap::new()),
            mtls_fingerprints: Arc::new(HashMap::from([(
                "sha256:known".to_string(),
                "svc-known".to_string(),
            )])),
            mtls_enabled: true,
            mtls_default_role: "service".to_string(),
            db: None,
            default_ttl_secs: 3600,
            max_ttl_secs: 86_400,
        };

        assert!(runtime.principal_for_mtls_fingerprint("unknown").is_none());
    }
}
