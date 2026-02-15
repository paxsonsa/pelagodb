//! WatchService gRPC handler
//!
//! Implements the Phase 4 reactive subscription endpoints:
//! - WatchPoint: subscribe to specific node/edge changes
//! - WatchQuery: subscribe to CEL/PQL query result changes
//! - WatchNamespace: subscribe to all namespace changes
//! - ListSubscriptions: list active subscriptions for a connection
//! - CancelSubscription: cancel a subscription by ID

use crate::error::IntoStatus;
use pelago_core::Value;
use pelago_proto::{
    watch_service_server::WatchService, CancelSubscriptionRequest, CancelSubscriptionResponse,
    ListSubscriptionsRequest, ListSubscriptionsResponse, WatchEvent, WatchNamespaceRequest,
    WatchPointRequest, WatchQueryRequest,
};
use pelago_storage::watch::subscriptions::{
    NamespaceWatchSubscription, PointWatchSubscription, QueryWatchSubscription,
};
use pelago_storage::watch::{validate_resume_position, ResumeStatus};
use pelago_storage::{SchemaRegistry, SubscriptionRegistry, Versionstamp};
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};

pub struct WatchServiceImpl {
    registry: Arc<SubscriptionRegistry>,
    schema_registry: Arc<SchemaRegistry>,
}

impl WatchServiceImpl {
    pub fn new(
        registry: Arc<SubscriptionRegistry>,
        schema_registry: Arc<SchemaRegistry>,
    ) -> Self {
        Self {
            registry,
            schema_registry,
        }
    }
}

#[tonic::async_trait]
impl WatchService for WatchServiceImpl {
    type WatchPointStream = Pin<Box<dyn Stream<Item = Result<WatchEvent, Status>> + Send>>;

    async fn watch_point(
        &self,
        request: Request<WatchPointRequest>,
    ) -> Result<Response<Self::WatchPointStream>, Status> {
        let connection_id = extract_connection_id(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;

        if req.nodes.is_empty() && req.edges.is_empty() {
            return Err(Status::invalid_argument(
                "at least one node or edge watch required",
            ));
        }

        // Validate resume position if provided
        if !req.resume_position.is_empty() {
            validate_resume(&self.registry, &ctx.database, &ctx.namespace, &req.resume_position)
                .await?;
        }

        let ttl = self.registry.compute_ttl(
            req.options
                .as_ref()
                .map(|o| o.ttl_seconds)
                .unwrap_or(0),
        );

        let sub_id = format!("sub_{}", uuid::Uuid::now_v7());

        // Build subscription
        let watched_nodes: HashSet<(String, String)> = req
            .nodes
            .iter()
            .map(|n| (n.entity_type.clone(), n.node_id.clone()))
            .collect();

        let watched_edges: HashSet<(String, String, String)> = req
            .edges
            .iter()
            .filter_map(|e| {
                let source = e.source.as_ref()?;
                Some((
                    source.entity_type.clone(),
                    source.node_id.clone(),
                    e.label.clone(),
                ))
            })
            .collect();

        // Build property filters from node watches
        let mut property_filters: HashMap<(String, String), HashSet<String>> = HashMap::new();
        for n in &req.nodes {
            if !n.properties.is_empty() {
                property_filters.insert(
                    (n.entity_type.clone(), n.node_id.clone()),
                    n.properties.iter().cloned().collect(),
                );
            }
        }

        let subscription = PointWatchSubscription {
            subscription_id: sub_id.clone(),
            database: ctx.database.clone(),
            namespace: ctx.namespace.clone(),
            watched_nodes,
            watched_edges,
            property_filters,
            sender: tokio::sync::mpsc::channel(1).0, // placeholder, registry replaces
            last_position: Versionstamp::zero(),
            created_at: Instant::now(),
            expires_at: Instant::now() + ttl,
        };

        let (_sub_id, receiver) = self
            .registry
            .create_point_watch(
                connection_id,
                ctx.database,
                ctx.namespace,
                subscription,
            )
            .await
            .into_status()?;

        let stream = tokio_stream::wrappers::ReceiverStream::new(receiver).map(Ok);
        Ok(Response::new(Box::pin(stream)))
    }

    type WatchQueryStream = Pin<Box<dyn Stream<Item = Result<WatchEvent, Status>> + Send>>;

    async fn watch_query(
        &self,
        request: Request<WatchQueryRequest>,
    ) -> Result<Response<Self::WatchQueryStream>, Status> {
        let connection_id = extract_connection_id(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;

        // Extract and compile the query expression
        let (expression, filter) = match req.query {
            Some(pelago_proto::watch_query_request::Query::CelExpression(expr)) => {
                compile_cel_filter(&self.schema_registry, &ctx.database, &ctx.namespace, &req.entity_type, &expr).await?
            }
            Some(pelago_proto::watch_query_request::Query::PqlQuery(_pql)) => {
                // PQL watches use client-compiled CEL in Phase 4
                return Err(Status::unimplemented(
                    "PQL query watches not yet supported; use CEL expression instead",
                ));
            }
            None => {
                return Err(Status::invalid_argument(
                    "query specification required (cel_expression or pql_query)",
                ));
            }
        };

        if req.entity_type.is_empty() {
            return Err(Status::invalid_argument(
                "entity_type required for query watches",
            ));
        }

        // Validate resume position
        if !req.resume_position.is_empty() {
            validate_resume(&self.registry, &ctx.database, &ctx.namespace, &req.resume_position)
                .await?;
        }

        let ttl = self.registry.compute_ttl(
            req.options
                .as_ref()
                .map(|o| o.ttl_seconds)
                .unwrap_or(0),
        );

        let sub_id = format!("sub_{}", uuid::Uuid::now_v7());

        let subscription = QueryWatchSubscription {
            subscription_id: sub_id.clone(),
            database: ctx.database.clone(),
            namespace: ctx.namespace.clone(),
            entity_type: req.entity_type,
            expression,
            filter,
            matching_nodes: HashSet::new(),
            sender: tokio::sync::mpsc::channel(1).0, // placeholder
            include_data: req.include_data,
            last_position: Versionstamp::zero(),
            created_at: Instant::now(),
            expires_at: Instant::now() + ttl,
        };

        let (_sub_id, receiver) = self
            .registry
            .create_query_watch(
                connection_id,
                ctx.database,
                ctx.namespace,
                subscription,
            )
            .await
            .into_status()?;

        let stream = tokio_stream::wrappers::ReceiverStream::new(receiver).map(Ok);
        Ok(Response::new(Box::pin(stream)))
    }

    type WatchNamespaceStream = Pin<Box<dyn Stream<Item = Result<WatchEvent, Status>> + Send>>;

    async fn watch_namespace(
        &self,
        request: Request<WatchNamespaceRequest>,
    ) -> Result<Response<Self::WatchNamespaceStream>, Status> {
        let connection_id = extract_connection_id(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;

        // Validate resume position
        if !req.resume_position.is_empty() {
            validate_resume(&self.registry, &ctx.database, &ctx.namespace, &req.resume_position)
                .await?;
        }

        let ttl = self.registry.compute_ttl(
            req.options
                .as_ref()
                .map(|o| o.ttl_seconds)
                .unwrap_or(0),
        );

        let sub_id = format!("sub_{}", uuid::Uuid::now_v7());

        let subscription = NamespaceWatchSubscription {
            subscription_id: sub_id.clone(),
            database: ctx.database.clone(),
            namespace: ctx.namespace.clone(),
            entity_type_filter: req.entity_types.into_iter().collect(),
            operation_type_filter: req.operation_types.into_iter().collect(),
            sender: tokio::sync::mpsc::channel(1).0, // placeholder
            last_position: Versionstamp::zero(),
            created_at: Instant::now(),
            expires_at: Instant::now() + ttl,
        };

        let (_sub_id, receiver) = self
            .registry
            .create_namespace_watch(
                connection_id,
                ctx.database,
                ctx.namespace,
                subscription,
            )
            .await
            .into_status()?;

        let stream = tokio_stream::wrappers::ReceiverStream::new(receiver).map(Ok);
        Ok(Response::new(Box::pin(stream)))
    }

    async fn list_subscriptions(
        &self,
        request: Request<ListSubscriptionsRequest>,
    ) -> Result<Response<ListSubscriptionsResponse>, Status> {
        let connection_id = extract_connection_id(&request);

        let subscriptions = self.registry.list_subscriptions(&connection_id).await;

        Ok(Response::new(ListSubscriptionsResponse { subscriptions }))
    }

    async fn cancel_subscription(
        &self,
        request: Request<CancelSubscriptionRequest>,
    ) -> Result<Response<CancelSubscriptionResponse>, Status> {
        let req = request.into_inner();

        let cancelled = self
            .registry
            .cancel_subscription(&req.subscription_id)
            .await
            .into_status()?;

        Ok(Response::new(CancelSubscriptionResponse { cancelled }))
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────

fn extract_connection_id<T>(request: &Request<T>) -> String {
    // Use peer address as connection identifier.
    // In production, this could also incorporate auth metadata.
    match request.remote_addr() {
        Some(addr) => format!("conn_{}", addr),
        None => format!("conn_unknown_{}", uuid::Uuid::now_v7()),
    }
}

async fn validate_resume(
    registry: &SubscriptionRegistry,
    database: &str,
    namespace: &str,
    resume_bytes: &[u8],
) -> Result<(), Status> {
    let vs = Versionstamp::from_bytes(resume_bytes)
        .ok_or_else(|| Status::invalid_argument("invalid resume_position (must be 10 bytes)"))?;

    let status = validate_resume_position(registry.db(), database, namespace, &vs)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    match status {
        ResumeStatus::Expired { oldest_available } => Err(Status::out_of_range(format!(
            "Resume position expired. Oldest available: {}",
            oldest_available
        ))),
        ResumeStatus::Valid | ResumeStatus::StartFromNow => Ok(()),
    }
}

/// Compile a CEL expression into a filter closure for query watch evaluation.
async fn compile_cel_filter(
    _schema_registry: &SchemaRegistry,
    _database: &str,
    _namespace: &str,
    _entity_type: &str,
    expression: &str,
) -> Result<
    (
        String,
        Arc<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>,
    ),
    Status,
> {
    // Attempt to compile the CEL expression
    let program = cel_interpreter::Program::compile(expression).map_err(|e| {
        Status::invalid_argument(format!("Invalid CEL expression '{}': {}", expression, e))
    })?;

    let expr_string = expression.to_string();

    // Create a closure that evaluates the program against property maps
    let filter: Arc<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync> =
        Arc::new(move |properties: &HashMap<String, Value>| {
            let mut ctx = cel_interpreter::Context::default();

            for (key, value) in properties {
                ctx.add_variable(key, value_to_cel(value)).ok();
            }

            match program.execute(&ctx) {
                Ok(cel_interpreter::Value::Bool(b)) => b,
                Ok(cel_interpreter::Value::Null) => false,
                Ok(_) => false,
                Err(_) => false,
            }
        });

    Ok((expr_string, filter))
}

/// Convert a PelagoDB core Value to a CEL interpreter Value.
fn value_to_cel(v: &Value) -> cel_interpreter::Value {
    match v {
        Value::String(s) => cel_interpreter::Value::String(Arc::new(s.clone())),
        Value::Int(i) => cel_interpreter::Value::Int(*i),
        Value::Float(f) => cel_interpreter::Value::Float(*f),
        Value::Bool(b) => cel_interpreter::Value::Bool(*b),
        Value::Timestamp(t) => cel_interpreter::Value::Int(*t),
        Value::Bytes(b) => cel_interpreter::Value::Bytes(Arc::new(b.clone())),
        Value::Null => cel_interpreter::Value::Null,
    }
}
