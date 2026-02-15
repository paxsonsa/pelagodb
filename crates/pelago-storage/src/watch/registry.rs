//! Subscription Registry
//!
//! Central registry for managing watch subscriptions across all namespaces.
//! Handles resource limits, connection tracking, and dispatcher lifecycle.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

use crate::db::PelagoDb;
use crate::watch::dispatcher::{DispatcherState, WatchDispatcher};
use crate::watch::subscriptions::*;
use pelago_core::PelagoError;
use pelago_proto::{SubscriptionInfo, WatchEvent};

pub type ConnectionId = String;

// ─── Configuration ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct RegistryConfig {
    /// Maximum subscriptions per connection
    pub max_subscriptions_per_connection: usize,
    /// Maximum total subscriptions per namespace
    pub max_subscriptions_per_namespace: usize,
    /// Maximum query watches (more expensive to evaluate)
    pub max_query_watches_per_namespace: usize,
    /// Default subscription TTL
    pub default_ttl: Duration,
    /// Maximum subscription TTL
    pub max_ttl: Duration,
    /// Heartbeat interval
    pub heartbeat_interval: Duration,
    /// Event queue depth per subscription
    pub event_queue_size: usize,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            max_subscriptions_per_connection: 100,
            max_subscriptions_per_namespace: 10_000,
            max_query_watches_per_namespace: 1_000,
            default_ttl: Duration::from_secs(3600),
            max_ttl: Duration::from_secs(86400),
            heartbeat_interval: Duration::from_secs(30),
            event_queue_size: 1000,
        }
    }
}

// ─── Subscription Metadata ──────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubType {
    Point,
    Query,
    Namespace,
}

struct SubscriptionMeta {
    subscription_id: SubscriptionId,
    sub_type: SubType,
    database: String,
    namespace: String,
    connection_id: ConnectionId,
    created_at: Instant,
    expires_at: Instant,
    events_delivered: u64,
}

// ─── Registry ───────────────────────────────────────────────────────────

/// Central subscription registry with resource limits and connection tracking.
pub struct SubscriptionRegistry {
    db: PelagoDb,
    config: RegistryConfig,

    /// Per-namespace dispatcher shared state
    dispatcher_states: RwLock<HashMap<String, Arc<DispatcherState>>>,
    /// Per-namespace shutdown signals for dispatchers
    dispatcher_shutdowns: RwLock<HashMap<String, tokio::sync::watch::Sender<bool>>>,

    /// Global subscription metadata
    subscriptions: RwLock<HashMap<SubscriptionId, SubscriptionMeta>>,
    /// Connection → subscription IDs
    connections: RwLock<HashMap<ConnectionId, Vec<SubscriptionId>>>,
}

impl SubscriptionRegistry {
    pub fn new(db: PelagoDb, config: RegistryConfig) -> Self {
        Self {
            db,
            config,
            dispatcher_states: RwLock::new(HashMap::new()),
            dispatcher_shutdowns: RwLock::new(HashMap::new()),
            subscriptions: RwLock::new(HashMap::new()),
            connections: RwLock::new(HashMap::new()),
        }
    }

    /// Create a point watch subscription.
    pub async fn create_point_watch(
        &self,
        connection_id: ConnectionId,
        database: String,
        namespace: String,
        subscription: PointWatchSubscription,
    ) -> Result<(SubscriptionId, mpsc::Receiver<WatchEvent>), PelagoError> {
        self.check_limits(&connection_id, &namespace, SubType::Point)
            .await?;

        let sub_id = subscription.subscription_id.clone();
        let (sender, receiver) = mpsc::channel(self.config.event_queue_size);

        let state = self
            .get_or_create_dispatcher_state(&database, &namespace)
            .await?;

        let sub = PointWatchSubscription {
            sender,
            ..subscription
        };
        state.register_point_watch(sub_id.clone(), sub).await;

        self.track_subscription(
            sub_id.clone(),
            SubType::Point,
            database,
            namespace,
            connection_id,
        )
        .await;

        Ok((sub_id, receiver))
    }

    /// Create a query watch subscription.
    pub async fn create_query_watch(
        &self,
        connection_id: ConnectionId,
        database: String,
        namespace: String,
        subscription: QueryWatchSubscription,
    ) -> Result<(SubscriptionId, mpsc::Receiver<WatchEvent>), PelagoError> {
        self.check_limits(&connection_id, &namespace, SubType::Query)
            .await?;

        let sub_id = subscription.subscription_id.clone();
        let (sender, receiver) = mpsc::channel(self.config.event_queue_size);

        let state = self
            .get_or_create_dispatcher_state(&database, &namespace)
            .await?;

        let sub = QueryWatchSubscription {
            sender,
            ..subscription
        };
        state.register_query_watch(sub_id.clone(), sub).await;

        self.track_subscription(
            sub_id.clone(),
            SubType::Query,
            database,
            namespace,
            connection_id,
        )
        .await;

        Ok((sub_id, receiver))
    }

    /// Create a namespace watch subscription.
    pub async fn create_namespace_watch(
        &self,
        connection_id: ConnectionId,
        database: String,
        namespace: String,
        subscription: NamespaceWatchSubscription,
    ) -> Result<(SubscriptionId, mpsc::Receiver<WatchEvent>), PelagoError> {
        self.check_limits(&connection_id, &namespace, SubType::Namespace)
            .await?;

        let sub_id = subscription.subscription_id.clone();
        let (sender, receiver) = mpsc::channel(self.config.event_queue_size);

        let state = self
            .get_or_create_dispatcher_state(&database, &namespace)
            .await?;

        let sub = NamespaceWatchSubscription {
            sender,
            ..subscription
        };
        state
            .register_namespace_watch(sub_id.clone(), sub)
            .await;

        self.track_subscription(
            sub_id.clone(),
            SubType::Namespace,
            database,
            namespace,
            connection_id,
        )
        .await;

        Ok((sub_id, receiver))
    }

    /// Cancel a subscription by ID.
    pub async fn cancel_subscription(
        &self,
        subscription_id: &SubscriptionId,
    ) -> Result<bool, PelagoError> {
        let meta = self.subscriptions.write().await.remove(subscription_id);
        if let Some(meta) = meta {
            // Remove from dispatcher state
            let key = format!("{}:{}", meta.database, meta.namespace);
            if let Some(state) = self.dispatcher_states.read().await.get(&key) {
                state.unregister(subscription_id).await;
            }
            // Remove from connection tracking
            if let Some(subs) = self.connections.write().await.get_mut(&meta.connection_id) {
                subs.retain(|id| id != subscription_id);
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Cleanup all subscriptions for a connection (e.g., on disconnect).
    pub async fn cleanup_connection(&self, connection_id: &ConnectionId) {
        let sub_ids = self.connections.write().await.remove(connection_id);
        if let Some(sub_ids) = sub_ids {
            for sub_id in sub_ids {
                let _ = self.cancel_subscription(&sub_id).await;
            }
        }
    }

    /// List subscriptions for a connection.
    pub async fn list_subscriptions(
        &self,
        connection_id: &ConnectionId,
    ) -> Vec<SubscriptionInfo> {
        let connections = self.connections.read().await;
        let subscriptions = self.subscriptions.read().await;

        let sub_ids = match connections.get(connection_id) {
            Some(ids) => ids,
            None => return Vec::new(),
        };

        sub_ids
            .iter()
            .filter_map(|id| {
                let meta = subscriptions.get(id)?;
                let sub_type = match meta.sub_type {
                    SubType::Point => pelago_proto::SubscriptionType::Point as i32,
                    SubType::Query => pelago_proto::SubscriptionType::Query as i32,
                    SubType::Namespace => pelago_proto::SubscriptionType::Namespace as i32,
                };
                Some(SubscriptionInfo {
                    subscription_id: meta.subscription_id.clone(),
                    r#type: sub_type,
                    database: meta.database.clone(),
                    namespace: meta.namespace.clone(),
                    created_at: meta
                        .created_at
                        .elapsed()
                        .as_micros() as i64, // relative for now
                    expires_at: meta
                        .expires_at
                        .duration_since(meta.created_at)
                        .as_secs() as i64,
                    events_delivered: meta.events_delivered,
                })
            })
            .collect()
    }

    /// Get all expired subscription IDs.
    pub async fn get_expired_subscriptions(&self) -> Vec<SubscriptionId> {
        let now = Instant::now();
        self.subscriptions
            .read()
            .await
            .iter()
            .filter(|(_, meta)| meta.expires_at <= now)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get the sender for a subscription (used by TTL manager to send end events).
    pub async fn get_sender(
        &self,
        subscription_id: &SubscriptionId,
    ) -> Option<mpsc::Sender<WatchEvent>> {
        let meta = self.subscriptions.read().await;
        let meta = meta.get(subscription_id)?;
        let key = format!("{}:{}", meta.database, meta.namespace);
        let states = self.dispatcher_states.read().await;
        let state = states.get(&key)?;

        // Check each subscription type for the sender
        if let Some(sub) = state.point_watches.read().await.get(subscription_id) {
            return Some(sub.sender.clone());
        }
        if let Some(sub) = state.query_watches.read().await.get(subscription_id) {
            return Some(sub.sender.clone());
        }
        if let Some(sub) = state.namespace_watches.read().await.get(subscription_id) {
            return Some(sub.sender.clone());
        }
        None
    }

    /// Get the database handle (for resume position validation).
    pub fn db(&self) -> &PelagoDb {
        &self.db
    }

    /// Get the registry configuration.
    pub fn config(&self) -> &RegistryConfig {
        &self.config
    }

    /// Shutdown all dispatchers gracefully.
    pub async fn shutdown(&self) {
        let shutdowns = self.dispatcher_shutdowns.write().await;
        for (key, tx) in shutdowns.iter() {
            tracing::info!(namespace = %key, "Shutting down WatchDispatcher");
            let _ = tx.send(true);
        }
    }

    // ─── Internal ───────────────────────────────────────────────────────

    async fn check_limits(
        &self,
        connection_id: &ConnectionId,
        namespace: &str,
        sub_type: SubType,
    ) -> Result<(), PelagoError> {
        // Per-connection limit
        let connections = self.connections.read().await;
        if let Some(subs) = connections.get(connection_id) {
            if subs.len() >= self.config.max_subscriptions_per_connection {
                return Err(PelagoError::WatchSubscriptionLimit {
                    limit: self.config.max_subscriptions_per_connection,
                    current: subs.len(),
                });
            }
        }
        drop(connections);

        // Per-namespace limit
        let subscriptions = self.subscriptions.read().await;
        let ns_count = subscriptions
            .values()
            .filter(|m| m.namespace == namespace)
            .count();
        if ns_count >= self.config.max_subscriptions_per_namespace {
            return Err(PelagoError::WatchSubscriptionLimit {
                limit: self.config.max_subscriptions_per_namespace,
                current: ns_count,
            });
        }

        // Query watch limit
        if sub_type == SubType::Query {
            let query_count = subscriptions
                .values()
                .filter(|m| m.namespace == namespace && m.sub_type == SubType::Query)
                .count();
            if query_count >= self.config.max_query_watches_per_namespace {
                return Err(PelagoError::WatchQueryLimit {
                    limit: self.config.max_query_watches_per_namespace,
                    current: query_count,
                });
            }
        }

        Ok(())
    }

    async fn track_subscription(
        &self,
        sub_id: SubscriptionId,
        sub_type: SubType,
        database: String,
        namespace: String,
        connection_id: ConnectionId,
    ) {
        let ttl = self.config.default_ttl;
        let meta = SubscriptionMeta {
            subscription_id: sub_id.clone(),
            sub_type,
            database,
            namespace,
            connection_id: connection_id.clone(),
            created_at: Instant::now(),
            expires_at: Instant::now() + ttl,
            events_delivered: 0,
        };

        self.subscriptions.write().await.insert(sub_id.clone(), meta);
        self.connections
            .write()
            .await
            .entry(connection_id)
            .or_default()
            .push(sub_id);
    }

    /// Compute TTL from watch options (clamped to max_ttl).
    pub fn compute_ttl(&self, ttl_seconds: u32) -> Duration {
        if ttl_seconds > 0 {
            let requested = Duration::from_secs(ttl_seconds as u64);
            requested.min(self.config.max_ttl)
        } else {
            self.config.default_ttl
        }
    }

    /// Get or create the dispatcher state for a namespace, spawning
    /// the dispatch loop if this is the first subscription.
    async fn get_or_create_dispatcher_state(
        &self,
        database: &str,
        namespace: &str,
    ) -> Result<Arc<DispatcherState>, PelagoError> {
        let key = format!("{}:{}", database, namespace);

        // Fast path: state exists
        {
            let states = self.dispatcher_states.read().await;
            if let Some(state) = states.get(&key) {
                return Ok(Arc::clone(state));
            }
        }

        // Slow path: create state and spawn dispatcher
        let mut states = self.dispatcher_states.write().await;
        // Double-check after acquiring write lock
        if let Some(state) = states.get(&key) {
            return Ok(Arc::clone(state));
        }

        let state = Arc::new(DispatcherState::new());
        states.insert(key.clone(), Arc::clone(&state));

        // Create and spawn the dispatcher
        let dispatcher = WatchDispatcher::new(
            self.db.clone(),
            database.to_string(),
            namespace.to_string(),
            Arc::clone(&state),
        )
        .await?;

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        self.dispatcher_shutdowns
            .write()
            .await
            .insert(key.clone(), shutdown_tx);

        let dispatch_key = key.clone();
        tokio::spawn(async move {
            if let Err(e) = dispatcher.run(shutdown_rx).await {
                tracing::error!(
                    namespace = %dispatch_key,
                    error = %e,
                    "WatchDispatcher error"
                );
            }
        });

        Ok(state)
    }
}
