//! WatchService gRPC handler.

use crate::authz::{authorize, principal_from_request};
use crate::schema_service::core_to_proto_properties;
use pelago_core::Value;
use pelago_proto::{
    watch_service_server::WatchService, CancelSubscriptionRequest, CancelSubscriptionResponse,
    ListSubscriptionsRequest, ListSubscriptionsResponse, SubscriptionInfo, WatchEvent,
    WatchEventType, WatchNamespaceRequest, WatchOptions, WatchPointRequest, WatchQueryRequest,
    WatchSubscriptionType,
};
use pelago_query::cel::CelEnvironment;
use pelago_query::pql::{parse_pql, Directive, LiteralValue, RootFunction};
use pelago_storage::{
    cdc_position_exists, read_cdc_entries, CdcOperation, PelagoDb, SchemaRegistry, Versionstamp,
};
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

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
        authorize(
            &self.db,
            principal.as_ref(),
            "watch.subscribe",
            &ctx.database,
            &ctx.namespace,
            &req.entity_type,
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
                entity_type: req.entity_type,
                node_id: req.node_id,
            },
            WatchContext {
                subscription_id: sub_id,
                database: ctx.database,
                namespace: ctx.namespace,
                expires_at,
                after,
                max_dropped_events: self.limits.max_dropped_events,
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
}

#[derive(Clone)]
enum WatchFilter {
    Point {
        entity_type: String,
        node_id: String,
    },
    Query(QueryFilter),
    Namespace,
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

        'outer: loop {
            if *stop_rx.borrow() || now_micros() >= ctx.expires_at {
                registry.remove(&ctx.subscription_id).await;
                break;
            }
            if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow() {
                registry.remove(&ctx.subscription_id).await;
                break;
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
                                match tx.try_send(Ok(event)) {
                                    Ok(_) => {
                                        dropped_events = 0;
                                    }
                                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                        dropped_events += 1;
                                        if dropped_events >= ctx.max_dropped_events {
                                            let _ = tx
                                                .send(Err(Status::resource_exhausted(
                                                    "watch consumer is too slow; subscription dropped",
                                                )))
                                                .await;
                                            registry.remove(&ctx.subscription_id).await;
                                            break 'outer;
                                        }
                                    }
                                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                        registry.remove(&ctx.subscription_id).await;
                                        break 'outer;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    let _ = tx
                        .send(Err(Status::internal(format!(
                            "watch dispatcher failed: {}",
                            err
                        ))))
                        .await;
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
            if !matches_node_scope(filter, &entity_type, &node_id) {
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
            if !matches_edge_scope(filter, &source_type, &source_id, &target_type, &target_id) {
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
            if !matches_edge_scope(filter, &source_type, &source_id, &target_type, &target_id) {
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

fn matches_node_scope(filter: &WatchFilter, entity_type: &str, node_id: &str) -> bool {
    match filter {
        WatchFilter::Namespace => true,
        WatchFilter::Point {
            entity_type: et,
            node_id: id,
        } => et == entity_type && id == node_id,
        WatchFilter::Query(q) => query_scope_matches(q, entity_type),
    }
}

fn matches_edge_scope(
    filter: &WatchFilter,
    source_type: &str,
    source_id: &str,
    target_type: &str,
    target_id: &str,
) -> bool {
    match filter {
        WatchFilter::Namespace => true,
        WatchFilter::Point {
            entity_type,
            node_id,
        } => {
            (entity_type == source_type && node_id == source_id)
                || (entity_type == target_type && node_id == target_id)
        }
        WatchFilter::Query(q) => {
            query_scope_matches(q, source_type) || query_scope_matches(q, target_type)
        }
    }
}

fn query_scope_matches(filter: &QueryFilter, entity_type: &str) -> bool {
    if !filter.entity_type.is_empty() && filter.entity_type != entity_type {
        return false;
    }
    if let Some(pql) = &filter.pql_predicate {
        if let Some(root_et) = &pql.root_entity_type {
            if root_et != entity_type {
                return false;
            }
        }
    }
    true
}

async fn query_matches_node(
    filter: &QueryFilter,
    database: &str,
    namespace: &str,
    entity_type: &str,
    properties: &HashMap<String, Value>,
    schema_registry: &Arc<SchemaRegistry>,
) -> bool {
    if !query_scope_matches(filter, entity_type) {
        return false;
    }

    if let Some(cel) = &filter.cel_expression {
        if !evaluate_cel(
            schema_registry,
            database,
            namespace,
            entity_type,
            cel,
            properties,
        )
        .await
        {
            return false;
        }
    }

    if let Some(pql) = &filter.pql_predicate {
        if let Some(cel) = &pql.cel_expression {
            if !evaluate_cel(
                schema_registry,
                database,
                namespace,
                entity_type,
                cel,
                properties,
            )
            .await
            {
                return false;
            }
        }
    }

    true
}

async fn evaluate_cel(
    schema_registry: &Arc<SchemaRegistry>,
    database: &str,
    namespace: &str,
    entity_type: &str,
    cel_expression: &str,
    properties: &HashMap<String, Value>,
) -> bool {
    if cel_expression.trim().is_empty() {
        return true;
    }

    let schema = match schema_registry
        .get_schema(database, namespace, entity_type)
        .await
    {
        Ok(Some(schema)) => schema,
        _ => return false,
    };

    let env = CelEnvironment::new(schema);
    let compiled = match env.compile(cel_expression) {
        Ok(compiled) => compiled,
        Err(_) => return false,
    };
    compiled.evaluate(properties).unwrap_or(false)
}

fn build_query_filter(
    entity_type: &str,
    cel_expression: &str,
    pql_query: &str,
) -> Result<QueryFilter, Status> {
    let cel_expression = if cel_expression.trim().is_empty() {
        None
    } else {
        Some(cel_expression.trim().to_string())
    };

    let pql_predicate = if pql_query.trim().is_empty() {
        None
    } else {
        Some(build_pql_predicate(pql_query.trim())?)
    };

    Ok(QueryFilter {
        entity_type: entity_type.to_string(),
        cel_expression,
        pql_predicate,
    })
}

fn build_pql_predicate(query: &str) -> Result<PqlPredicate, Status> {
    let ast = parse_pql(query).map_err(|e| Status::invalid_argument(e.to_string()))?;
    let block = ast.blocks.first().ok_or_else(|| {
        Status::invalid_argument("PQL watch query must include at least one block")
    })?;

    let root_entity_type = root_entity_type(&block.root);
    let mut filters = Vec::new();
    if let Some(root_expr) = root_to_cel_expression(&block.root) {
        filters.push(root_expr);
    }
    for directive in &block.directives {
        if let Directive::Filter(expr) = directive {
            if !expr.trim().is_empty() {
                filters.push(expr.trim().to_string());
            }
        }
    }

    let cel_expression = if filters.is_empty() {
        None
    } else {
        Some(filters.join(" && "))
    };

    Ok(PqlPredicate {
        root_entity_type,
        cel_expression,
    })
}

fn root_entity_type(root: &RootFunction) -> Option<String> {
    match root {
        RootFunction::Type(qt) => Some(qt.entity_type.clone()),
        RootFunction::Uid(qr) => Some(qr.entity_type.clone()),
        _ => None,
    }
}

fn root_to_cel_expression(root: &RootFunction) -> Option<String> {
    match root {
        RootFunction::Eq(field, value) => Some(format!("{} == {}", field, literal_to_cel(value))),
        RootFunction::Ge(field, value) => Some(format!("{} >= {}", field, literal_to_cel(value))),
        RootFunction::Le(field, value) => Some(format!("{} <= {}", field, literal_to_cel(value))),
        RootFunction::Gt(field, value) => Some(format!("{} > {}", field, literal_to_cel(value))),
        RootFunction::Lt(field, value) => Some(format!("{} < {}", field, literal_to_cel(value))),
        RootFunction::Between(field, lo, hi) => Some(format!(
            "({} >= {}) && ({} <= {})",
            field,
            literal_to_cel(lo),
            field,
            literal_to_cel(hi)
        )),
        RootFunction::Has(field) => Some(format!("{} != null", field)),
        _ => None,
    }
}

fn literal_to_cel(value: &LiteralValue) -> String {
    match value {
        LiteralValue::String(s) => format!("'{}'", s.replace('\'', "\\'")),
        LiteralValue::Int(i) => i.to_string(),
        LiteralValue::Float(f) => {
            if f.fract() == 0.0 {
                format!("{:.1}", f)
            } else {
                f.to_string()
            }
        }
        LiteralValue::Bool(b) => b.to_string(),
    }
}

fn normalize_options(mut options: WatchOptions, limits: &WatchLimits) -> WatchOptions {
    if options.ttl_secs == 0 {
        options.ttl_secs = 60;
    }
    options.ttl_secs = options.ttl_secs.min(limits.max_ttl_secs).max(60);

    if options.max_queue_size == 0 {
        options.max_queue_size = 256;
    }
    options.max_queue_size = options.max_queue_size.min(limits.max_queue_size).max(32);

    options
}

async fn validate_resume_position(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    resume_after: &[u8],
) -> Result<Versionstamp, Status> {
    if resume_after.is_empty() {
        return Ok(Versionstamp::zero());
    }

    let vs = Versionstamp::from_bytes(resume_after)
        .ok_or_else(|| Status::invalid_argument("resume_after must be a 10-byte versionstamp"))?;

    let exists = cdc_position_exists(db, database, namespace, &vs)
        .await
        .map_err(|e| Status::internal(format!("resume validation failed: {}", e)))?;
    if !exists {
        return Err(Status::out_of_range(
            "resume_after versionstamp does not exist in CDC log",
        ));
    }

    Ok(vs)
}

fn node_key(entity_type: &str, node_id: &str) -> String {
    format!("{}:{}", entity_type, node_id)
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
            entity_type: "Person".to_string(),
            node_id: "1_7".to_string(),
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
            max_queue_size: 64,
            max_dropped_events: 10,
        };
        let opts = normalize_options(
            WatchOptions {
                include_initial: false,
                ttl_secs: 500,
                max_queue_size: 999,
            },
            &limits,
        );
        assert_eq!(opts.ttl_secs, 120);
        assert_eq!(opts.max_queue_size, 64);
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
}
