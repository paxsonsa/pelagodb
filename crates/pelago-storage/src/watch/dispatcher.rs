//! CDC Watch Dispatcher
//!
//! Consumes CDC entries from a namespace and routes events to active
//! subscriptions. Point watches use O(1) index lookup; namespace watches
//! use simple filter checks; query watches evaluate their filter function
//! and track enter/update/exit transitions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::cdc::{CdcOperation, Versionstamp};
use crate::consumer::{CdcConsumer, ConsumerConfig};
use crate::db::PelagoDb;
use crate::watch::subscriptions::*;
use pelago_core::PelagoError;
use pelago_proto::{
    EdgeEventData, NodeRef as ProtoNodeRef, QueryWatchMeta, WatchEvent, WatchEventType,
};

// ─── Shared State ───────────────────────────────────────────────────────

/// Shared subscription state accessed by both the dispatch loop (reads/writes)
/// and the registry (writes for registration).
pub struct DispatcherState {
    // Subscription registries by type
    pub point_watches: RwLock<HashMap<SubscriptionId, PointWatchSubscription>>,
    pub query_watches: RwLock<HashMap<SubscriptionId, QueryWatchSubscription>>,
    pub namespace_watches: RwLock<HashMap<SubscriptionId, NamespaceWatchSubscription>>,

    // Indexes for O(1) point watch lookup
    // (entity_type, node_id) → subscription IDs
    pub node_index: RwLock<HashMap<(String, String), Vec<SubscriptionId>>>,
    // (source_type, source_id, label) → subscription IDs
    pub edge_index: RwLock<HashMap<(String, String, String), Vec<SubscriptionId>>>,
}

impl DispatcherState {
    pub fn new() -> Self {
        Self {
            point_watches: RwLock::new(HashMap::new()),
            query_watches: RwLock::new(HashMap::new()),
            namespace_watches: RwLock::new(HashMap::new()),
            node_index: RwLock::new(HashMap::new()),
            edge_index: RwLock::new(HashMap::new()),
        }
    }

    /// Register a point watch subscription and update the lookup indexes.
    pub async fn register_point_watch(
        &self,
        sub_id: SubscriptionId,
        subscription: PointWatchSubscription,
    ) {
        // Update node index
        {
            let mut node_index = self.node_index.write().await;
            for key in &subscription.watched_nodes {
                node_index
                    .entry(key.clone())
                    .or_default()
                    .push(sub_id.clone());
            }
        }
        // Update edge index
        {
            let mut edge_index = self.edge_index.write().await;
            for key in &subscription.watched_edges {
                edge_index
                    .entry(key.clone())
                    .or_default()
                    .push(sub_id.clone());
            }
        }
        self.point_watches
            .write()
            .await
            .insert(sub_id, subscription);
    }

    /// Register a query watch subscription.
    pub async fn register_query_watch(
        &self,
        sub_id: SubscriptionId,
        subscription: QueryWatchSubscription,
    ) {
        self.query_watches
            .write()
            .await
            .insert(sub_id, subscription);
    }

    /// Register a namespace watch subscription.
    pub async fn register_namespace_watch(
        &self,
        sub_id: SubscriptionId,
        subscription: NamespaceWatchSubscription,
    ) {
        self.namespace_watches
            .write()
            .await
            .insert(sub_id, subscription);
    }

    /// Unregister a subscription (any type) and clean up indexes.
    pub async fn unregister(&self, sub_id: &SubscriptionId) {
        // Remove from point watches and indexes
        if let Some(sub) = self.point_watches.write().await.remove(sub_id) {
            let mut node_index = self.node_index.write().await;
            for key in &sub.watched_nodes {
                if let Some(subs) = node_index.get_mut(key) {
                    subs.retain(|id| id != sub_id);
                    if subs.is_empty() {
                        node_index.remove(key);
                    }
                }
            }
            let mut edge_index = self.edge_index.write().await;
            for key in &sub.watched_edges {
                if let Some(subs) = edge_index.get_mut(key) {
                    subs.retain(|id| id != sub_id);
                    if subs.is_empty() {
                        edge_index.remove(key);
                    }
                }
            }
        }
        // Remove from query watches
        self.query_watches.write().await.remove(sub_id);
        // Remove from namespace watches
        self.namespace_watches.write().await.remove(sub_id);
    }

    /// O(1) lookup for point watches matching a node operation.
    async fn lookup_point_watches_for_node(
        &self,
        entity_type: &str,
        node_id: &str,
    ) -> Option<Vec<SubscriptionId>> {
        self.node_index
            .read()
            .await
            .get(&(entity_type.to_string(), node_id.to_string()))
            .cloned()
    }

    /// O(1) lookup for point watches matching an edge operation.
    async fn lookup_point_watches_for_edge(
        &self,
        source_type: &str,
        source_id: &str,
        edge_type: &str,
    ) -> Option<Vec<SubscriptionId>> {
        self.edge_index
            .read()
            .await
            .get(&(
                source_type.to_string(),
                source_id.to_string(),
                edge_type.to_string(),
            ))
            .cloned()
    }
}

// ─── Dispatcher ─────────────────────────────────────────────────────────

/// CDC event dispatcher. Wraps a `CdcConsumer` and routes events to
/// subscriptions registered in the shared `DispatcherState`.
pub struct WatchDispatcher {
    namespace: String,
    consumer: CdcConsumer,
    state: Arc<DispatcherState>,
}

impl WatchDispatcher {
    /// Create a new dispatcher for the given namespace.
    pub async fn new(
        db: PelagoDb,
        database: String,
        namespace: String,
        state: Arc<DispatcherState>,
    ) -> Result<Self, PelagoError> {
        let consumer_id = format!(
            "watch_dispatcher_{}_{}_{}",
            database,
            namespace,
            dispatcher_instance_id()
        );
        let consumer_config = ConsumerConfig::new(&consumer_id, &database, &namespace);
        let consumer = CdcConsumer::new(db, consumer_config).await?;

        Ok(Self {
            namespace,
            consumer,
            state,
        })
    }

    /// Main dispatch loop. Polls CDC entries and routes them to subscriptions.
    /// Runs until the shutdown signal is received.
    pub async fn run(
        mut self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), PelagoError> {
        tracing::info!(
            namespace = %self.namespace,
            "WatchDispatcher started"
        );

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    tracing::info!(
                        namespace = %self.namespace,
                        "WatchDispatcher shutting down"
                    );
                    break;
                }
                batch_result = self.consumer.poll_batch() => {
                    let entries = batch_result?;
                    if entries.is_empty() {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    let last_vs = entries.last().map(|(vs, _)| vs.clone());

                    for (vs, entry) in &entries {
                        for op in &entry.operations {
                            self.dispatch_operation(op, vs).await;
                        }
                    }

                    if let Some(last_vs) = last_vs {
                        self.consumer.ack_through(&last_vs).await?;
                    }
                }
            }
        }

        // Final checkpoint before shutdown
        self.consumer.checkpoint().await?;
        Ok(())
    }

    /// Dispatch a single CDC operation to all matching subscriptions.
    async fn dispatch_operation(&self, op: &CdcOperation, position: &Versionstamp) {
        // 1. Fast path: point watches via index
        self.dispatch_to_point_watches(op, position).await;

        // 2. Query watches: evaluate filter, track enter/update/exit
        self.dispatch_to_query_watches(op, position).await;

        // 3. Namespace watches: simple filter check
        self.dispatch_to_namespace_watches(op, position).await;
    }

    async fn dispatch_to_point_watches(&self, op: &CdcOperation, position: &Versionstamp) {
        let sub_ids = match op {
            CdcOperation::NodeCreate {
                entity_type,
                node_id,
                ..
            }
            | CdcOperation::NodeUpdate {
                entity_type,
                node_id,
                ..
            }
            | CdcOperation::NodeDelete {
                entity_type,
                node_id,
                ..
            } => self
                .state
                .lookup_point_watches_for_node(entity_type, node_id)
                .await,

            CdcOperation::EdgeCreate {
                source_type,
                source_id,
                edge_type,
                ..
            }
            | CdcOperation::EdgeDelete {
                source_type,
                source_id,
                edge_type,
                ..
            } => self
                .state
                .lookup_point_watches_for_edge(source_type, source_id, edge_type)
                .await,

            CdcOperation::SchemaRegister { .. } => None,
        };

        if let Some(sub_ids) = sub_ids {
            let point_watches = self.state.point_watches.read().await;
            for sub_id in sub_ids {
                if let Some(sub) = point_watches.get(&sub_id) {
                    if sub.matches(op) {
                        let event = build_watch_event(op, position, &sub.subscription_id);
                        // Non-blocking send — if the client is slow, events are buffered
                        let _ = sub.sender.try_send(event);
                    }
                }
            }
        }
    }

    async fn dispatch_to_query_watches(&self, op: &CdcOperation, position: &Versionstamp) {
        let op_entity_type = entity_type_of(op);
        let node_id = match node_id_of(op) {
            Some(id) => id.to_string(),
            None => return, // Query watches only apply to node operations
        };

        let mut query_watches = self.state.query_watches.write().await;
        for sub in query_watches.values_mut() {
            if sub.entity_type != op_entity_type {
                continue;
            }

            match op {
                CdcOperation::NodeCreate { properties, .. } => {
                    let matches = (sub.filter)(properties);
                    if matches {
                        sub.matching_nodes.insert(node_id.clone());
                        let mut event = build_watch_event(op, position, &sub.subscription_id);
                        event.event_type = WatchEventType::QueryEnter as i32;
                        event.query_meta = Some(QueryWatchMeta {
                            exit_reason: 0,
                            initial_load: false,
                        });
                        if sub.include_data {
                            event.data = core_props_to_proto(properties);
                        }
                        let _ = sub.sender.try_send(event);
                    }
                }

                CdcOperation::NodeUpdate {
                    changed_properties,
                    old_properties,
                    ..
                } => {
                    let was_matching = sub.matching_nodes.contains(&node_id);

                    // Build the "after" property set from old + changed
                    // Note: this is the subset of properties available in the CDC entry.
                    // For full correctness, a DB read would be needed for properties
                    // not mentioned in the update.
                    let mut after_props = old_properties.clone();
                    after_props.extend(changed_properties.clone());

                    let now_matches = (sub.filter)(&after_props);

                    if was_matching && now_matches {
                        // UPDATE: still matches
                        let mut event = build_watch_event(op, position, &sub.subscription_id);
                        event.event_type = WatchEventType::NodeUpdated as i32;
                        if sub.include_data {
                            event.data = core_props_to_proto(&after_props);
                        }
                        let _ = sub.sender.try_send(event);
                    } else if !was_matching && now_matches {
                        // ENTER: newly matches
                        sub.matching_nodes.insert(node_id.clone());
                        let mut event = build_watch_event(op, position, &sub.subscription_id);
                        event.event_type = WatchEventType::QueryEnter as i32;
                        event.query_meta = Some(QueryWatchMeta {
                            exit_reason: 0,
                            initial_load: false,
                        });
                        if sub.include_data {
                            event.data = core_props_to_proto(&after_props);
                        }
                        let _ = sub.sender.try_send(event);
                    } else if was_matching && !now_matches {
                        // EXIT: no longer matches
                        sub.matching_nodes.remove(&node_id);
                        let mut event = build_watch_event(op, position, &sub.subscription_id);
                        event.event_type = WatchEventType::QueryExit as i32;
                        event.query_meta = Some(QueryWatchMeta {
                            exit_reason: pelago_proto::ExitReason::NoLongerMatches as i32,
                            initial_load: false,
                        });
                        let _ = sub.sender.try_send(event);
                    }
                    // else: !was_matching && !now_matches → no event
                }

                CdcOperation::NodeDelete { .. } => {
                    if sub.matching_nodes.remove(&node_id) {
                        let mut event = build_watch_event(op, position, &sub.subscription_id);
                        event.event_type = WatchEventType::QueryExit as i32;
                        event.query_meta = Some(QueryWatchMeta {
                            exit_reason: pelago_proto::ExitReason::Deleted as i32,
                            initial_load: false,
                        });
                        let _ = sub.sender.try_send(event);
                    }
                }

                _ => {}
            }
        }
    }

    async fn dispatch_to_namespace_watches(&self, op: &CdcOperation, position: &Versionstamp) {
        let namespace_watches = self.state.namespace_watches.read().await;
        for sub in namespace_watches.values() {
            if sub.matches(op) {
                let event = build_watch_event(op, position, &sub.subscription_id);
                let _ = sub.sender.try_send(event);
            }
        }
    }
}

fn dispatcher_instance_id() -> String {
    std::env::var("POD_NAME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| format!("pid{}", std::process::id()))
}

// ─── Event Building ─────────────────────────────────────────────────────

/// Build a `WatchEvent` proto from a CDC operation and position.
fn build_watch_event(
    op: &CdcOperation,
    position: &Versionstamp,
    subscription_id: &str,
) -> WatchEvent {
    let timestamp = now_micros();
    let pos_bytes = position.to_bytes().to_vec();

    match op {
        CdcOperation::NodeCreate {
            entity_type,
            node_id,
            properties,
            ..
        } => WatchEvent {
            event_id: pos_bytes.clone(),
            position: pos_bytes,
            event_type: WatchEventType::NodeCreated as i32,
            timestamp,
            subscription_id: subscription_id.to_string(),
            entity_type: entity_type.clone(),
            node_id: node_id.clone(),
            data: core_props_to_proto(properties),
            ..Default::default()
        },

        CdcOperation::NodeUpdate {
            entity_type,
            node_id,
            changed_properties,
            ..
        } => WatchEvent {
            event_id: pos_bytes.clone(),
            position: pos_bytes,
            event_type: WatchEventType::NodeUpdated as i32,
            timestamp,
            subscription_id: subscription_id.to_string(),
            entity_type: entity_type.clone(),
            node_id: node_id.clone(),
            changed_properties: core_props_to_proto(changed_properties),
            ..Default::default()
        },

        CdcOperation::NodeDelete {
            entity_type,
            node_id,
        } => WatchEvent {
            event_id: pos_bytes.clone(),
            position: pos_bytes,
            event_type: WatchEventType::NodeDeleted as i32,
            timestamp,
            subscription_id: subscription_id.to_string(),
            entity_type: entity_type.clone(),
            node_id: node_id.clone(),
            ..Default::default()
        },

        CdcOperation::EdgeCreate {
            source_type,
            source_id,
            target_type,
            target_id,
            edge_type,
            properties,
            ..
        } => WatchEvent {
            event_id: pos_bytes.clone(),
            position: pos_bytes,
            event_type: WatchEventType::EdgeCreated as i32,
            timestamp,
            subscription_id: subscription_id.to_string(),
            entity_type: source_type.clone(),
            node_id: source_id.clone(),
            edge: Some(EdgeEventData {
                source: Some(ProtoNodeRef {
                    entity_type: source_type.clone(),
                    node_id: source_id.clone(),
                    ..Default::default()
                }),
                target: Some(ProtoNodeRef {
                    entity_type: target_type.clone(),
                    node_id: target_id.clone(),
                    ..Default::default()
                }),
                label: edge_type.clone(),
                properties: core_props_to_proto(properties),
            }),
            ..Default::default()
        },

        CdcOperation::EdgeDelete {
            source_type,
            source_id,
            target_type,
            target_id,
            edge_type,
        } => WatchEvent {
            event_id: pos_bytes.clone(),
            position: pos_bytes,
            event_type: WatchEventType::EdgeDeleted as i32,
            timestamp,
            subscription_id: subscription_id.to_string(),
            entity_type: source_type.clone(),
            node_id: source_id.clone(),
            edge: Some(EdgeEventData {
                source: Some(ProtoNodeRef {
                    entity_type: source_type.clone(),
                    node_id: source_id.clone(),
                    ..Default::default()
                }),
                target: Some(ProtoNodeRef {
                    entity_type: target_type.clone(),
                    node_id: target_id.clone(),
                    ..Default::default()
                }),
                label: edge_type.clone(),
                properties: HashMap::new(),
            }),
            ..Default::default()
        },

        CdcOperation::SchemaRegister {
            entity_type,
            version,
        } => WatchEvent {
            event_id: pos_bytes.clone(),
            position: pos_bytes,
            event_type: WatchEventType::NodeCreated as i32, // Schema events use generic type
            timestamp,
            subscription_id: subscription_id.to_string(),
            entity_type: entity_type.clone(),
            node_id: format!("v{}", version),
            ..Default::default()
        },
    }
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64
}
