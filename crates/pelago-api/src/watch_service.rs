//! WatchService gRPC handler.

use crate::authz::{authorize, principal_from_request};
use crate::schema_service::core_to_proto_properties;
use pelago_proto::{
    watch_service_server::WatchService, CancelSubscriptionRequest, CancelSubscriptionResponse,
    ListSubscriptionsRequest, ListSubscriptionsResponse, SubscriptionInfo, WatchEvent,
    WatchEventType, WatchNamespaceRequest, WatchOptions, WatchPointRequest, WatchQueryRequest,
    WatchSubscriptionType,
};
use pelago_storage::{
    get_query_watch_state, read_cdc_entries, CdcOperation, PelagoDb, SchemaRegistry, Versionstamp,
};
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

mod dispatch;
mod filter;
mod snapshot;
mod state;

use self::dispatch::{try_dispatch_watch_event, DispatchResult};
use self::filter::{
    build_query_filter, matches_edge_scope, matches_node_scope, matches_node_update_scope,
    query_matches_node,
};
use self::snapshot::emit_initial_snapshot;
use self::state::{
    compute_query_state_id, node_key, persist_query_watch_state, should_restore_query_watch_state,
    validate_resume_position,
};

#[cfg(test)]
use self::filter::{
    build_pql_predicate, literal_to_cel, point_edge_target_matches, query_scope_matches,
    root_to_cel_expression,
};
#[cfg(test)]
use self::state::{parse_resume_after, resolve_resume_position};
#[cfg(test)]
use pelago_core::Value;
#[cfg(test)]
use pelago_proto::EdgeDirection;
#[cfg(test)]
use pelago_query::pql::{LiteralValue, RootFunction};

#[derive(Clone)]
struct ActiveSubscription {
    info: SubscriptionInfo,
    stop_tx: watch::Sender<bool>,
    principal_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct WatchRegistry {
    inner: Arc<RwLock<HashMap<String, ActiveSubscription>>>,
}

impl WatchRegistry {
    async fn try_insert(
        &self,
        id: String,
        sub: ActiveSubscription,
        limits: &WatchLimits,
    ) -> Result<(), Status> {
        let mut guard = self.inner.write().await;

        if guard.len() >= limits.max_total_subscriptions {
            return Err(Status::resource_exhausted(format!(
                "max subscriptions reached ({})",
                limits.max_total_subscriptions
            )));
        }

        let ns_count = guard
            .values()
            .filter(|s| {
                s.info.database == sub.info.database && s.info.namespace == sub.info.namespace
            })
            .count();
        if ns_count >= limits.max_namespace_subscriptions {
            return Err(Status::resource_exhausted(format!(
                "max subscriptions reached for namespace {}/{} ({})",
                sub.info.database, sub.info.namespace, limits.max_namespace_subscriptions
            )));
        }

        if sub.info.subscription_type == WatchSubscriptionType::Query as i32 {
            let ns_query_count = guard
                .values()
                .filter(|s| {
                    s.info.database == sub.info.database
                        && s.info.namespace == sub.info.namespace
                        && s.info.subscription_type == WatchSubscriptionType::Query as i32
                })
                .count();
            if ns_query_count >= limits.max_query_watches_namespace {
                return Err(Status::resource_exhausted(format!(
                    "max query watches reached for namespace {}/{} ({})",
                    sub.info.database, sub.info.namespace, limits.max_query_watches_namespace
                )));
            }
        }

        if let Some(principal_id) = &sub.principal_id {
            let principal_count = guard
                .values()
                .filter(|s| s.principal_id.as_ref() == Some(principal_id))
                .count();
            if principal_count >= limits.max_principal_subscriptions {
                return Err(Status::resource_exhausted(format!(
                    "max subscriptions reached for principal '{}' ({})",
                    principal_id, limits.max_principal_subscriptions
                )));
            }
        }

        guard.insert(id, sub);
        Ok(())
    }

    async fn remove(&self, id: &str) {
        self.inner.write().await.remove(id);
    }

    async fn list(&self) -> Vec<SubscriptionInfo> {
        self.inner
            .read()
            .await
            .values()
            .map(|s| s.info.clone())
            .collect()
    }

    async fn cancel(&self, id: &str) -> bool {
        let mut guard = self.inner.write().await;
        if let Some(sub) = guard.remove(id) {
            let _ = sub.stop_tx.send(true);
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
struct WatchLimits {
    max_total_subscriptions: usize,
    max_namespace_subscriptions: usize,
    max_query_watches_namespace: usize,
    max_principal_subscriptions: usize,
    max_ttl_secs: u32,
    default_heartbeat_secs: u32,
    max_heartbeat_secs: u32,
    max_queue_size: u32,
    max_dropped_events: u32,
}

impl Default for WatchLimits {
    fn default() -> Self {
        Self {
            max_total_subscriptions: env_usize("PELAGO_WATCH_MAX_SUBSCRIPTIONS", 1024),
            max_namespace_subscriptions: env_usize("PELAGO_WATCH_MAX_NAMESPACE_SUBSCRIPTIONS", 256),
            max_query_watches_namespace: env_usize("PELAGO_WATCH_MAX_QUERY_WATCHES", 1000),
            max_principal_subscriptions: env_usize("PELAGO_WATCH_MAX_PRINCIPAL_SUBSCRIPTIONS", 128),
            max_ttl_secs: env_u32("PELAGO_WATCH_MAX_TTL_SECS", 86_400),
            default_heartbeat_secs: env_u32("PELAGO_WATCH_HEARTBEAT_SECS", 30),
            max_heartbeat_secs: env_u32("PELAGO_WATCH_MAX_HEARTBEAT_SECS", 300),
            max_queue_size: env_u32("PELAGO_WATCH_MAX_QUEUE_SIZE", 8192),
            max_dropped_events: env_u32("PELAGO_WATCH_MAX_DROPPED_EVENTS", 512),
        }
    }
}

pub struct WatchServiceImpl {
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    registry: WatchRegistry,
    limits: WatchLimits,
}

impl WatchServiceImpl {
    pub fn new(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        registry: WatchRegistry,
    ) -> Self {
        Self {
            db,
            schema_registry,
            registry,
            limits: WatchLimits::default(),
        }
    }
}

#[tonic::async_trait]
impl WatchService for WatchServiceImpl {
    type WatchPointStream = Pin<Box<dyn Stream<Item = Result<WatchEvent, Status>> + Send>>;
    type WatchQueryStream = Pin<Box<dyn Stream<Item = Result<WatchEvent, Status>> + Send>>;
    type WatchNamespaceStream = Pin<Box<dyn Stream<Item = Result<WatchEvent, Status>> + Send>>;

    async fn watch_point(
        &self,
        request: Request<WatchPointRequest>,
    ) -> Result<Response<Self::WatchPointStream>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;

        let mut node_targets = Vec::new();
        for node in req.nodes {
            if node.entity_type.is_empty() || node.node_id.is_empty() {
                continue;
            }
            node_targets.push(PointNodeTarget {
                entity_type: node.entity_type,
                node_id: node.node_id,
                properties: node.properties.into_iter().collect(),
            });
        }
        if node_targets.is_empty() && !req.entity_type.is_empty() && !req.node_id.is_empty() {
            node_targets.push(PointNodeTarget {
                entity_type: req.entity_type.clone(),
                node_id: req.node_id.clone(),
                properties: HashSet::new(),
            });
        }

        let mut edge_targets = Vec::new();
        for edge in req.edges {
            let Some(source) = edge.source else {
                continue;
            };
            if source.entity_type.is_empty() || source.node_id.is_empty() {
                continue;
            }
            let target = edge
                .target
                .filter(|n| !n.entity_type.is_empty() && !n.node_id.is_empty())
                .map(|n| (n.entity_type, n.node_id));
            edge_targets.push(PointEdgeTarget {
                source_entity_type: source.entity_type,
                source_node_id: source.node_id,
                label: edge.label,
                target,
                direction: edge.direction,
            });
        }

        if node_targets.is_empty() && edge_targets.is_empty() {
            return Err(Status::invalid_argument(
                "watch_point requires at least one node or edge target",
            ));
        }

        let acl_entity = node_targets
            .first()
            .map(|t| t.entity_type.as_str())
            .unwrap_or("*");
        authorize(
            &self.db,
            principal.as_ref(),
            "watch.subscribe",
            &ctx.database,
            &ctx.namespace,
            acl_entity,
        )
        .await?;

        let options = normalize_options(req.options.unwrap_or_default(), &self.limits);
        let after =
            validate_resume_position(&self.db, &ctx.database, &ctx.namespace, &req.resume_after)
                .await?;

        let sub_id = Uuid::now_v7().to_string();
        let (tx, rx) = mpsc::channel(options.max_queue_size as usize);
        let (stop_tx, stop_rx) = watch::channel(false);
        let created_at = now_micros();
        let expires_at = created_at + (options.ttl_secs as i64 * 1_000_000);

        let sub = ActiveSubscription {
            info: SubscriptionInfo {
                subscription_id: sub_id.clone(),
                subscription_type: WatchSubscriptionType::Point as i32,
                database: ctx.database.clone(),
                namespace: ctx.namespace.clone(),
                created_at,
                expires_at,
            },
            stop_tx,
            principal_id: principal.as_ref().map(|p| p.principal_id.clone()),
        };
        self.registry
            .try_insert(sub_id.clone(), sub, &self.limits)
            .await?;

        spawn_watch_loop(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            self.registry.clone(),
            WatchFilter::Point {
                nodes: node_targets,
                edges: edge_targets,
            },
            WatchContext {
                subscription_id: sub_id,
                database: ctx.database,
                namespace: ctx.namespace,
                expires_at,
                after,
                max_dropped_events: self.limits.max_dropped_events,
                include_initial_snapshot: options.initial_snapshot || options.include_initial,
                heartbeat_secs: options.heartbeat_secs,
                initial_snapshot_limit: (options.max_queue_size as usize).saturating_mul(4),
                query_state_id: None,
            },
            tx,
            stop_rx,
        );

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn watch_query(
        &self,
        request: Request<WatchQueryRequest>,
    ) -> Result<Response<Self::WatchQueryStream>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "watch.subscribe",
            &ctx.database,
            &ctx.namespace,
            if req.entity_type.is_empty() {
                "*"
            } else {
                &req.entity_type
            },
        )
        .await?;

        let options = normalize_options(req.options.unwrap_or_default(), &self.limits);
        let after =
            validate_resume_position(&self.db, &ctx.database, &ctx.namespace, &req.resume_after)
                .await?;
        let query_filter =
            build_query_filter(&req.entity_type, &req.cel_expression, &req.pql_query)?;
        let query_state_id = compute_query_state_id(&ctx.database, &ctx.namespace, &query_filter);

        let sub_id = Uuid::now_v7().to_string();
        let (tx, rx) = mpsc::channel(options.max_queue_size as usize);
        let (stop_tx, stop_rx) = watch::channel(false);
        let created_at = now_micros();
        let expires_at = created_at + (options.ttl_secs as i64 * 1_000_000);

        let sub = ActiveSubscription {
            info: SubscriptionInfo {
                subscription_id: sub_id.clone(),
                subscription_type: WatchSubscriptionType::Query as i32,
                database: ctx.database.clone(),
                namespace: ctx.namespace.clone(),
                created_at,
                expires_at,
            },
            stop_tx,
            principal_id: principal.as_ref().map(|p| p.principal_id.clone()),
        };
        self.registry
            .try_insert(sub_id.clone(), sub, &self.limits)
            .await?;

        spawn_watch_loop(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            self.registry.clone(),
            WatchFilter::Query(query_filter),
            WatchContext {
                subscription_id: sub_id,
                database: ctx.database,
                namespace: ctx.namespace,
                expires_at,
                after,
                max_dropped_events: self.limits.max_dropped_events,
                include_initial_snapshot: options.initial_snapshot || options.include_initial,
                heartbeat_secs: options.heartbeat_secs,
                initial_snapshot_limit: (options.max_queue_size as usize).saturating_mul(4),
                query_state_id: Some(query_state_id),
            },
            tx,
            stop_rx,
        );

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn watch_namespace(
        &self,
        request: Request<WatchNamespaceRequest>,
    ) -> Result<Response<Self::WatchNamespaceStream>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "watch.subscribe",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;

        let options = normalize_options(req.options.unwrap_or_default(), &self.limits);
        let after =
            validate_resume_position(&self.db, &ctx.database, &ctx.namespace, &req.resume_after)
                .await?;

        let sub_id = Uuid::now_v7().to_string();
        let (tx, rx) = mpsc::channel(options.max_queue_size as usize);
        let (stop_tx, stop_rx) = watch::channel(false);
        let created_at = now_micros();
        let expires_at = created_at + (options.ttl_secs as i64 * 1_000_000);

        let sub = ActiveSubscription {
            info: SubscriptionInfo {
                subscription_id: sub_id.clone(),
                subscription_type: WatchSubscriptionType::Namespace as i32,
                database: ctx.database.clone(),
                namespace: ctx.namespace.clone(),
                created_at,
                expires_at,
            },
            stop_tx,
            principal_id: principal.as_ref().map(|p| p.principal_id.clone()),
        };
        self.registry
            .try_insert(sub_id.clone(), sub, &self.limits)
            .await?;

        spawn_watch_loop(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            self.registry.clone(),
            WatchFilter::Namespace,
            WatchContext {
                subscription_id: sub_id,
                database: ctx.database,
                namespace: ctx.namespace,
                expires_at,
                after,
                max_dropped_events: self.limits.max_dropped_events,
                include_initial_snapshot: options.initial_snapshot || options.include_initial,
                heartbeat_secs: options.heartbeat_secs,
                initial_snapshot_limit: (options.max_queue_size as usize).saturating_mul(4),
                query_state_id: None,
            },
            tx,
            stop_rx,
        );

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn list_subscriptions(
        &self,
        request: Request<ListSubscriptionsRequest>,
    ) -> Result<Response<ListSubscriptionsResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "watch.list",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        Ok(Response::new(ListSubscriptionsResponse {
            subscriptions: self.registry.list().await,
        }))
    }

    async fn cancel_subscription(
        &self,
        request: Request<CancelSubscriptionRequest>,
    ) -> Result<Response<CancelSubscriptionResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "watch.cancel",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let cancelled = self.registry.cancel(&req.subscription_id).await;
        Ok(Response::new(CancelSubscriptionResponse { cancelled }))
    }
}

#[derive(Clone)]
struct WatchContext {
    subscription_id: String,
    database: String,
    namespace: String,
    expires_at: i64,
    after: Versionstamp,
    max_dropped_events: u32,
    include_initial_snapshot: bool,
    heartbeat_secs: u32,
    initial_snapshot_limit: usize,
    query_state_id: Option<String>,
}

#[derive(Clone)]
enum WatchFilter {
    Point {
        nodes: Vec<PointNodeTarget>,
        edges: Vec<PointEdgeTarget>,
    },
    Query(QueryFilter),
    Namespace,
}

#[derive(Clone)]
struct PointNodeTarget {
    entity_type: String,
    node_id: String,
    properties: HashSet<String>,
}

#[derive(Clone)]
struct PointEdgeTarget {
    source_entity_type: String,
    source_node_id: String,
    label: String,
    target: Option<(String, String)>,
    direction: i32,
}

#[derive(Clone, Default)]
struct QueryFilter {
    entity_type: String,
    cel_expression: Option<String>,
    pql_predicate: Option<PqlPredicate>,
}

#[derive(Clone, Default)]
struct PqlPredicate {
    root_entity_type: Option<String>,
    cel_expression: Option<String>,
}

fn spawn_watch_loop(
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    registry: WatchRegistry,
    filter: WatchFilter,
    mut ctx: WatchContext,
    tx: mpsc::Sender<Result<WatchEvent, Status>>,
    stop_rx: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let mut matched_nodes: HashSet<String> = HashSet::new();
        let mut dropped_events: u32 = 0;
        let mut last_heartbeat_at = now_micros();

        if should_restore_query_watch_state(&ctx, &filter) {
            if let Some(state_id) = ctx.query_state_id.as_deref() {
                if let Ok(Some(saved)) =
                    get_query_watch_state(&db, &ctx.database, &ctx.namespace, state_id).await
                {
                    if saved.last_position <= ctx.after {
                        matched_nodes = saved.matched_nodes.into_iter().collect();
                    }
                }
            }
        }

        if ctx.include_initial_snapshot {
            if let Err(err) = emit_initial_snapshot(
                &db,
                &schema_registry,
                &ctx,
                &filter,
                &tx,
                &mut matched_nodes,
            )
            .await
            {
                let _ = tx.send(Err(err)).await;
                let _ = persist_query_watch_state(&db, &ctx, &filter, &matched_nodes).await;
                registry.remove(&ctx.subscription_id).await;
                return;
            }
        }

        'outer: loop {
            if *stop_rx.borrow() || now_micros() >= ctx.expires_at {
                let _ = persist_query_watch_state(&db, &ctx, &filter, &matched_nodes).await;
                registry.remove(&ctx.subscription_id).await;
                break;
            }
            if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow() {
                let _ = persist_query_watch_state(&db, &ctx, &filter, &matched_nodes).await;
                registry.remove(&ctx.subscription_id).await;
                break;
            }

            let now = now_micros();
            if now.saturating_sub(last_heartbeat_at) >= (ctx.heartbeat_secs as i64 * 1_000_000) {
                let heartbeat = WatchEvent {
                    subscription_id: ctx.subscription_id.clone(),
                    r#type: WatchEventType::Heartbeat as i32,
                    versionstamp: ctx.after.to_bytes().to_vec(),
                    node: None,
                    edge: None,
                    reason: "heartbeat".to_string(),
                };
                let _ = tx.try_send(Ok(heartbeat));
                last_heartbeat_at = now;
            }

            match read_cdc_entries(&db, &ctx.database, &ctx.namespace, Some(&ctx.after), 256).await
            {
                Ok(entries) if entries.is_empty() => {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                Ok(entries) => {
                    for (vs, entry) in entries {
                        ctx.after = vs.clone();
                        for op in entry.operations {
                            let events = cdc_op_to_watch_events(
                                &ctx.subscription_id,
                                &vs,
                                op,
                                &filter,
                                &ctx.database,
                                &ctx.namespace,
                                &schema_registry,
                                &mut matched_nodes,
                            )
                            .await;

                            for event in events {
                                match try_dispatch_watch_event(
                                    &tx,
                                    event,
                                    &mut dropped_events,
                                    ctx.max_dropped_events,
                                ) {
                                    DispatchResult::Sent | DispatchResult::Backpressured => {}
                                    DispatchResult::DropSubscription(status) => {
                                        let _ = tx.send(Err(status)).await;
                                        let _ = persist_query_watch_state(
                                            &db,
                                            &ctx,
                                            &filter,
                                            &matched_nodes,
                                        )
                                        .await;
                                        registry.remove(&ctx.subscription_id).await;
                                        break 'outer;
                                    }
                                    DispatchResult::ConsumerClosed => {
                                        let _ = persist_query_watch_state(
                                            &db,
                                            &ctx,
                                            &filter,
                                            &matched_nodes,
                                        )
                                        .await;
                                        registry.remove(&ctx.subscription_id).await;
                                        break 'outer;
                                    }
                                }
                            }
                        }
                    }
                    let _ = persist_query_watch_state(&db, &ctx, &filter, &matched_nodes).await;
                }
                Err(err) => {
                    let _ = tx
                        .send(Err(Status::internal(format!(
                            "watch dispatcher failed: {}",
                            err
                        ))))
                        .await;
                    let _ = persist_query_watch_state(&db, &ctx, &filter, &matched_nodes).await;
                    registry.remove(&ctx.subscription_id).await;
                    break;
                }
            }
        }
    });
}

async fn cdc_op_to_watch_events(
    sub_id: &str,
    versionstamp: &Versionstamp,
    op: CdcOperation,
    filter: &WatchFilter,
    database: &str,
    namespace: &str,
    schema_registry: &Arc<SchemaRegistry>,
    matched_nodes: &mut HashSet<String>,
) -> Vec<WatchEvent> {
    use pelago_storage::CdcOperation::*;

    match op {
        NodeCreate {
            entity_type,
            node_id,
            properties,
            home_site,
        } => {
            if !matches_node_scope(filter, &entity_type, &node_id) {
                return Vec::new();
            }

            if let WatchFilter::Query(query_filter) = filter {
                let matched = query_matches_node(
                    query_filter,
                    database,
                    namespace,
                    &entity_type,
                    &properties,
                    schema_registry,
                )
                .await;
                if !matched {
                    return Vec::new();
                }
                matched_nodes.insert(node_key(&entity_type, &node_id));
            }

            vec![WatchEvent {
                subscription_id: sub_id.to_string(),
                r#type: WatchEventType::Enter as i32,
                versionstamp: versionstamp.to_bytes().to_vec(),
                node: Some(pelago_proto::Node {
                    id: node_id,
                    entity_type,
                    properties: core_to_proto_properties(&properties),
                    locality: home_site,
                    created_at: 0,
                    updated_at: 0,
                }),
                edge: None,
                reason: "node_create".to_string(),
            }]
        }
        NodeUpdate {
            entity_type,
            node_id,
            changed_properties,
            old_properties,
        } => {
            if !matches_node_update_scope(filter, &entity_type, &node_id, &changed_properties) {
                return Vec::new();
            }

            let mut event_type = WatchEventType::Update;
            let mut should_emit = true;
            if let WatchFilter::Query(query_filter) = filter {
                let old_match = query_matches_node(
                    query_filter,
                    database,
                    namespace,
                    &entity_type,
                    &old_properties,
                    schema_registry,
                )
                .await;
                let mut new_properties = old_properties.clone();
                for (k, v) in &changed_properties {
                    new_properties.insert(k.clone(), v.clone());
                }
                let new_match = query_matches_node(
                    query_filter,
                    database,
                    namespace,
                    &entity_type,
                    &new_properties,
                    schema_registry,
                )
                .await;

                let key = node_key(&entity_type, &node_id);
                let was_tracked = matched_nodes.contains(&key);
                let was_match = was_tracked || old_match;

                match (was_match, new_match) {
                    (false, true) => {
                        event_type = WatchEventType::Enter;
                        matched_nodes.insert(key);
                    }
                    (true, true) => {
                        event_type = WatchEventType::Update;
                        matched_nodes.insert(key);
                    }
                    (true, false) => {
                        event_type = WatchEventType::Exit;
                        matched_nodes.remove(&key);
                    }
                    (false, false) => {
                        should_emit = false;
                    }
                }
            }

            if !should_emit {
                return Vec::new();
            }

            vec![WatchEvent {
                subscription_id: sub_id.to_string(),
                r#type: event_type as i32,
                versionstamp: versionstamp.to_bytes().to_vec(),
                node: Some(pelago_proto::Node {
                    id: node_id,
                    entity_type,
                    properties: core_to_proto_properties(&changed_properties),
                    locality: String::new(),
                    created_at: 0,
                    updated_at: 0,
                }),
                edge: None,
                reason: "node_update".to_string(),
            }]
        }
        NodeDelete {
            entity_type,
            node_id,
        } => {
            if !matches_node_scope(filter, &entity_type, &node_id) {
                return Vec::new();
            }

            if let WatchFilter::Query(_) = filter {
                let key = node_key(&entity_type, &node_id);
                if !matched_nodes.remove(&key) {
                    return Vec::new();
                }
            }

            vec![WatchEvent {
                subscription_id: sub_id.to_string(),
                r#type: WatchEventType::Delete as i32,
                versionstamp: versionstamp.to_bytes().to_vec(),
                node: Some(pelago_proto::Node {
                    id: node_id,
                    entity_type,
                    properties: HashMap::new(),
                    locality: String::new(),
                    created_at: 0,
                    updated_at: 0,
                }),
                edge: None,
                reason: "node_delete".to_string(),
            }]
        }
        EdgeCreate {
            source_type,
            source_id,
            target_type,
            target_id,
            edge_type,
            edge_id,
            properties,
        } => {
            if !matches_edge_scope(
                filter,
                &source_type,
                &source_id,
                &target_type,
                &target_id,
                &edge_type,
            ) {
                return Vec::new();
            }

            if let WatchFilter::Query(_) = filter {
                // For query subscriptions, only emit edge events if at least one endpoint
                // is currently in the matched node set.
                let src_key = node_key(&source_type, &source_id);
                let tgt_key = node_key(&target_type, &target_id);
                if !matched_nodes.contains(&src_key) && !matched_nodes.contains(&tgt_key) {
                    return Vec::new();
                }
            }

            vec![WatchEvent {
                subscription_id: sub_id.to_string(),
                r#type: WatchEventType::Update as i32,
                versionstamp: versionstamp.to_bytes().to_vec(),
                node: None,
                edge: Some(pelago_proto::Edge {
                    edge_id,
                    source: Some(pelago_proto::NodeRef {
                        entity_type: source_type,
                        node_id: source_id,
                        database: String::new(),
                        namespace: String::new(),
                    }),
                    target: Some(pelago_proto::NodeRef {
                        entity_type: target_type,
                        node_id: target_id,
                        database: String::new(),
                        namespace: String::new(),
                    }),
                    label: edge_type,
                    properties: core_to_proto_properties(&properties),
                    created_at: 0,
                }),
                reason: "edge_create".to_string(),
            }]
        }
        EdgeDelete {
            source_type,
            source_id,
            target_type,
            target_id,
            edge_type,
        } => {
            if !matches_edge_scope(
                filter,
                &source_type,
                &source_id,
                &target_type,
                &target_id,
                &edge_type,
            ) {
                return Vec::new();
            }

            if let WatchFilter::Query(_) = filter {
                let src_key = node_key(&source_type, &source_id);
                let tgt_key = node_key(&target_type, &target_id);
                if !matched_nodes.contains(&src_key) && !matched_nodes.contains(&tgt_key) {
                    return Vec::new();
                }
            }

            vec![WatchEvent {
                subscription_id: sub_id.to_string(),
                r#type: WatchEventType::Delete as i32,
                versionstamp: versionstamp.to_bytes().to_vec(),
                node: None,
                edge: Some(pelago_proto::Edge {
                    edge_id: String::new(),
                    source: Some(pelago_proto::NodeRef {
                        entity_type: source_type,
                        node_id: source_id,
                        database: String::new(),
                        namespace: String::new(),
                    }),
                    target: Some(pelago_proto::NodeRef {
                        entity_type: target_type,
                        node_id: target_id,
                        database: String::new(),
                        namespace: String::new(),
                    }),
                    label: edge_type,
                    properties: HashMap::new(),
                    created_at: 0,
                }),
                reason: "edge_delete".to_string(),
            }]
        }
        SchemaRegister { .. } => Vec::new(),
        OwnershipTransfer {
            entity_type,
            node_id,
            current_site_id,
            ..
        } => {
            if !matches_node_scope(filter, &entity_type, &node_id) {
                return Vec::new();
            }

            if let WatchFilter::Query(_) = filter {
                let key = node_key(&entity_type, &node_id);
                if !matched_nodes.contains(&key) {
                    return Vec::new();
                }
            }

            vec![WatchEvent {
                subscription_id: sub_id.to_string(),
                r#type: WatchEventType::Update as i32,
                versionstamp: versionstamp.to_bytes().to_vec(),
                node: Some(pelago_proto::Node {
                    id: node_id,
                    entity_type,
                    properties: HashMap::new(),
                    locality: current_site_id,
                    created_at: 0,
                    updated_at: 0,
                }),
                edge: None,
                reason: "ownership_transfer".to_string(),
            }]
        }
    }
}

fn normalize_options(mut options: WatchOptions, limits: &WatchLimits) -> WatchOptions {
    if options.ttl_secs == 0 {
        options.ttl_secs = 60;
    }
    options.ttl_secs = options.ttl_secs.min(limits.max_ttl_secs).max(60);

    if options.heartbeat_secs == 0 {
        options.heartbeat_secs = limits.default_heartbeat_secs.max(5);
    }
    options.heartbeat_secs = options.heartbeat_secs.min(limits.max_heartbeat_secs).max(5);

    if options.max_queue_size == 0 {
        options.max_queue_size = 256;
    }
    options.max_queue_size = options.max_queue_size.min(limits.max_queue_size).max(32);

    if options.initial_snapshot {
        options.include_initial = true;
    }

    options
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(default)
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_pql_predicate_extracts_root_type_and_filter() {
        let pred = build_pql_predicate("User @filter(age >= 30) { name }").unwrap();
        assert_eq!(pred.root_entity_type.as_deref(), Some("User"));
        assert_eq!(pred.cel_expression.as_deref(), Some("age >= 30"));
    }

    #[test]
    fn test_build_pql_predicate_root_function_to_cel() {
        let pred = build_pql_predicate("query { start(func: eq(age, 42)) { uid } }").unwrap();
        assert_eq!(pred.cel_expression.as_deref(), Some("age == 42"));
    }

    #[test]
    fn test_matches_node_point() {
        let filter = WatchFilter::Point {
            nodes: vec![PointNodeTarget {
                entity_type: "Person".to_string(),
                node_id: "1_7".to_string(),
                properties: HashSet::new(),
            }],
            edges: Vec::new(),
        };
        assert!(matches_node_scope(&filter, "Person", "1_7"));
        assert!(!matches_node_scope(&filter, "Person", "1_8"));
        assert!(!matches_node_scope(&filter, "Company", "1_7"));
    }

    #[test]
    fn test_normalize_options_caps_values() {
        let limits = WatchLimits {
            max_total_subscriptions: 10,
            max_namespace_subscriptions: 10,
            max_query_watches_namespace: 10,
            max_principal_subscriptions: 10,
            max_ttl_secs: 120,
            default_heartbeat_secs: 30,
            max_heartbeat_secs: 90,
            max_queue_size: 64,
            max_dropped_events: 10,
        };
        let opts = normalize_options(
            WatchOptions {
                include_initial: false,
                ttl_secs: 500,
                max_queue_size: 999,
                heartbeat_secs: 999,
                initial_snapshot: false,
            },
            &limits,
        );
        assert_eq!(opts.ttl_secs, 120);
        assert_eq!(opts.max_queue_size, 64);
        assert_eq!(opts.heartbeat_secs, 90);
    }

    #[tokio::test]
    async fn test_query_watch_limit_enforced_per_namespace() {
        let registry = WatchRegistry::default();
        let limits = WatchLimits {
            max_total_subscriptions: 10,
            max_namespace_subscriptions: 10,
            max_query_watches_namespace: 1,
            max_principal_subscriptions: 10,
            max_ttl_secs: 120,
            default_heartbeat_secs: 30,
            max_heartbeat_secs: 90,
            max_queue_size: 64,
            max_dropped_events: 10,
        };

        let (stop_tx_1, _stop_rx_1) = watch::channel(false);
        let first = ActiveSubscription {
            info: SubscriptionInfo {
                subscription_id: "s1".to_string(),
                subscription_type: WatchSubscriptionType::Query as i32,
                database: "db".to_string(),
                namespace: "ns".to_string(),
                created_at: 1,
                expires_at: 2,
            },
            stop_tx: stop_tx_1,
            principal_id: Some("p1".to_string()),
        };
        registry
            .try_insert("s1".to_string(), first, &limits)
            .await
            .expect("first query watch should succeed");

        let (stop_tx_2, _stop_rx_2) = watch::channel(false);
        let second = ActiveSubscription {
            info: SubscriptionInfo {
                subscription_id: "s2".to_string(),
                subscription_type: WatchSubscriptionType::Query as i32,
                database: "db".to_string(),
                namespace: "ns".to_string(),
                created_at: 1,
                expires_at: 2,
            },
            stop_tx: stop_tx_2,
            principal_id: Some("p2".to_string()),
        };
        let err = registry
            .try_insert("s2".to_string(), second, &limits)
            .await
            .expect_err("second query watch should be rejected");
        assert_eq!(err.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn test_literal_to_cel_string_escape() {
        let expr = literal_to_cel(&pelago_query::pql::LiteralValue::String(
            "O'Reilly".to_string(),
        ));
        assert_eq!(expr, "'O\\'Reilly'");
    }

    #[test]
    fn test_root_to_cel_between() {
        let expr = root_to_cel_expression(&RootFunction::Between(
            "age".to_string(),
            LiteralValue::Int(1),
            LiteralValue::Int(2),
        ))
        .unwrap();
        assert_eq!(expr, "(age >= 1) && (age <= 2)");
    }

    #[test]
    fn test_point_edge_target_outgoing_match() {
        let target = PointEdgeTarget {
            source_entity_type: "Person".to_string(),
            source_node_id: "1_1".to_string(),
            label: "KNOWS".to_string(),
            target: Some(("Person".to_string(), "1_2".to_string())),
            direction: EdgeDirection::Outgoing as i32,
        };
        assert!(point_edge_target_matches(
            &target, "Person", "1_1", "Person", "1_2", "KNOWS"
        ));
        assert!(!point_edge_target_matches(
            &target, "Person", "1_2", "Person", "1_1", "KNOWS"
        ));
    }

    #[test]
    fn test_matches_node_update_scope_property_filter() {
        let mut properties = HashSet::new();
        properties.insert("email".to_string());
        let filter = WatchFilter::Point {
            nodes: vec![PointNodeTarget {
                entity_type: "Person".to_string(),
                node_id: "1_7".to_string(),
                properties,
            }],
            edges: Vec::new(),
        };
        let mut changed = HashMap::new();
        changed.insert("email".to_string(), Value::String("a@b.com".to_string()));
        assert!(matches_node_update_scope(
            &filter, "Person", "1_7", &changed
        ));
        let mut changed_other = HashMap::new();
        changed_other.insert("name".to_string(), Value::String("Alice".to_string()));
        assert!(!matches_node_update_scope(
            &filter,
            "Person",
            "1_7",
            &changed_other
        ));
    }

    #[test]
    fn test_query_scope_matches_root_and_request_type() {
        let filter = QueryFilter {
            entity_type: "Person".to_string(),
            cel_expression: None,
            pql_predicate: Some(PqlPredicate {
                root_entity_type: Some("Person".to_string()),
                cel_expression: None,
            }),
        };
        assert!(query_scope_matches(&filter, "Person"));
        assert!(!query_scope_matches(&filter, "Company"));
    }

    #[test]
    fn test_parse_resume_after_empty_defaults_to_zero_position() {
        let parsed = parse_resume_after(&[]).unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn test_parse_resume_after_rejects_invalid_bytes() {
        let err = parse_resume_after(&[1, 2, 3]).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn test_resolve_resume_position_reports_out_of_range_for_expired_position() {
        let err = resolve_resume_position(
            Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 1]).unwrap(),
            false,
            Some(Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 2]).unwrap()),
        )
        .unwrap_err();
        assert_eq!(err.code(), tonic::Code::OutOfRange);
    }

    #[test]
    fn test_resolve_resume_position_starts_from_now_when_missing_but_not_expired() {
        let resolved = resolve_resume_position(
            Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 3]).unwrap(),
            false,
            Some(Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 2]).unwrap()),
        )
        .unwrap();
        assert!(resolved.is_zero());
    }

    #[test]
    fn test_backpressure_dispatch_drops_subscription_after_threshold() {
        let (tx, mut rx) = mpsc::channel(1);
        let seed = sample_watch_event();
        tx.try_send(Ok(seed)).expect("seed send should fill queue");

        let mut dropped = 0;
        let first = try_dispatch_watch_event(&tx, sample_watch_event(), &mut dropped, 2);
        assert!(matches!(first, DispatchResult::Backpressured));
        assert_eq!(dropped, 1);

        let second = try_dispatch_watch_event(&tx, sample_watch_event(), &mut dropped, 2);
        match second {
            DispatchResult::DropSubscription(status) => {
                assert_eq!(status.code(), tonic::Code::ResourceExhausted);
            }
            other => panic!("expected drop-subscription result, got {other:?}"),
        }

        // Drain the seeded item so sender doesn't remain permanently full in this test.
        let _ = rx.try_recv();
    }

    #[test]
    fn test_backpressure_dispatch_detects_closed_consumer() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let mut dropped = 0;
        let result = try_dispatch_watch_event(&tx, sample_watch_event(), &mut dropped, 1);
        assert!(matches!(result, DispatchResult::ConsumerClosed));
    }

    #[test]
    fn test_compute_query_state_id_is_stable_for_same_filter() {
        let filter = QueryFilter {
            entity_type: "Person".to_string(),
            cel_expression: Some("age >= 30".to_string()),
            pql_predicate: Some(PqlPredicate {
                root_entity_type: Some("Person".to_string()),
                cel_expression: Some("active == true".to_string()),
            }),
        };
        let a = compute_query_state_id("db", "ns", &filter);
        let b = compute_query_state_id("db", "ns", &filter);
        assert_eq!(a, b);
    }

    #[test]
    fn test_compute_query_state_id_changes_with_filter_inputs() {
        let filter_a = QueryFilter {
            entity_type: "Person".to_string(),
            cel_expression: Some("age >= 30".to_string()),
            pql_predicate: None,
        };
        let filter_b = QueryFilter {
            entity_type: "Person".to_string(),
            cel_expression: Some("age >= 31".to_string()),
            pql_predicate: None,
        };
        let a = compute_query_state_id("db", "ns", &filter_a);
        let b = compute_query_state_id("db", "ns", &filter_b);
        assert_ne!(a, b);
    }

    #[test]
    fn test_should_restore_query_watch_state_only_on_query_resume() {
        let query_filter = WatchFilter::Query(QueryFilter::default());
        let ns_filter = WatchFilter::Namespace;

        let base_ctx = WatchContext {
            subscription_id: "s".to_string(),
            database: "db".to_string(),
            namespace: "ns".to_string(),
            expires_at: 1,
            after: Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 1]).unwrap(),
            max_dropped_events: 10,
            include_initial_snapshot: false,
            heartbeat_secs: 30,
            initial_snapshot_limit: 100,
            query_state_id: Some("qws-1".to_string()),
        };
        assert!(should_restore_query_watch_state(&base_ctx, &query_filter));
        assert!(!should_restore_query_watch_state(&base_ctx, &ns_filter));

        let with_initial = WatchContext {
            include_initial_snapshot: true,
            ..base_ctx.clone()
        };
        assert!(!should_restore_query_watch_state(
            &with_initial,
            &query_filter
        ));

        let from_now = WatchContext {
            after: Versionstamp::zero(),
            ..base_ctx.clone()
        };
        assert!(!should_restore_query_watch_state(&from_now, &query_filter));

        let missing_state = WatchContext {
            query_state_id: None,
            ..base_ctx
        };
        assert!(!should_restore_query_watch_state(
            &missing_state,
            &query_filter
        ));
    }

    fn sample_watch_event() -> WatchEvent {
        WatchEvent {
            subscription_id: "s".to_string(),
            r#type: WatchEventType::Update as i32,
            versionstamp: Versionstamp::zero().to_bytes().to_vec(),
            node: None,
            edge: None,
            reason: "test".to_string(),
        }
    }
}
