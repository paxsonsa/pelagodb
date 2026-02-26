use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderName, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use futures_util::StreamExt;
use pelago_api::{
    AdminServiceImpl, AuthPrincipal, AuthRuntime, AuthServiceImpl, EdgeServiceImpl,
    HealthServiceImpl, NodeServiceImpl, QueryServiceImpl, SchemaServiceImpl, WatchRegistry,
    WatchServiceImpl,
};
use pelago_proto::admin_service_server::AdminService;
use pelago_proto::auth_service_server::AuthService;
use pelago_proto::edge_service_server::EdgeService;
use pelago_proto::health_service_server::HealthService;
use pelago_proto::node_service_server::NodeService;
use pelago_proto::query_service_server::QueryService;
use pelago_proto::schema_service_server::SchemaService;
use pelago_proto::watch_service_server::WatchService;
use pelago_proto::{
    AuthenticateRequest, CancelSubscriptionRequest, DropIndexRequest, EdgeDirection,
    ExecutePqlRequest, ExplainRequest, FindNodesRequest, GetNodeRequest,
    GetReplicationStatusRequest, GetSchemaRequest, HealthCheckRequest, ListEdgesRequest,
    ListJobsRequest, ListSitesRequest, ListSubscriptionsRequest, NodeRef, QueryAuditLogRequest,
    ReadConsistency, RefreshTokenRequest, RequestContext, SetNamespaceSchemaOwnerRequest,
    SnapshotMode, TransferNamespaceSchemaOwnerRequest, TransferOwnershipRequest, TraverseRequest,
    TraverseResult, ValidateTokenRequest, WatchEvent, WatchNamespaceRequest, WatchOptions,
    WatchPointRequest, WatchQueryRequest,
};
use pelago_storage::{CachedReadPath, IdAllocator, NodeStore, PelagoDb, SchemaRegistry};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex};
use tonic::Request as TonicRequest;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;
use uuid::Uuid;

use crate::auth_identity::resolve_principal_from_http_headers;

#[derive(Clone)]
pub struct UiServerConfig {
    pub addr: String,
    pub assets_dir: String,
    pub title: String,
}

#[derive(Clone)]
pub struct UiServerDeps {
    pub db: PelagoDb,
    pub schema_registry: Arc<SchemaRegistry>,
    pub id_allocator: Arc<IdAllocator>,
    pub node_store: Arc<NodeStore>,
    pub cached_read_path: Option<Arc<CachedReadPath>>,
    pub watch_registry: WatchRegistry,
    pub auth_runtime: AuthRuntime,
    pub auth_required: bool,
    pub mtls_subject_header: String,
    pub default_database: String,
    pub default_namespace: String,
    pub site_id: u8,
    pub metrics_enabled: bool,
    pub metrics_addr: String,
}

#[derive(Clone)]
struct UiState {
    auth_required: bool,
    mtls_subject_header: String,
    auth_runtime: AuthRuntime,
    default_database: String,
    default_namespace: String,
    site_id: u8,
    metrics_enabled: bool,
    metrics_addr: String,
    schema_service: Arc<SchemaServiceImpl>,
    node_service: Arc<NodeServiceImpl>,
    edge_service: Arc<EdgeServiceImpl>,
    query_service: Arc<QueryServiceImpl>,
    admin_service: Arc<AdminServiceImpl>,
    health_service: Arc<HealthServiceImpl>,
    watch_service: Arc<WatchServiceImpl>,
    auth_service: Arc<AuthServiceImpl>,
    rate_limit: Arc<Mutex<RateLimitWindow>>,
}

#[derive(Clone)]
struct UiRequestContext {
    database: String,
    namespace: String,
    site_id: String,
    request_id: String,
}

struct RateLimitWindow {
    started_at: Instant,
    requests: u32,
}

const UI_RATE_LIMIT_MAX_REQUESTS_PER_WINDOW: u32 = 300;
const UI_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "BAD_REQUEST",
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "UNAUTHORIZED",
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND",
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL",
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": {
                    "code": self.code,
                    "message": self.message
                }
            })),
        )
            .into_response()
    }
}

impl From<tonic::Status> for ApiError {
    fn from(value: tonic::Status) -> Self {
        let status = match value.code() {
            tonic::Code::InvalidArgument => StatusCode::BAD_REQUEST,
            tonic::Code::Unauthenticated => StatusCode::UNAUTHORIZED,
            tonic::Code::PermissionDenied => StatusCode::FORBIDDEN,
            tonic::Code::NotFound => StatusCode::NOT_FOUND,
            tonic::Code::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
            tonic::Code::FailedPrecondition => StatusCode::PRECONDITION_FAILED,
            tonic::Code::OutOfRange => StatusCode::RANGE_NOT_SATISFIABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let code = match status {
            StatusCode::BAD_REQUEST => "BAD_REQUEST",
            StatusCode::UNAUTHORIZED => "UNAUTHORIZED",
            StatusCode::FORBIDDEN => "FORBIDDEN",
            StatusCode::NOT_FOUND => "NOT_FOUND",
            StatusCode::TOO_MANY_REQUESTS => "RESOURCE_EXHAUSTED",
            StatusCode::PRECONDITION_FAILED => "FAILED_PRECONDITION",
            StatusCode::RANGE_NOT_SATISFIABLE => "OUT_OF_RANGE",
            _ => "INTERNAL",
        };
        Self {
            status,
            code,
            message: value.message().to_string(),
        }
    }
}

pub async fn serve_ui(
    config: UiServerConfig,
    deps: UiServerDeps,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let state = Arc::new(build_state(deps));

    let api_router = Router::new()
        .route("/auth/authenticate", post(auth_authenticate))
        .route("/auth/refresh", post(auth_refresh))
        .route("/auth/validate", post(auth_validate))
        .route("/schema", get(schema_list))
        .route("/schema/:entity_type", get(schema_get))
        .route("/query/find", post(query_find))
        .route("/query/traverse", post(query_traverse))
        .route("/query/pql", post(query_pql))
        .route("/query/explain", post(query_explain))
        .route("/graph/node/:entity_type/:node_id", get(graph_get_node))
        .route("/graph/edges", get(graph_list_edges))
        .route("/state/health", get(state_health))
        .route("/state/sites", get(state_sites))
        .route("/state/replication", get(state_replication))
        .route("/state/jobs", get(state_jobs))
        .route("/state/audit", get(state_audit))
        .route("/state/watch/subscriptions", get(state_watch_subscriptions))
        .route("/metrics/raw", get(metrics_raw))
        .route("/watch/point/stream", post(watch_point_stream))
        .route("/watch/query/stream", post(watch_query_stream))
        .route("/watch/namespace/stream", post(watch_namespace_stream))
        .route("/watch/cancel", post(watch_cancel))
        .route("/admin/drop-index", post(admin_drop_index))
        .route("/admin/strip-property", post(admin_strip_property))
        .route(
            "/admin/set-namespace-schema-owner",
            post(admin_set_namespace_schema_owner),
        )
        .route(
            "/admin/transfer-namespace-schema-owner",
            post(admin_transfer_namespace_schema_owner),
        )
        .route(
            "/admin/transfer-node-ownership",
            post(admin_transfer_node_ownership),
        )
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            rate_limit_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth_middleware,
        ))
        .layer(RequestBodyLimitLayer::new(1024 * 1024));

    let ui_dir = config.assets_dir.clone();
    let ui_index = format!("{}/index.html", config.assets_dir);
    let static_router = Router::new()
        .route("/ui", get(|| async { Redirect::permanent("/ui/") }))
        .nest_service(
            "/ui/",
            ServeDir::new(ui_dir).fallback(ServeFile::new(ui_index)),
        );

    let app = Router::new()
        .nest("/ui/api/v1", api_router)
        .merge(static_router)
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            context_middleware,
        ))
        .with_state(Arc::clone(&state));

    let listener = tokio::net::TcpListener::bind(&config.addr)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind ui address {}: {}", config.addr, e))?;

    info!(
        "UI server enabled at http://{}/ui/ (title='{}')",
        config.addr, config.title
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            loop {
                if *shutdown.borrow() {
                    break;
                }
                if shutdown.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("ui server failed: {}", e))?;

    Ok(())
}

fn build_state(deps: UiServerDeps) -> UiState {
    let schema_service = Arc::new(SchemaServiceImpl::new(
        deps.db.clone(),
        Arc::clone(&deps.schema_registry),
    ));
    let node_service = Arc::new(NodeServiceImpl::new(
        deps.db.clone(),
        Arc::clone(&deps.schema_registry),
        Arc::clone(&deps.id_allocator),
        deps.site_id.to_string(),
        deps.cached_read_path.clone(),
    ));
    let edge_service = Arc::new(EdgeServiceImpl::new(
        deps.db.clone(),
        Arc::clone(&deps.schema_registry),
        Arc::clone(&deps.id_allocator),
        Arc::clone(&deps.node_store),
        deps.site_id.to_string(),
        deps.cached_read_path.clone(),
    ));
    let query_service = Arc::new(QueryServiceImpl::new(
        deps.db.clone(),
        Arc::clone(&deps.schema_registry),
        Arc::clone(&deps.id_allocator),
        deps.site_id.to_string(),
    ));
    let admin_service = Arc::new(AdminServiceImpl::new(Arc::new(deps.db.clone())));
    let watch_service = Arc::new(WatchServiceImpl::new(
        deps.db.clone(),
        Arc::clone(&deps.schema_registry),
        deps.watch_registry,
    ));
    let health_service = Arc::new(HealthServiceImpl::new());
    let auth_service = Arc::new(AuthServiceImpl::new(deps.db, deps.auth_runtime.clone()));

    UiState {
        auth_required: deps.auth_required,
        mtls_subject_header: deps.mtls_subject_header,
        auth_runtime: deps.auth_runtime,
        default_database: deps.default_database,
        default_namespace: deps.default_namespace,
        site_id: deps.site_id,
        metrics_enabled: deps.metrics_enabled,
        metrics_addr: deps.metrics_addr,
        schema_service,
        node_service,
        edge_service,
        query_service,
        admin_service,
        health_service,
        watch_service,
        auth_service,
        rate_limit: Arc::new(Mutex::new(RateLimitWindow {
            started_at: Instant::now(),
            requests: 0,
        })),
    }
}

async fn context_middleware(
    State(state): State<Arc<UiState>>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let database = req
        .headers()
        .get("x-pelago-database")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(&state.default_database)
        .to_string();

    let namespace = req
        .headers()
        .get("x-pelago-namespace")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(&state.default_namespace)
        .to_string();

    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| Uuid::now_v7().to_string());

    let resolved = UiRequestContext {
        database,
        namespace,
        site_id: state.site_id.to_string(),
        request_id: request_id.clone(),
    };
    req.extensions_mut().insert(resolved);

    let mut response = next.run(req).await;
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-request-id"), value);
    }
    response
}

async fn auth_middleware(
    State(state): State<Arc<UiState>>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let is_public_auth = path.ends_with("/auth/authenticate")
        || path.ends_with("/auth/refresh")
        || path.ends_with("/auth/validate");

    let principal = resolve_principal_from_http_headers(
        &state.auth_runtime,
        &state.mtls_subject_header,
        req.headers(),
    );

    if let Some(principal) = principal {
        req.extensions_mut().insert(principal);
    } else if state.auth_required && !is_public_auth {
        return ApiError::unauthorized(
            "authentication required: provide x-api-key, bearer token, or mTLS identity",
        )
        .into_response();
    }

    next.run(req).await
}

fn reserve_rate_limit_slot(window: &mut RateLimitWindow, now: Instant) -> Result<(), u64> {
    let elapsed = now.duration_since(window.started_at);
    if elapsed >= UI_RATE_LIMIT_WINDOW {
        window.started_at = now;
        window.requests = 0;
    }

    if window.requests >= UI_RATE_LIMIT_MAX_REQUESTS_PER_WINDOW {
        let retry_after = UI_RATE_LIMIT_WINDOW
            .checked_sub(now.duration_since(window.started_at))
            .unwrap_or(Duration::from_secs(0))
            .as_secs()
            .max(1);
        return Err(retry_after);
    }

    window.requests += 1;
    Ok(())
}

async fn rate_limit_middleware(
    State(state): State<Arc<UiState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let now = Instant::now();
    let retry_after = {
        let mut limit = state.rate_limit.lock().await;
        reserve_rate_limit_slot(&mut limit, now).err()
    };

    if let Some(retry_after_secs) = retry_after {
        let mut response = ApiError {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "RATE_LIMITED",
            message: format!(
                "ui api rate limit exceeded (max {} requests per {} seconds)",
                UI_RATE_LIMIT_MAX_REQUESTS_PER_WINDOW,
                UI_RATE_LIMIT_WINDOW.as_secs()
            ),
        }
        .into_response();

        if let Ok(value) = HeaderValue::from_str(&retry_after_secs.to_string()) {
            response.headers_mut().insert(RETRY_AFTER, value);
        }
        return response;
    }

    next.run(req).await
}

fn principal_from_extension(ext: Option<Extension<AuthPrincipal>>) -> Option<AuthPrincipal> {
    ext.map(|e| e.0)
}

fn proto_context(ctx: &UiRequestContext) -> RequestContext {
    RequestContext {
        database: ctx.database.clone(),
        namespace: ctx.namespace.clone(),
        site_id: ctx.site_id.clone(),
        request_id: ctx.request_id.clone(),
    }
}

fn with_principal<T>(payload: T, principal: Option<AuthPrincipal>) -> tonic::Request<T> {
    let mut req = TonicRequest::new(payload);
    if let Some(principal) = principal {
        req.extensions_mut().insert(principal);
    }
    req
}

fn decode_cursor_base64(raw: Option<&str>) -> Result<Vec<u8>, ApiError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    B64.decode(trimmed)
        .map_err(|e| ApiError::bad_request(format!("invalid base64 cursor: {}", e)))
}

fn encode_cursor_base64(raw: &[u8]) -> String {
    if raw.is_empty() {
        String::new()
    } else {
        B64.encode(raw)
    }
}

fn parse_read_consistency(raw: Option<&str>) -> i32 {
    match raw.unwrap_or("strong").trim().to_ascii_lowercase().as_str() {
        "strong" | "read_consistency_strong" => ReadConsistency::Strong as i32,
        "session" | "read_consistency_session" => ReadConsistency::Session as i32,
        "eventual" | "read_consistency_eventual" => ReadConsistency::Eventual as i32,
        _ => ReadConsistency::Strong as i32,
    }
}

fn parse_snapshot_mode(raw: Option<&str>, default_strict: bool) -> i32 {
    match raw
        .unwrap_or(if default_strict {
            "strict"
        } else {
            "best_effort"
        })
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "strict" | "snapshot_mode_strict" => SnapshotMode::Strict as i32,
        "best_effort" | "snapshot_mode_best_effort" => SnapshotMode::BestEffort as i32,
        _ if default_strict => SnapshotMode::Strict as i32,
        _ => SnapshotMode::BestEffort as i32,
    }
}

fn parse_edge_direction(raw: Option<&str>) -> i32 {
    match raw
        .unwrap_or("outgoing")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "incoming" | "edge_direction_incoming" | "in" => EdgeDirection::Incoming as i32,
        "both" | "edge_direction_both" => EdgeDirection::Both as i32,
        _ => EdgeDirection::Outgoing as i32,
    }
}

fn proto_value_to_json(value: &pelago_proto::Value) -> JsonValue {
    match &value.kind {
        Some(pelago_proto::value::Kind::StringValue(v)) => JsonValue::String(v.clone()),
        Some(pelago_proto::value::Kind::IntValue(v)) => json!(*v),
        Some(pelago_proto::value::Kind::FloatValue(v)) => json!(*v),
        Some(pelago_proto::value::Kind::BoolValue(v)) => json!(*v),
        Some(pelago_proto::value::Kind::TimestampValue(v)) => json!({"$timestamp": *v}),
        Some(pelago_proto::value::Kind::BytesValue(v)) => json!({"$bytes": B64.encode(v)}),
        Some(pelago_proto::value::Kind::NullValue(_)) | None => JsonValue::Null,
    }
}

fn proto_node_to_json(node: &pelago_proto::Node) -> JsonValue {
    let properties: HashMap<String, JsonValue> = node
        .properties
        .iter()
        .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
        .collect();

    json!({
        "id": node.id,
        "entity_type": node.entity_type,
        "properties": properties,
        "locality": node.locality,
        "created_at": node.created_at,
        "updated_at": node.updated_at
    })
}

fn proto_node_ref_to_json(node_ref: &NodeRef) -> JsonValue {
    json!({
        "entity_type": node_ref.entity_type,
        "node_id": node_ref.node_id,
        "database": node_ref.database,
        "namespace": node_ref.namespace
    })
}

fn proto_edge_to_json(edge: &pelago_proto::Edge) -> JsonValue {
    let properties: HashMap<String, JsonValue> = edge
        .properties
        .iter()
        .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
        .collect();

    json!({
        "edge_id": edge.edge_id,
        "source": edge.source.as_ref().map(proto_node_ref_to_json),
        "target": edge.target.as_ref().map(proto_node_ref_to_json),
        "label": edge.label,
        "properties": properties,
        "created_at": edge.created_at
    })
}

fn parse_subscription_type(raw: i32) -> &'static str {
    match pelago_proto::WatchSubscriptionType::try_from(raw)
        .unwrap_or(pelago_proto::WatchSubscriptionType::Unspecified)
    {
        pelago_proto::WatchSubscriptionType::Point => "point",
        pelago_proto::WatchSubscriptionType::Query => "query",
        pelago_proto::WatchSubscriptionType::Namespace => "namespace",
        pelago_proto::WatchSubscriptionType::Unspecified => "unspecified",
    }
}

fn parse_watch_event_type(raw: i32) -> &'static str {
    match pelago_proto::WatchEventType::try_from(raw)
        .unwrap_or(pelago_proto::WatchEventType::Unspecified)
    {
        pelago_proto::WatchEventType::Enter => "enter",
        pelago_proto::WatchEventType::Update => "update",
        pelago_proto::WatchEventType::Exit => "exit",
        pelago_proto::WatchEventType::Delete => "delete",
        pelago_proto::WatchEventType::Heartbeat => "heartbeat",
        pelago_proto::WatchEventType::Unspecified => "unspecified",
    }
}

#[derive(Debug, Deserialize)]
struct AuthAuthenticateBody {
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

async fn auth_authenticate(
    State(state): State<Arc<UiState>>,
    Json(body): Json<AuthAuthenticateBody>,
) -> Result<Json<JsonValue>, ApiError> {
    let req = AuthenticateRequest {
        api_key: body.api_key.unwrap_or_default(),
        username: body.username.unwrap_or_default(),
        password: body.password.unwrap_or_default(),
    };

    let response = state
        .auth_service
        .authenticate(TonicRequest::new(req))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "access_token": response.access_token,
        "refresh_token": response.refresh_token,
        "expires_at": response.expires_at,
        "principal": response.principal.map(|p| json!({
            "principal_id": p.principal_id,
            "principal_type": p.principal_type,
            "roles": p.roles,
        }))
    })))
}

#[derive(Debug, Deserialize)]
struct AuthRefreshBody {
    refresh_token: String,
}

async fn auth_refresh(
    State(state): State<Arc<UiState>>,
    Json(body): Json<AuthRefreshBody>,
) -> Result<Json<JsonValue>, ApiError> {
    let response = state
        .auth_service
        .refresh_token(TonicRequest::new(RefreshTokenRequest {
            refresh_token: body.refresh_token,
        }))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "access_token": response.access_token,
        "refresh_token": response.refresh_token,
        "expires_at": response.expires_at,
    })))
}

#[derive(Debug, Deserialize)]
struct AuthValidateBody {
    token: String,
}

async fn auth_validate(
    State(state): State<Arc<UiState>>,
    Json(body): Json<AuthValidateBody>,
) -> Result<Json<JsonValue>, ApiError> {
    let response = state
        .auth_service
        .validate_token(TonicRequest::new(ValidateTokenRequest {
            token: body.token,
        }))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "valid": response.valid,
        "expires_at": response.expires_at,
        "principal": response.principal.map(|p| json!({
            "principal_id": p.principal_id,
            "principal_type": p.principal_type,
            "roles": p.roles,
        }))
    })))
}

async fn schema_list(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = pelago_proto::ListSchemasRequest {
        context: Some(proto_context(&ctx)),
    };
    let principal = principal_from_extension(principal_ext);
    let response = state
        .schema_service
        .list_schemas(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    let schemas: Vec<JsonValue> = response
        .schemas
        .into_iter()
        .map(|schema| {
            let properties: HashMap<String, JsonValue> = schema
                .properties
                .into_iter()
                .map(|(name, prop)| {
                    (
                        name,
                        json!({
                            "type": prop.r#type,
                            "required": prop.required,
                            "index": prop.index,
                            "default": prop.default_value.map(|v| proto_value_to_json(&v)),
                        }),
                    )
                })
                .collect();
            let edges: HashMap<String, JsonValue> = schema
                .edges
                .into_iter()
                .map(|(name, edge)| {
                    (
                        name,
                        json!({
                            "direction": edge.direction,
                            "sort_key": edge.sort_key,
                            "ownership": edge.ownership,
                        }),
                    )
                })
                .collect();
            json!({
                "name": schema.name,
                "version": schema.version,
                "properties": properties,
                "edges": edges,
                "created_at": schema.created_at,
                "created_by": schema.created_by,
            })
        })
        .collect();

    Ok(Json(json!({"schemas": schemas})))
}

async fn schema_get(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Path(entity_type): Path<String>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = GetSchemaRequest {
        context: Some(proto_context(&ctx)),
        entity_type,
        version: 0,
    };
    let principal = principal_from_extension(principal_ext);
    let response = state
        .schema_service
        .get_schema(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    let Some(schema) = response.schema else {
        return Err(ApiError::not_found("schema not found"));
    };

    Ok(Json(json!({
        "schema": {
            "name": schema.name,
            "version": schema.version,
            "created_at": schema.created_at,
            "created_by": schema.created_by,
        }
    })))
}

#[derive(Debug, Deserialize)]
struct FindNodesBody {
    entity_type: String,
    cel_expression: String,
    consistency: Option<String>,
    fields: Option<Vec<String>>,
    limit: Option<u32>,
    cursor: Option<String>,
    snapshot_mode: Option<String>,
    allow_degrade_to_best_effort: Option<bool>,
}

async fn query_find(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<FindNodesBody>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = FindNodesRequest {
        context: Some(proto_context(&ctx)),
        entity_type: body.entity_type,
        cel_expression: body.cel_expression,
        consistency: parse_read_consistency(body.consistency.as_deref()),
        fields: body.fields.unwrap_or_default(),
        limit: body.limit.unwrap_or(100),
        cursor: decode_cursor_base64(body.cursor.as_deref())?,
        snapshot_mode: parse_snapshot_mode(body.snapshot_mode.as_deref(), true),
        allow_degrade_to_best_effort: body.allow_degrade_to_best_effort.unwrap_or(false),
    };
    let principal = principal_from_extension(principal_ext);
    let response = state
        .query_service
        .find_nodes(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?;

    let mut stream = response.into_inner();
    let mut items = Vec::new();
    let mut next_cursor = Vec::new();
    let mut degraded = false;
    let mut degraded_reason = String::new();
    let mut consistency_applied = 0i32;
    let mut snapshot_read_version = 0i64;

    while let Some(item) = stream.next().await {
        let row = item.map_err(ApiError::from)?;
        if !row.next_cursor.is_empty() {
            next_cursor = row.next_cursor.clone();
        }
        degraded = row.degraded;
        degraded_reason = row.degraded_reason.clone();
        consistency_applied = row.consistency_applied;
        snapshot_read_version = row.snapshot_read_version;
        if let Some(node) = row.node {
            items.push(proto_node_to_json(&node));
        }
    }

    Ok(Json(json!({
        "items": items,
        "next_cursor": encode_cursor_base64(&next_cursor),
        "degraded": degraded,
        "degraded_reason": degraded_reason,
        "consistency_applied": consistency_applied,
        "snapshot_read_version": snapshot_read_version,
    })))
}

#[derive(Debug, Deserialize)]
struct TraverseStepBody {
    edge_type: Option<String>,
    direction: Option<String>,
    edge_filter: Option<String>,
    node_filter: Option<String>,
    fields: Option<Vec<String>>,
    per_node_limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct TraverseBody {
    start_entity_type: String,
    start_node_id: String,
    steps: Vec<TraverseStepBody>,
    max_depth: Option<u32>,
    timeout_ms: Option<u32>,
    max_results: Option<u32>,
    consistency: Option<String>,
    cascade: Option<bool>,
    cursor: Option<String>,
    snapshot_mode: Option<String>,
    allow_degrade_to_best_effort: Option<bool>,
}

async fn query_traverse(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<TraverseBody>,
) -> Result<Json<JsonValue>, ApiError> {
    let steps = body
        .steps
        .into_iter()
        .map(|s| pelago_proto::TraversalStep {
            edge_type: s.edge_type.unwrap_or_default(),
            direction: parse_edge_direction(s.direction.as_deref()),
            edge_filter: s.edge_filter.unwrap_or_default(),
            node_filter: s.node_filter.unwrap_or_default(),
            fields: s.fields.unwrap_or_default(),
            per_node_limit: s.per_node_limit.unwrap_or(0),
            edge_fields: Vec::new(),
            sort: None,
        })
        .collect();

    let request = TraverseRequest {
        context: Some(proto_context(&ctx)),
        start: Some(NodeRef {
            entity_type: body.start_entity_type,
            node_id: body.start_node_id,
            database: String::new(),
            namespace: String::new(),
        }),
        steps,
        max_depth: body.max_depth.unwrap_or(4),
        timeout_ms: body.timeout_ms.unwrap_or(5_000),
        max_results: body.max_results.unwrap_or(1_000),
        consistency: parse_read_consistency(body.consistency.as_deref()),
        cascade: body.cascade.unwrap_or(false),
        cursor: decode_cursor_base64(body.cursor.as_deref())?,
        snapshot_mode: parse_snapshot_mode(body.snapshot_mode.as_deref(), false),
        allow_degrade_to_best_effort: body.allow_degrade_to_best_effort.unwrap_or(false),
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .query_service
        .traverse(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?;

    let mut stream = response.into_inner();
    let mut items = Vec::new();
    let mut next_cursor = Vec::new();
    let mut degraded = false;
    let mut degraded_reason = String::new();
    let mut consistency_applied = 0i32;
    let mut snapshot_read_version = 0i64;

    while let Some(item) = stream.next().await {
        let row: TraverseResult = item.map_err(ApiError::from)?;
        if !row.next_cursor.is_empty() {
            next_cursor = row.next_cursor.clone();
        }
        degraded = row.degraded;
        degraded_reason = row.degraded_reason.clone();
        consistency_applied = row.consistency_applied;
        snapshot_read_version = row.snapshot_read_version;
        let path_nodes: Vec<JsonValue> = row
            .path
            .into_iter()
            .map(|node_ref| proto_node_ref_to_json(&node_ref))
            .collect();
        items.push(json!({
            "depth": row.depth,
            "path": path_nodes,
            "node": row.node.as_ref().map(proto_node_to_json),
            "edge": row.edge.as_ref().map(proto_edge_to_json),
        }));
    }

    Ok(Json(json!({
        "items": items,
        "next_cursor": encode_cursor_base64(&next_cursor),
        "degraded": degraded,
        "degraded_reason": degraded_reason,
        "consistency_applied": consistency_applied,
        "snapshot_read_version": snapshot_read_version,
    })))
}

#[derive(Debug, Deserialize)]
struct ExecutePqlBody {
    pql: String,
    params: Option<HashMap<String, String>>,
    explain: Option<bool>,
    snapshot_mode: Option<String>,
    allow_degrade_to_best_effort: Option<bool>,
}

async fn query_pql(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<ExecutePqlBody>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = ExecutePqlRequest {
        context: Some(proto_context(&ctx)),
        pql: body.pql,
        params: body.params.unwrap_or_default(),
        explain: body.explain.unwrap_or(false),
        snapshot_mode: parse_snapshot_mode(body.snapshot_mode.as_deref(), false),
        allow_degrade_to_best_effort: body.allow_degrade_to_best_effort.unwrap_or(false),
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .query_service
        .execute_pql(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?;

    let mut stream = response.into_inner();
    let mut rows = Vec::new();
    let mut next_cursor = Vec::new();
    let mut degraded = false;
    let mut degraded_reason = String::new();
    let mut consistency_applied = 0i32;
    let mut snapshot_read_version = 0i64;

    while let Some(item) = stream.next().await {
        let row = item.map_err(ApiError::from)?;
        if !row.next_cursor.is_empty() {
            next_cursor = row.next_cursor.clone();
        }
        degraded = row.degraded;
        degraded_reason = row.degraded_reason.clone();
        consistency_applied = row.consistency_applied;
        snapshot_read_version = row.snapshot_read_version;

        rows.push(json!({
            "block_name": row.block_name,
            "node": row.node.as_ref().map(proto_node_to_json),
            "edge": row.edge.as_ref().map(proto_edge_to_json),
            "explain": row.explain,
        }));
    }

    Ok(Json(json!({
        "items": rows,
        "next_cursor": encode_cursor_base64(&next_cursor),
        "degraded": degraded,
        "degraded_reason": degraded_reason,
        "consistency_applied": consistency_applied,
        "snapshot_read_version": snapshot_read_version,
    })))
}

#[derive(Debug, Deserialize)]
struct ExplainBody {
    entity_type: String,
    cel_expression: String,
}

async fn query_explain(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<ExplainBody>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = ExplainRequest {
        context: Some(proto_context(&ctx)),
        entity_type: body.entity_type,
        cel_expression: body.cel_expression,
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .query_service
        .explain(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "plan": response.plan.map(|p| json!({
            "strategy": p.strategy,
            "indexes": p.indexes.into_iter().map(|i| json!({
                "field": i.field,
                "operator": i.operator,
                "value": i.value,
                "estimated_rows": i.estimated_rows,
            })).collect::<Vec<_>>(),
            "residual_filter": p.residual_filter,
        })),
        "explanation": response.explanation,
        "estimated_cost": response.estimated_cost,
        "estimated_rows": response.estimated_rows,
    })))
}

#[derive(Debug, Deserialize)]
struct GetNodeQuery {
    consistency: Option<String>,
    fields: Option<String>,
}

async fn graph_get_node(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Path((entity_type, node_id)): Path<(String, String)>,
    Query(query): Query<GetNodeQuery>,
) -> Result<Json<JsonValue>, ApiError> {
    let fields = query
        .fields
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .collect();

    let request = GetNodeRequest {
        context: Some(proto_context(&ctx)),
        entity_type,
        node_id,
        consistency: parse_read_consistency(query.consistency.as_deref()),
        fields,
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .node_service
        .get_node(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    let Some(node) = response.node else {
        return Err(ApiError::not_found("node not found"));
    };

    Ok(Json(json!({"node": proto_node_to_json(&node)})))
}

#[derive(Debug, Deserialize)]
struct ListEdgesQuery {
    entity_type: String,
    node_id: String,
    label: Option<String>,
    direction: Option<String>,
    consistency: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
}

async fn graph_list_edges(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Query(query): Query<ListEdgesQuery>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = ListEdgesRequest {
        context: Some(proto_context(&ctx)),
        entity_type: query.entity_type,
        node_id: query.node_id,
        label: query.label.unwrap_or_default(),
        direction: parse_edge_direction(query.direction.as_deref()),
        consistency: parse_read_consistency(query.consistency.as_deref()),
        limit: query.limit.unwrap_or(100),
        cursor: decode_cursor_base64(query.cursor.as_deref())?,
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .edge_service
        .list_edges(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?;

    let mut stream = response.into_inner();
    let mut rows = Vec::new();
    let mut next_cursor = Vec::new();

    while let Some(item) = stream.next().await {
        let row = item.map_err(ApiError::from)?;
        if !row.next_cursor.is_empty() {
            next_cursor = row.next_cursor.clone();
        }
        if let Some(edge) = row.edge {
            rows.push(proto_edge_to_json(&edge));
        }
    }

    Ok(Json(json!({
        "items": rows,
        "next_cursor": encode_cursor_base64(&next_cursor),
    })))
}

async fn state_health(State(state): State<Arc<UiState>>) -> Result<Json<JsonValue>, ApiError> {
    let response = state
        .health_service
        .check(TonicRequest::new(HealthCheckRequest {
            service: "pelago".to_string(),
        }))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "status": response.status,
    })))
}

async fn state_sites(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = ListSitesRequest {
        context: Some(proto_context(&ctx)),
    };
    let principal = principal_from_extension(principal_ext);
    let response = state
        .admin_service
        .list_sites(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "sites": response.sites.into_iter().map(|s| json!({
            "site_id": s.site_id,
            "site_name": s.site_name,
            "claimed_at": s.claimed_at,
            "status": s.status,
        })).collect::<Vec<_>>()
    })))
}

async fn state_replication(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = GetReplicationStatusRequest {
        context: Some(proto_context(&ctx)),
    };
    let principal = principal_from_extension(principal_ext);
    let response = state
        .admin_service
        .get_replication_status(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "peers": response.peers.into_iter().map(|p| json!({
            "remote_site_id": p.remote_site_id,
            "last_applied_versionstamp": encode_cursor_base64(&p.last_applied_versionstamp),
            "lag_events": p.lag_events,
            "updated_at": p.updated_at,
        })).collect::<Vec<_>>()
    })))
}

async fn state_jobs(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = ListJobsRequest {
        context: Some(proto_context(&ctx)),
    };
    let principal = principal_from_extension(principal_ext);
    let response = state
        .admin_service
        .list_jobs(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "jobs": response.jobs.into_iter().map(|job| json!({
            "job_id": job.job_id,
            "job_type": job.job_type,
            "status": job.status,
            "created_at": job.created_at,
            "updated_at": job.updated_at,
            "progress": job.progress,
            "error": job.error,
        })).collect::<Vec<_>>()
    })))
}

#[derive(Debug, Deserialize)]
struct AuditQuery {
    principal_id: Option<String>,
    action: Option<String>,
    from_timestamp: Option<i64>,
    to_timestamp: Option<i64>,
    limit: Option<u32>,
}

async fn state_audit(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = QueryAuditLogRequest {
        context: Some(proto_context(&ctx)),
        principal_id: query.principal_id.unwrap_or_default(),
        action: query.action.unwrap_or_default(),
        from_timestamp: query.from_timestamp.unwrap_or_default(),
        to_timestamp: query.to_timestamp.unwrap_or_default(),
        limit: query.limit.unwrap_or(100),
    };
    let principal = principal_from_extension(principal_ext);
    let response = state
        .admin_service
        .query_audit_log(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "events": response.events.into_iter().map(|event| json!({
            "event_id": event.event_id,
            "timestamp": event.timestamp,
            "principal_id": event.principal_id,
            "action": event.action,
            "resource": event.resource,
            "allowed": event.allowed,
            "reason": event.reason,
            "metadata": event.metadata,
        })).collect::<Vec<_>>()
    })))
}

async fn state_watch_subscriptions(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
) -> Result<Json<JsonValue>, ApiError> {
    let request = ListSubscriptionsRequest {
        context: Some(proto_context(&ctx)),
    };
    let principal = principal_from_extension(principal_ext);
    let response = state
        .watch_service
        .list_subscriptions(with_principal(request, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "subscriptions": response.subscriptions.into_iter().map(|sub| json!({
            "subscription_id": sub.subscription_id,
            "subscription_type": parse_subscription_type(sub.subscription_type),
            "database": sub.database,
            "namespace": sub.namespace,
            "created_at": sub.created_at,
            "expires_at": sub.expires_at,
        })).collect::<Vec<_>>()
    })))
}

async fn metrics_raw(State(state): State<Arc<UiState>>) -> Result<Response, ApiError> {
    if !state.metrics_enabled {
        return Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "METRICS_DISABLED",
            message: "prometheus metrics exporter is disabled".to_string(),
        });
    }

    let body = fetch_metrics_text(&state.metrics_addr).await?;
    Ok((
        StatusCode::OK,
        [("content-type", "text/plain; charset=utf-8")],
        body,
    )
        .into_response())
}

async fn fetch_metrics_text(addr: &str) -> Result<String, ApiError> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| ApiError::internal(format!("failed to connect to metrics exporter: {}", e)))?;

    let request = format!(
        "GET /metrics HTTP/1.1\\r\\nHost: {}\\r\\nConnection: close\\r\\n\\r\\n",
        addr
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| ApiError::internal(format!("failed to query metrics exporter: {}", e)))?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.map_err(|e| {
        ApiError::internal(format!("failed to read metrics exporter response: {}", e))
    })?;

    let payload = String::from_utf8_lossy(&buf);
    let mut parts = payload.splitn(2, "\\r\\n\\r\\n");
    let headers = parts.next().unwrap_or_default();
    let body = parts.next().unwrap_or_default();

    if !headers.starts_with("HTTP/1.1 200") && !headers.starts_with("HTTP/1.0 200") {
        return Err(ApiError::internal(format!(
            "metrics exporter returned non-200 response: {}",
            headers.lines().next().unwrap_or("<unknown>")
        )));
    }

    Ok(body.to_string())
}

#[derive(Debug, Deserialize, Default)]
struct WatchOptionsBody {
    include_initial: Option<bool>,
    ttl_secs: Option<u32>,
    max_queue_size: Option<u32>,
    heartbeat_secs: Option<u32>,
    initial_snapshot: Option<bool>,
}

impl WatchOptionsBody {
    fn to_proto(&self) -> WatchOptions {
        WatchOptions {
            include_initial: self.include_initial.unwrap_or(false),
            ttl_secs: self.ttl_secs.unwrap_or(0),
            max_queue_size: self.max_queue_size.unwrap_or(0),
            heartbeat_secs: self.heartbeat_secs.unwrap_or(0),
            initial_snapshot: self.initial_snapshot.unwrap_or(false),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct NodeWatchBody {
    entity_type: String,
    node_id: String,
    properties: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
struct NodeRefBody {
    entity_type: String,
    node_id: String,
    database: Option<String>,
    namespace: Option<String>,
}

impl NodeRefBody {
    fn to_proto(&self) -> NodeRef {
        NodeRef {
            entity_type: self.entity_type.clone(),
            node_id: self.node_id.clone(),
            database: self.database.clone().unwrap_or_default(),
            namespace: self.namespace.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct EdgeWatchBody {
    source: Option<NodeRefBody>,
    label: Option<String>,
    target: Option<NodeRefBody>,
    direction: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WatchPointStreamBody {
    entity_type: Option<String>,
    node_id: Option<String>,
    options: Option<WatchOptionsBody>,
    resume_after: Option<String>,
    nodes: Option<Vec<NodeWatchBody>>,
    edges: Option<Vec<EdgeWatchBody>>,
}

async fn watch_point_stream(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<WatchPointStreamBody>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let nodes = body
        .nodes
        .unwrap_or_default()
        .into_iter()
        .map(|n| pelago_proto::NodeWatch {
            entity_type: n.entity_type,
            node_id: n.node_id,
            properties: n.properties.unwrap_or_default(),
        })
        .collect();

    let edges = body
        .edges
        .unwrap_or_default()
        .into_iter()
        .map(|e| pelago_proto::EdgeWatch {
            source: e.source.map(|s| s.to_proto()),
            label: e.label.unwrap_or_default(),
            target: e.target.map(|t| t.to_proto()),
            direction: parse_edge_direction(e.direction.as_deref()),
        })
        .collect();

    let req = WatchPointRequest {
        context: Some(proto_context(&ctx)),
        entity_type: body.entity_type.unwrap_or_default(),
        node_id: body.node_id.unwrap_or_default(),
        options: Some(body.options.unwrap_or_default().to_proto()),
        resume_after: decode_cursor_base64(body.resume_after.as_deref())?,
        nodes,
        edges,
    };

    let principal = principal_from_extension(principal_ext);
    let stream = state
        .watch_service
        .watch_point(with_principal(req, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(stream_watch_events(stream))
}

#[derive(Debug, Deserialize)]
struct WatchQueryStreamBody {
    entity_type: Option<String>,
    cel_expression: Option<String>,
    pql_query: Option<String>,
    options: Option<WatchOptionsBody>,
    resume_after: Option<String>,
}

async fn watch_query_stream(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<WatchQueryStreamBody>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let req = WatchQueryRequest {
        context: Some(proto_context(&ctx)),
        entity_type: body.entity_type.unwrap_or_default(),
        cel_expression: body.cel_expression.unwrap_or_default(),
        pql_query: body.pql_query.unwrap_or_default(),
        options: Some(body.options.unwrap_or_default().to_proto()),
        resume_after: decode_cursor_base64(body.resume_after.as_deref())?,
    };

    let principal = principal_from_extension(principal_ext);
    let stream = state
        .watch_service
        .watch_query(with_principal(req, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(stream_watch_events(stream))
}

#[derive(Debug, Deserialize)]
struct WatchNamespaceStreamBody {
    options: Option<WatchOptionsBody>,
    resume_after: Option<String>,
}

async fn watch_namespace_stream(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<WatchNamespaceStreamBody>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let req = WatchNamespaceRequest {
        context: Some(proto_context(&ctx)),
        options: Some(body.options.unwrap_or_default().to_proto()),
        resume_after: decode_cursor_base64(body.resume_after.as_deref())?,
    };

    let principal = principal_from_extension(principal_ext);
    let stream = state
        .watch_service
        .watch_namespace(with_principal(req, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(stream_watch_events(stream))
}

fn stream_watch_events(
    stream: impl futures_util::Stream<Item = Result<WatchEvent, tonic::Status>> + Send + 'static,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let mapped = stream.map(|item| {
        let event = match item {
            Ok(ev) => {
                let payload = json!({
                    "subscription_id": ev.subscription_id,
                    "type": parse_watch_event_type(ev.r#type),
                    "versionstamp": encode_cursor_base64(&ev.versionstamp),
                    "node": ev.node.as_ref().map(proto_node_to_json),
                    "edge": ev.edge.as_ref().map(proto_edge_to_json),
                    "reason": ev.reason,
                });
                Event::default().event("watch").data(payload.to_string())
            }
            Err(status) => {
                let payload = json!({
                    "code": status.code().to_string(),
                    "message": status.message(),
                });
                Event::default().event("error").data(payload.to_string())
            }
        };
        Ok::<Event, Infallible>(event)
    });

    Sse::new(mapped).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(20)))
}

#[derive(Debug, Deserialize)]
struct CancelWatchBody {
    subscription_id: String,
}

async fn watch_cancel(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<CancelWatchBody>,
) -> Result<Json<JsonValue>, ApiError> {
    let req = CancelSubscriptionRequest {
        context: Some(proto_context(&ctx)),
        subscription_id: body.subscription_id,
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .watch_service
        .cancel_subscription(with_principal(req, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({"cancelled": response.cancelled})))
}

#[derive(Debug, Deserialize)]
struct DropIndexBody {
    entity_type: String,
    property_name: String,
    confirm: String,
}

async fn admin_drop_index(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<DropIndexBody>,
) -> Result<Json<JsonValue>, ApiError> {
    if body.confirm != "DROP_INDEX" {
        return Err(ApiError::bad_request(
            "confirmation token mismatch (expected DROP_INDEX)",
        ));
    }

    let req = DropIndexRequest {
        context: Some(proto_context(&ctx)),
        entity_type: body.entity_type,
        property_name: body.property_name,
    };
    let principal = principal_from_extension(principal_ext);
    let response = state
        .admin_service
        .drop_index(with_principal(req, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({"entries_deleted": response.entries_deleted})))
}

#[derive(Debug, Deserialize)]
struct StripPropertyBody {
    entity_type: String,
    property_name: String,
    confirm: String,
}

async fn admin_strip_property(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<StripPropertyBody>,
) -> Result<Json<JsonValue>, ApiError> {
    if body.confirm != "STRIP_PROPERTY" {
        return Err(ApiError::bad_request(
            "confirmation token mismatch (expected STRIP_PROPERTY)",
        ));
    }

    let req = pelago_proto::StripPropertyRequest {
        context: Some(proto_context(&ctx)),
        entity_type: body.entity_type,
        property_name: body.property_name,
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .admin_service
        .strip_property(with_principal(req, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({"job_id": response.job_id})))
}

#[derive(Debug, Deserialize)]
struct SetNamespaceSchemaOwnerBody {
    site_id: Option<String>,
    confirm: String,
}

async fn admin_set_namespace_schema_owner(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<SetNamespaceSchemaOwnerBody>,
) -> Result<Json<JsonValue>, ApiError> {
    if body.confirm != "SET_NAMESPACE_SCHEMA_OWNER" {
        return Err(ApiError::bad_request(
            "confirmation token mismatch (expected SET_NAMESPACE_SCHEMA_OWNER)",
        ));
    }

    let req = SetNamespaceSchemaOwnerRequest {
        context: Some(proto_context(&ctx)),
        site_id: body.site_id.unwrap_or_default(),
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .admin_service
        .set_namespace_schema_owner(with_principal(req, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "settings": response.settings.map(|s| json!({
            "database": s.database,
            "namespace": s.namespace,
            "schema_owner_site_id": s.schema_owner_site_id,
            "epoch": s.epoch,
            "updated_at": s.updated_at,
        }))
    })))
}

#[derive(Debug, Deserialize)]
struct TransferNamespaceSchemaOwnerBody {
    expected_site_id: String,
    target_site_id: String,
    confirm: String,
}

async fn admin_transfer_namespace_schema_owner(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<TransferNamespaceSchemaOwnerBody>,
) -> Result<Json<JsonValue>, ApiError> {
    if body.confirm != "TRANSFER_NAMESPACE_SCHEMA_OWNER" {
        return Err(ApiError::bad_request(
            "confirmation token mismatch (expected TRANSFER_NAMESPACE_SCHEMA_OWNER)",
        ));
    }

    let req = TransferNamespaceSchemaOwnerRequest {
        context: Some(proto_context(&ctx)),
        expected_site_id: body.expected_site_id,
        target_site_id: body.target_site_id,
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .admin_service
        .transfer_namespace_schema_owner(with_principal(req, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "settings": response.settings.map(|s| json!({
            "database": s.database,
            "namespace": s.namespace,
            "schema_owner_site_id": s.schema_owner_site_id,
            "epoch": s.epoch,
            "updated_at": s.updated_at,
        }))
    })))
}

#[derive(Debug, Deserialize)]
struct TransferNodeOwnershipBody {
    entity_type: String,
    node_id: String,
    target_site_id: String,
    confirm: String,
}

async fn admin_transfer_node_ownership(
    State(state): State<Arc<UiState>>,
    Extension(ctx): Extension<UiRequestContext>,
    principal_ext: Option<Extension<AuthPrincipal>>,
    Json(body): Json<TransferNodeOwnershipBody>,
) -> Result<Json<JsonValue>, ApiError> {
    if body.confirm != "TRANSFER_NODE_OWNERSHIP" {
        return Err(ApiError::bad_request(
            "confirmation token mismatch (expected TRANSFER_NODE_OWNERSHIP)",
        ));
    }

    let req = TransferOwnershipRequest {
        context: Some(proto_context(&ctx)),
        entity_type: body.entity_type,
        node_id: body.node_id,
        target_site_id: body.target_site_id,
    };

    let principal = principal_from_extension(principal_ext);
    let response = state
        .node_service
        .transfer_ownership(with_principal(req, principal))
        .await
        .map_err(ApiError::from)?
        .into_inner();

    Ok(Json(json!({
        "transferred": response.transferred,
        "previous_site_id": response.previous_site_id,
        "current_site_id": response.current_site_id,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_cursor_base64_empty() {
        let out = decode_cursor_base64(None).expect("empty cursor should decode");
        assert!(out.is_empty());
    }

    #[test]
    fn test_parse_read_consistency_default_strong() {
        assert_eq!(
            parse_read_consistency(None),
            pelago_proto::ReadConsistency::Strong as i32
        );
    }

    #[test]
    fn test_reserve_rate_limit_slot_returns_retry_after_when_exhausted() {
        let start = Instant::now();
        let mut window = RateLimitWindow {
            started_at: start,
            requests: UI_RATE_LIMIT_MAX_REQUESTS_PER_WINDOW,
        };
        let retry_after =
            reserve_rate_limit_slot(&mut window, start + Duration::from_secs(10)).unwrap_err();
        assert_eq!(retry_after, 50);
    }

    #[test]
    fn test_reserve_rate_limit_slot_resets_after_window() {
        let start = Instant::now();
        let mut window = RateLimitWindow {
            started_at: start,
            requests: UI_RATE_LIMIT_MAX_REQUESTS_PER_WINDOW,
        };

        let result = reserve_rate_limit_slot(&mut window, start + UI_RATE_LIMIT_WINDOW);
        assert!(result.is_ok());
        assert_eq!(window.requests, 1);
    }
}
