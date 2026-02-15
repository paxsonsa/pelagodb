## 18. Reactive Subscriptions (Watch System)

The Watch System provides real-time change notifications via server-streaming gRPC. Clients subscribe to data changes using point watches, query watches, or namespace watches. All subscriptions are powered by the CDC infrastructure defined in Section 11.

### 18.1 Overview

The Watch System enables three primary use cases:

1. **Reactive UIs:** Watch specific nodes/edges and receive updates as they change
2. **Materialized Views:** Subscribe to query results and maintain external projections (Flink, Kafka, etc.)
3. **Coordination Primitives:** Wait for a specific condition to become true ("watch until X happens")

**Design Principles:**

- **CDC-powered:** All watch events are derived from CDC entries, sharing infrastructure with replication
- **gRPC streaming:** Server-streaming RPCs minimize connection overhead compared to WebSocket
- **Position-based resume:** Clients track versionstamps to resume after disconnection
- **Server-side filtering:** CDC events are filtered server-side to reduce network traffic
- **Resource-bounded:** Subscription limits and TTLs prevent unbounded resource consumption

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Watch System Architecture                          │
└─────────────────────────────────────────────────────────────────────────────┘

                                  CDC Log (FDB)
                                       │
                                       ▼
                        ┌──────────────────────────────┐
                        │     CDC Consumer Pool        │
                        │  (shared with replication)   │
                        └──────────────────────────────┘
                                       │
                          CDC entries flow to...
                                       │
                                       ▼
                        ┌──────────────────────────────┐
                        │    Subscription Registry     │
                        │  ┌────────────────────────┐  │
                        │  │ Point Watch: node_42   │──┼──► gRPC stream → Client A
                        │  ├────────────────────────┤  │
                        │  │ Query Watch: age > 30  │──┼──► gRPC stream → Client B
                        │  ├────────────────────────┤  │
                        │  │ NS Watch: user_data/*  │──┼──► gRPC stream → Client C
                        │  └────────────────────────┘  │
                        └──────────────────────────────┘
```

### 18.2 WatchService gRPC API

```protobuf
service WatchService {
  // Subscribe to changes on specific nodes, edges, or properties
  rpc WatchPoint(WatchPointRequest) returns (stream WatchEvent);

  // Subscribe to changes matching a CEL or PQL query
  rpc WatchQuery(WatchQueryRequest) returns (stream WatchEvent);

  // Subscribe to all changes within a namespace
  rpc WatchNamespace(WatchNamespaceRequest) returns (stream WatchEvent);

  // List active subscriptions for this connection
  rpc ListSubscriptions(ListSubscriptionsRequest) returns (ListSubscriptionsResponse);

  // Cancel a subscription by ID
  rpc CancelSubscription(CancelSubscriptionRequest) returns (CancelSubscriptionResponse);
}
```

### 18.3 Subscription Types

#### 18.3.1 Point Watch

Point watches monitor specific nodes, edges, or properties. They are the most efficient subscription type with O(1) matching cost per CDC event.

```protobuf
message WatchPointRequest {
  RequestContext context = 1;

  // What to watch (at least one required)
  repeated NodeWatch nodes = 2;
  repeated EdgeWatch edges = 3;

  // Resume position (optional, empty = start from now)
  bytes resume_position = 4;

  // Subscription options
  WatchOptions options = 5;
}

message NodeWatch {
  string entity_type = 1;
  string node_id = 2;

  // Optional: watch only specific properties (empty = all)
  repeated string properties = 3;
}

message EdgeWatch {
  NodeRef source = 1;
  string label = 2;

  // Optional: watch specific target (empty = all edges with label)
  NodeRef target = 3;

  // Optional: watch edge direction
  EdgeDirection direction = 4;
}
```

**Rust Implementation:**

```rust
struct PointWatchSubscription {
    subscription_id: SubscriptionId,
    database: String,
    namespace: String,

    // O(1) lookup structures
    watched_nodes: HashSet<(String, NodeId)>,      // (entity_type, node_id)
    watched_edges: HashSet<(NodeRef, String)>,     // (source, label)
    property_filters: HashMap<(String, NodeId), HashSet<String>>, // property restrictions

    // Client stream handle
    sender: mpsc::Sender<WatchEvent>,

    // Position tracking
    last_position: Versionstamp,

    // Lifecycle
    created_at: Instant,
    expires_at: Instant,
}

impl PointWatchSubscription {
    fn matches(&self, op: &CdcOperation) -> bool {
        match op {
            CdcOperation::NodeCreate { entity_type, node_id, .. } |
            CdcOperation::NodeUpdate { entity_type, node_id, .. } |
            CdcOperation::NodeDelete { entity_type, node_id, .. } => {
                if !self.watched_nodes.contains(&(entity_type.clone(), node_id.clone())) {
                    return false;
                }

                // Check property filter if specified
                if let Some(props) = self.property_filters.get(&(entity_type.clone(), node_id.clone())) {
                    if let CdcOperation::NodeUpdate { changed_properties, .. } = op {
                        // Only match if watched properties changed
                        return changed_properties.keys().any(|k| props.contains(k));
                    }
                }
                true
            }

            CdcOperation::EdgeCreate { source, edge_type, .. } |
            CdcOperation::EdgeDelete { source, edge_type, .. } => {
                self.watched_edges.contains(&(source.clone(), edge_type.clone()))
            }

            _ => false,
        }
    }
}
```

**Use Cases:**

| Pattern | Example | Description |
|---------|---------|-------------|
| Single node | Watch `Person:p_42` | Notify on any change to node |
| Property subset | Watch `Person:p_42.{name,email}` | Notify only when name or email change |
| Node edges | Watch `Person:p_42 -[KNOWS]->` | Notify when KNOWS edges are added/removed |
| Specific edge | Watch `Person:p_42 -[KNOWS]-> Person:p_99` | Notify when this specific edge changes |

#### 18.3.2 Query Watch

Query watches monitor all entities matching a CEL or PQL predicate. Events are emitted when:

1. A new entity starts matching the query (enters result set)
2. An existing match changes (update within result set)
3. An entity stops matching (exits result set)

```protobuf
message WatchQueryRequest {
  RequestContext context = 1;

  // Query specification (exactly one required)
  oneof query {
    // CEL expression (e.g., "entity_type == 'Person' && age >= 30")
    string cel_expression = 2;

    // PQL query string
    string pql_query = 3;
  }

  // Entity type scope (required for CEL, optional for PQL)
  string entity_type = 4;

  // Include full entity data in events (default: true)
  bool include_data = 5;

  // Resume position
  bytes resume_position = 6;

  // Subscription options
  WatchOptions options = 7;
}
```

**Query Watch Semantics:**

```rust
enum QueryWatchEvent {
    /// Entity now matches query (was created or updated to match)
    Enter {
        entity_type: String,
        node_id: NodeId,
        data: Option<HashMap<String, Value>>,
    },

    /// Entity still matches but properties changed
    Update {
        entity_type: String,
        node_id: NodeId,
        changed_properties: HashMap<String, Value>,
    },

    /// Entity no longer matches query (deleted or updated to not match)
    Exit {
        entity_type: String,
        node_id: NodeId,
        reason: ExitReason,
    },
}

enum ExitReason {
    Deleted,
    NoLongerMatches,
}
```

**Rust Implementation:**

```rust
struct QueryWatchSubscription {
    subscription_id: SubscriptionId,
    database: String,
    namespace: String,
    entity_type: String,

    // Compiled query
    cel_program: Arc<cel_interpreter::Program>,

    // Track which nodes currently match (for enter/exit detection)
    // Stored in FDB for durability across restarts
    matching_nodes: HashSet<NodeId>,

    // Client stream handle
    sender: mpsc::Sender<WatchEvent>,

    // Position tracking
    last_position: Versionstamp,

    // Configuration
    include_data: bool,
}

impl QueryWatchSubscription {
    async fn process_event(&mut self, op: &CdcOperation, db: &FdbDatabase) -> Option<WatchEvent> {
        match op {
            CdcOperation::NodeCreate { entity_type, node_id, properties, .. } => {
                if entity_type != &self.entity_type {
                    return None;
                }

                let matches = self.evaluate_cel(properties);
                if matches {
                    self.matching_nodes.insert(node_id.clone());
                    return Some(WatchEvent {
                        event_type: WatchEventType::Enter,
                        node_id: node_id.clone(),
                        data: if self.include_data { Some(properties.clone()) } else { None },
                        ..default()
                    });
                }
                None
            }

            CdcOperation::NodeUpdate { entity_type, node_id, changed_properties, .. } => {
                if entity_type != &self.entity_type {
                    return None;
                }

                // Fetch current full properties to re-evaluate
                let current = self.fetch_node_properties(db, entity_type, node_id).await?;
                let now_matches = self.evaluate_cel(&current);
                let previously_matched = self.matching_nodes.contains(node_id);

                match (previously_matched, now_matches) {
                    (false, true) => {
                        // Entered result set
                        self.matching_nodes.insert(node_id.clone());
                        Some(WatchEvent {
                            event_type: WatchEventType::Enter,
                            node_id: node_id.clone(),
                            data: if self.include_data { Some(current) } else { None },
                            ..default()
                        })
                    }
                    (true, true) => {
                        // Updated within result set
                        Some(WatchEvent {
                            event_type: WatchEventType::Update,
                            node_id: node_id.clone(),
                            changed_properties: Some(changed_properties.clone()),
                            ..default()
                        })
                    }
                    (true, false) => {
                        // Exited result set
                        self.matching_nodes.remove(node_id);
                        Some(WatchEvent {
                            event_type: WatchEventType::Exit,
                            node_id: node_id.clone(),
                            exit_reason: Some(ExitReason::NoLongerMatches),
                            ..default()
                        })
                    }
                    (false, false) => None,
                }
            }

            CdcOperation::NodeDelete { entity_type, node_id, .. } => {
                if entity_type != &self.entity_type {
                    return None;
                }

                if self.matching_nodes.remove(node_id) {
                    return Some(WatchEvent {
                        event_type: WatchEventType::Exit,
                        node_id: node_id.clone(),
                        exit_reason: Some(ExitReason::Deleted),
                        ..default()
                    });
                }
                None
            }

            _ => None,
        }
    }

    fn evaluate_cel(&self, properties: &HashMap<String, Value>) -> bool {
        let context = cel_interpreter::Context::from_properties(properties);
        match self.cel_program.execute(&context) {
            Ok(cel_interpreter::Value::Bool(b)) => b,
            _ => false,
        }
    }
}
```

**PQL Query Watch:**

PQL queries compile to the same internal representation. Complex traversals are watched by monitoring the root entity and re-evaluating the full query on changes.

```
# PQL query watch example
watch query active_engineers {
  start(func: type(Person)) @filter(department == "Engineering" && active) {
    name
    email
    -[WORKS_ON]-> Project @filter(status == "active") {
      name
    }
  }
}
```

**Query Complexity Limits:**

| Limit | Default | Description |
|-------|---------|-------------|
| Max CEL complexity | 100 | CEL expression cost limit |
| Max PQL depth | 4 | Maximum traversal depth for query watches |
| Max matching nodes | 10,000 | Maximum tracked result set size |
| Re-evaluation timeout | 100ms | Timeout for re-evaluating query on update |

#### 18.3.3 Namespace Watch

Namespace watches monitor all changes within a namespace. They are intended for:

- Full change logs for audit trails
- External system projections (Kafka, data lakes)
- Debugging and development

```protobuf
message WatchNamespaceRequest {
  RequestContext context = 1;

  // Optional: filter by entity types (empty = all types)
  repeated string entity_types = 2;

  // Optional: filter by operation types
  repeated OperationType operation_types = 3;

  // Resume position
  bytes resume_position = 4;

  // Subscription options
  WatchOptions options = 5;
}

enum OperationType {
  OPERATION_TYPE_UNSPECIFIED = 0;
  OPERATION_TYPE_NODE_CREATE = 1;
  OPERATION_TYPE_NODE_UPDATE = 2;
  OPERATION_TYPE_NODE_DELETE = 3;
  OPERATION_TYPE_EDGE_CREATE = 4;
  OPERATION_TYPE_EDGE_DELETE = 5;
  OPERATION_TYPE_SCHEMA_REGISTER = 6;
}
```

**Rust Implementation:**

```rust
struct NamespaceWatchSubscription {
    subscription_id: SubscriptionId,
    database: String,
    namespace: String,

    // Filters (empty = no filter, all pass)
    entity_type_filter: HashSet<String>,
    operation_type_filter: HashSet<OperationType>,

    // Client stream handle
    sender: mpsc::Sender<WatchEvent>,

    // Position tracking
    last_position: Versionstamp,
}

impl NamespaceWatchSubscription {
    fn matches(&self, op: &CdcOperation) -> bool {
        // Check operation type filter
        if !self.operation_type_filter.is_empty() {
            let op_type = operation_type_of(op);
            if !self.operation_type_filter.contains(&op_type) {
                return false;
            }
        }

        // Check entity type filter
        if !self.entity_type_filter.is_empty() {
            let entity_type = entity_type_of(op);
            if !self.entity_type_filter.contains(&entity_type) {
                return false;
            }
        }

        true
    }
}
```

### 18.4 Watch Events

All subscription types emit the same `WatchEvent` message:

```protobuf
message WatchEvent {
  // Unique event ID (versionstamp from CDC)
  bytes event_id = 1;

  // Position for resume (opaque, pass back in resume_position)
  bytes position = 2;

  // Event type
  WatchEventType event_type = 3;

  // Timestamp (unix microseconds)
  int64 timestamp = 4;

  // Subscription that matched this event
  string subscription_id = 5;

  // Entity information
  string entity_type = 6;
  string node_id = 7;

  // Optional: full entity data (if requested)
  map<string, Value> data = 8;

  // Optional: changed properties only (for updates)
  map<string, Value> changed_properties = 9;

  // Optional: edge information (for edge events)
  EdgeEventData edge = 10;

  // For query watches: entry/exit reason
  QueryWatchMeta query_meta = 11;
}

enum WatchEventType {
  WATCH_EVENT_TYPE_UNSPECIFIED = 0;

  // Node events
  WATCH_EVENT_TYPE_NODE_CREATED = 1;
  WATCH_EVENT_TYPE_NODE_UPDATED = 2;
  WATCH_EVENT_TYPE_NODE_DELETED = 3;

  // Edge events
  WATCH_EVENT_TYPE_EDGE_CREATED = 4;
  WATCH_EVENT_TYPE_EDGE_DELETED = 5;

  // Query watch specific
  WATCH_EVENT_TYPE_QUERY_ENTER = 6;    // Entity now matches query
  WATCH_EVENT_TYPE_QUERY_EXIT = 7;     // Entity no longer matches query

  // System events
  WATCH_EVENT_TYPE_HEARTBEAT = 8;      // Keep-alive, no data change
  WATCH_EVENT_TYPE_SUBSCRIPTION_ENDED = 9; // Subscription terminated
}

message EdgeEventData {
  NodeRef source = 1;
  NodeRef target = 2;
  string label = 3;
  map<string, Value> properties = 4;
}

message QueryWatchMeta {
  // For QUERY_EXIT: why the entity exited
  ExitReason exit_reason = 1;

  // For QUERY_ENTER: was this initial load or live change
  bool initial_load = 2;
}

enum ExitReason {
  EXIT_REASON_UNSPECIFIED = 0;
  EXIT_REASON_DELETED = 1;
  EXIT_REASON_NO_LONGER_MATCHES = 2;
}
```

### 18.5 Subscription Options

```protobuf
message WatchOptions {
  // Subscription TTL in seconds (0 = use server default)
  uint32 ttl_seconds = 1;

  // Heartbeat interval in seconds (0 = use server default)
  uint32 heartbeat_seconds = 2;

  // Maximum events per second (0 = unlimited)
  uint32 rate_limit = 3;

  // Batch events before sending (reduces RPCs, increases latency)
  BatchOptions batch = 4;

  // Initial load: emit current state before streaming changes
  bool initial_snapshot = 5;
}

message BatchOptions {
  // Maximum events per batch
  uint32 max_events = 1;

  // Maximum time to wait for batch fill (milliseconds)
  uint32 max_wait_ms = 2;
}
```

### 18.6 CDC Integration Design

The Watch System shares CDC infrastructure with multi-site replication (Section 12).

#### 18.6.1 CDC Consumer Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Shared CDC Consumer                                │
└─────────────────────────────────────────────────────────────────────────────┘

                          FoundationDB CDC Log
                          (db, ns, _cdc, <versionstamp>)
                                    │
                                    ▼
                        ┌───────────────────────┐
                        │   CDC Consumer Pool   │
                        │  (per namespace)      │
                        └───────────────────────┘
                                    │
                    ┌───────────────┼───────────────┐
                    │               │               │
                    ▼               ▼               ▼
            ┌───────────┐   ┌───────────────┐   ┌──────────────┐
            │ Replicator│   │ WatchDispatcher│  │ Future: Hooks │
            │ (for SYD) │   │               │   │ (webhooks)   │
            └───────────┘   └───────────────┘   └──────────────┘
                                    │
                    ┌───────────────┼───────────────┐
                    │               │               │
                    ▼               ▼               ▼
            ┌───────────┐   ┌───────────┐   ┌───────────┐
            │ Point     │   │ Query     │   │ Namespace │
            │ Watch     │   │ Watch     │   │ Watch     │
            │ Subs      │   │ Subs      │   │ Subs      │
            └───────────┘   └───────────┘   └───────────┘
```

#### 18.6.2 Watch Dispatcher

The Watch Dispatcher is a CDC consumer that routes events to active subscriptions:

```rust
struct WatchDispatcher {
    namespace: String,
    consumer_id: String,

    // Subscription registries by type
    point_watches: RwLock<HashMap<SubscriptionId, PointWatchSubscription>>,
    query_watches: RwLock<HashMap<SubscriptionId, QueryWatchSubscription>>,
    namespace_watches: RwLock<HashMap<SubscriptionId, NamespaceWatchSubscription>>,

    // Index for O(1) point watch lookup
    node_index: RwLock<HashMap<(String, NodeId), Vec<SubscriptionId>>>,
    edge_index: RwLock<HashMap<(NodeRef, String), Vec<SubscriptionId>>>,

    // Position tracking
    hwm_key: Vec<u8>,
}

impl WatchDispatcher {
    async fn run(&self, db: &FdbDatabase) -> Result<()> {
        let mut hwm = self.load_hwm(db).await?;

        loop {
            let tx = db.create_transaction()?;

            // Range scan from HWM
            let range_start = (db_id, &self.namespace, "_cdc", hwm.next());
            let range_end = (db_id, &self.namespace, "_cdc").range_end();

            let entries = tx.get_range(&range_start, &range_end, RangeOption {
                limit: 1000,
                ..default()
            }).await?;

            if entries.is_empty() {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }

            for (key, value) in &entries {
                let entry: CdcEntry = decode_cbor(value)?;
                let position = extract_versionstamp(key);

                for op in &entry.operations {
                    self.dispatch_operation(op, &position).await;
                }

                hwm = position;
            }

            // Periodic checkpoint
            self.save_hwm(db, &hwm).await?;
        }
    }

    async fn dispatch_operation(&self, op: &CdcOperation, position: &Versionstamp) {
        // Fast path: point watches via index
        if let Some(sub_ids) = self.lookup_point_watches(op) {
            let point_watches = self.point_watches.read().await;
            for sub_id in sub_ids {
                if let Some(sub) = point_watches.get(&sub_id) {
                    if sub.matches(op) {
                        let event = self.build_watch_event(op, position, &sub.subscription_id);
                        let _ = sub.sender.send(event).await;
                    }
                }
            }
        }

        // Query watches: evaluate all (filtered by entity type)
        let entity_type = entity_type_of(op);
        let query_watches = self.query_watches.read().await;
        for sub in query_watches.values() {
            if sub.entity_type == entity_type {
                if let Some(event) = sub.process_event(op).await {
                    let _ = sub.sender.send(event).await;
                }
            }
        }

        // Namespace watches: simple filter check
        let namespace_watches = self.namespace_watches.read().await;
        for sub in namespace_watches.values() {
            if sub.matches(op) {
                let event = self.build_watch_event(op, position, &sub.subscription_id);
                let _ = sub.sender.send(event).await;
            }
        }
    }

    fn lookup_point_watches(&self, op: &CdcOperation) -> Option<Vec<SubscriptionId>> {
        match op {
            CdcOperation::NodeCreate { entity_type, node_id, .. } |
            CdcOperation::NodeUpdate { entity_type, node_id, .. } |
            CdcOperation::NodeDelete { entity_type, node_id, .. } => {
                self.node_index.read()
                    .get(&(entity_type.clone(), node_id.clone()))
                    .cloned()
            }

            CdcOperation::EdgeCreate { source, edge_type, .. } |
            CdcOperation::EdgeDelete { source, edge_type, .. } => {
                self.edge_index.read()
                    .get(&(source.clone(), edge_type.clone()))
                    .cloned()
            }

            _ => None,
        }
    }
}
```

#### 18.6.3 Consumer Position Isolation

Watch consumers track position separately from replication consumers:

```
Key: (db, ns, _meta, cdc_checkpoints, watch_dispatcher)
Value: 10-byte versionstamp
```

This ensures watch processing does not affect or depend on replication progress.

### 18.7 Server-Side Subscription Registry

#### 18.7.1 Registry Structure

```rust
struct SubscriptionRegistry {
    // Per-namespace dispatchers
    dispatchers: RwLock<HashMap<String, Arc<WatchDispatcher>>>,

    // Global subscription tracking
    subscriptions: RwLock<HashMap<SubscriptionId, SubscriptionMeta>>,

    // Connection tracking for cleanup
    connections: RwLock<HashMap<ConnectionId, Vec<SubscriptionId>>>,

    // Resource limits
    config: RegistryConfig,
}

struct SubscriptionMeta {
    subscription_id: SubscriptionId,
    subscription_type: SubscriptionType,
    database: String,
    namespace: String,
    connection_id: ConnectionId,
    created_at: Instant,
    expires_at: Instant,
    last_event_at: Option<Instant>,
    events_delivered: u64,
}

enum SubscriptionType {
    Point,
    Query,
    Namespace,
}

struct RegistryConfig {
    // Maximum subscriptions per connection
    max_subscriptions_per_connection: usize,  // Default: 100

    // Maximum total subscriptions per namespace
    max_subscriptions_per_namespace: usize,   // Default: 10,000

    // Maximum query watches (more expensive)
    max_query_watches_per_namespace: usize,   // Default: 1,000

    // Default subscription TTL
    default_ttl: Duration,                    // Default: 1 hour

    // Maximum subscription TTL
    max_ttl: Duration,                        // Default: 24 hours

    // Heartbeat interval
    heartbeat_interval: Duration,             // Default: 30 seconds
}
```

#### 18.7.2 Subscription Lifecycle

```rust
impl SubscriptionRegistry {
    async fn create_subscription(
        &self,
        request: &WatchRequest,
        connection_id: ConnectionId,
    ) -> Result<(SubscriptionId, mpsc::Receiver<WatchEvent>)> {
        // Check resource limits
        self.check_limits(connection_id, &request).await?;

        // Generate subscription ID
        let subscription_id = SubscriptionId::new();

        // Create channel
        let (sender, receiver) = mpsc::channel(1000);

        // Get or create dispatcher for namespace
        let dispatcher = self.get_or_create_dispatcher(&request.namespace).await?;

        // Register subscription with dispatcher
        match &request {
            WatchRequest::Point(req) => {
                dispatcher.register_point_watch(subscription_id, req, sender).await?;
            }
            WatchRequest::Query(req) => {
                dispatcher.register_query_watch(subscription_id, req, sender).await?;
            }
            WatchRequest::Namespace(req) => {
                dispatcher.register_namespace_watch(subscription_id, req, sender).await?;
            }
        }

        // Track in registry
        let meta = SubscriptionMeta {
            subscription_id,
            subscription_type: request.subscription_type(),
            database: request.database.clone(),
            namespace: request.namespace.clone(),
            connection_id,
            created_at: Instant::now(),
            expires_at: Instant::now() + self.compute_ttl(request),
            last_event_at: None,
            events_delivered: 0,
        };

        self.subscriptions.write().await.insert(subscription_id, meta);
        self.connections.write().await
            .entry(connection_id)
            .or_default()
            .push(subscription_id);

        Ok((subscription_id, receiver))
    }

    async fn cancel_subscription(&self, subscription_id: SubscriptionId) -> Result<bool> {
        let meta = self.subscriptions.write().await.remove(&subscription_id);

        if let Some(meta) = meta {
            // Remove from dispatcher
            let dispatcher = self.dispatchers.read().await.get(&meta.namespace).cloned();
            if let Some(dispatcher) = dispatcher {
                dispatcher.unregister(subscription_id).await;
            }

            // Remove from connection tracking
            if let Some(subs) = self.connections.write().await.get_mut(&meta.connection_id) {
                subs.retain(|&id| id != subscription_id);
            }

            return Ok(true);
        }

        Ok(false)
    }

    async fn cleanup_connection(&self, connection_id: ConnectionId) {
        let sub_ids = self.connections.write().await.remove(&connection_id);

        if let Some(sub_ids) = sub_ids {
            for sub_id in sub_ids {
                let _ = self.cancel_subscription(sub_id).await;
            }
        }
    }
}
```

### 18.8 Client Reconnection and Resume Semantics

#### 18.8.1 Position Tracking

Every `WatchEvent` includes a `position` field (opaque bytes containing the versionstamp). Clients store this position to resume after disconnection.

```protobuf
message WatchEvent {
  // Position for resume (opaque, pass back in resume_position)
  bytes position = 2;
  // ...
}
```

#### 18.8.2 Resume Protocol

```
Client                                Server
  │                                     │
  │  WatchPointRequest                  │
  │  { resume_position: <empty> }       │
  │────────────────────────────────────>│
  │                                     │  Stream starts from "now"
  │                                     │
  │<────────────────────────────────────│  WatchEvent { position: vs_1000 }
  │<────────────────────────────────────│  WatchEvent { position: vs_1001 }
  │                                     │
  │        (connection drops)           │
  │                                     │
  │  WatchPointRequest                  │
  │  { resume_position: vs_1001 }       │
  │────────────────────────────────────>│
  │                                     │  Validate position exists in CDC
  │                                     │  Stream from vs_1001 (exclusive)
  │                                     │
  │<────────────────────────────────────│  WatchEvent { position: vs_1002 }
  │<────────────────────────────────────│  WatchEvent { position: vs_1003 }
```

#### 18.8.3 Resume Validation

```rust
async fn validate_resume_position(
    db: &FdbDatabase,
    namespace: &str,
    position: &Versionstamp,
) -> Result<ResumeStatus> {
    let tx = db.create_transaction()?;

    // Check if position exists in CDC log
    let key = (db_id, namespace, "_cdc", position);
    if tx.get(&key).await?.is_some() {
        return Ok(ResumeStatus::Valid);
    }

    // Check if position is before CDC retention window
    let oldest_key = (db_id, namespace, "_cdc");
    let oldest = tx.get_range(&oldest_key, &oldest_key.range_end(), RangeOption {
        limit: 1,
        ..default()
    }).await?;

    if let Some((oldest_key, _)) = oldest.first() {
        let oldest_vs = extract_versionstamp(oldest_key);
        if position < &oldest_vs {
            return Ok(ResumeStatus::Expired {
                oldest_available: oldest_vs,
            });
        }
    }

    // Position is in future or CDC log is empty
    Ok(ResumeStatus::StartFromNow)
}

enum ResumeStatus {
    /// Position found, can resume
    Valid,

    /// Position expired (before retention window)
    Expired { oldest_available: Versionstamp },

    /// Start from current position
    StartFromNow,
}
```

#### 18.8.4 Handling Expired Positions

When a resume position is expired:

```protobuf
message WatchEvent {
  // ...
  WatchEventType event_type = 3;  // WATCH_EVENT_TYPE_SUBSCRIPTION_ENDED
  SubscriptionEndReason end_reason = 12;
}

enum SubscriptionEndReason {
  SUBSCRIPTION_END_REASON_UNSPECIFIED = 0;
  SUBSCRIPTION_END_REASON_CLIENT_CANCELLED = 1;
  SUBSCRIPTION_END_REASON_TTL_EXPIRED = 2;
  SUBSCRIPTION_END_REASON_POSITION_EXPIRED = 3;   // Resume position too old
  SUBSCRIPTION_END_REASON_RESOURCE_LIMIT = 4;
  SUBSCRIPTION_END_REASON_SERVER_SHUTDOWN = 5;
}
```

Clients receiving `POSITION_EXPIRED` should:

1. Start a new subscription without `resume_position` (fresh start)
2. If maintaining a projection, perform a full re-sync

### 18.9 Resource Management

#### 18.9.1 Subscription Limits

| Resource | Limit | Scope | Behavior |
|----------|-------|-------|----------|
| Subscriptions per connection | 100 | Connection | New subscription rejected |
| Total subscriptions per namespace | 10,000 | Namespace | New subscription rejected |
| Query watches per namespace | 1,000 | Namespace | Query watch rejected |
| Matching nodes per query watch | 10,000 | Subscription | Events dropped, warning emitted |
| Event queue depth | 1,000 | Subscription | Backpressure, events dropped |

#### 18.9.2 Subscription TTL

All subscriptions have a TTL after which they expire automatically:

```rust
struct TtlManager {
    registry: Arc<SubscriptionRegistry>,
}

impl TtlManager {
    async fn run(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(60));

        loop {
            interval.tick().await;

            let now = Instant::now();
            let expired: Vec<SubscriptionId> = {
                let subs = self.registry.subscriptions.read().await;
                subs.iter()
                    .filter(|(_, meta)| meta.expires_at <= now)
                    .map(|(id, _)| *id)
                    .collect()
            };

            for sub_id in expired {
                // Send termination event
                if let Some(sender) = self.registry.get_sender(sub_id).await {
                    let _ = sender.send(WatchEvent {
                        event_type: WatchEventType::SubscriptionEnded,
                        end_reason: Some(SubscriptionEndReason::TtlExpired),
                        ..default()
                    }).await;
                }

                // Clean up
                self.registry.cancel_subscription(sub_id).await;
            }
        }
    }
}
```

#### 18.9.3 Heartbeats

Heartbeats serve two purposes:

1. Keep connections alive through load balancers and proxies
2. Allow clients to detect server-side failures

```rust
async fn heartbeat_loop(
    sender: &mpsc::Sender<WatchEvent>,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;

        let event = WatchEvent {
            event_type: WatchEventType::Heartbeat,
            timestamp: current_time_micros(),
            ..default()
        };

        if sender.send(event).await.is_err() {
            break; // Client disconnected
        }
    }
}
```

#### 18.9.4 Backpressure and Event Dropping

When a client cannot keep up with events:

```rust
async fn send_event(
    sender: &mpsc::Sender<WatchEvent>,
    event: WatchEvent,
    subscription_id: SubscriptionId,
    metrics: &WatchMetrics,
) {
    match sender.try_send(event) {
        Ok(_) => {
            metrics.events_delivered.inc();
        }
        Err(mpsc::error::TrySendError::Full(_)) => {
            metrics.events_dropped.inc();
            log::warn!(
                "Dropping event for subscription {}: client not consuming",
                subscription_id
            );
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            // Client disconnected, will be cleaned up
        }
    }
}
```

### 18.10 Complete Proto Definitions

```protobuf
syntax = "proto3";
package pelago.v1;

import "google/protobuf/struct.proto";

// ═══════════════════════════════════════════════════════════════════════════
// WATCH SERVICE
// ═══════════════════════════════════════════════════════════════════════════

service WatchService {
  // Subscribe to changes on specific nodes, edges, or properties
  rpc WatchPoint(WatchPointRequest) returns (stream WatchEvent);

  // Subscribe to changes matching a CEL or PQL query
  rpc WatchQuery(WatchQueryRequest) returns (stream WatchEvent);

  // Subscribe to all changes within a namespace
  rpc WatchNamespace(WatchNamespaceRequest) returns (stream WatchEvent);

  // List active subscriptions for this connection
  rpc ListSubscriptions(ListSubscriptionsRequest) returns (ListSubscriptionsResponse);

  // Cancel a subscription by ID
  rpc CancelSubscription(CancelSubscriptionRequest) returns (CancelSubscriptionResponse);
}

// ═══════════════════════════════════════════════════════════════════════════
// POINT WATCH
// ═══════════════════════════════════════════════════════════════════════════

message WatchPointRequest {
  RequestContext context = 1;

  // What to watch (at least one required)
  repeated NodeWatch nodes = 2;
  repeated EdgeWatch edges = 3;

  // Resume position (optional, empty = start from now)
  bytes resume_position = 4;

  // Subscription options
  WatchOptions options = 5;
}

message NodeWatch {
  string entity_type = 1;
  string node_id = 2;

  // Optional: watch only specific properties (empty = all)
  repeated string properties = 3;
}

message EdgeWatch {
  NodeRef source = 1;
  string label = 2;

  // Optional: watch specific target (empty = all edges with label)
  NodeRef target = 3;

  // Optional: watch edge direction
  EdgeDirection direction = 4;
}

// ═══════════════════════════════════════════════════════════════════════════
// QUERY WATCH
// ═══════════════════════════════════════════════════════════════════════════

message WatchQueryRequest {
  RequestContext context = 1;

  // Query specification (exactly one required)
  oneof query {
    // CEL expression (e.g., "entity_type == 'Person' && age >= 30")
    string cel_expression = 2;

    // PQL query string
    string pql_query = 3;
  }

  // Entity type scope (required for CEL, optional for PQL)
  string entity_type = 4;

  // Include full entity data in events (default: true)
  bool include_data = 5;

  // Resume position
  bytes resume_position = 6;

  // Subscription options
  WatchOptions options = 7;
}

// ═══════════════════════════════════════════════════════════════════════════
// NAMESPACE WATCH
// ═══════════════════════════════════════════════════════════════════════════

message WatchNamespaceRequest {
  RequestContext context = 1;

  // Optional: filter by entity types (empty = all types)
  repeated string entity_types = 2;

  // Optional: filter by operation types
  repeated OperationType operation_types = 3;

  // Resume position
  bytes resume_position = 4;

  // Subscription options
  WatchOptions options = 5;
}

enum OperationType {
  OPERATION_TYPE_UNSPECIFIED = 0;
  OPERATION_TYPE_NODE_CREATE = 1;
  OPERATION_TYPE_NODE_UPDATE = 2;
  OPERATION_TYPE_NODE_DELETE = 3;
  OPERATION_TYPE_EDGE_CREATE = 4;
  OPERATION_TYPE_EDGE_DELETE = 5;
  OPERATION_TYPE_SCHEMA_REGISTER = 6;
}

// ═══════════════════════════════════════════════════════════════════════════
// WATCH OPTIONS
// ═══════════════════════════════════════════════════════════════════════════

message WatchOptions {
  // Subscription TTL in seconds (0 = use server default)
  uint32 ttl_seconds = 1;

  // Heartbeat interval in seconds (0 = use server default)
  uint32 heartbeat_seconds = 2;

  // Maximum events per second (0 = unlimited)
  uint32 rate_limit = 3;

  // Batch events before sending (reduces RPCs, increases latency)
  BatchOptions batch = 4;

  // Initial load: emit current state before streaming changes
  bool initial_snapshot = 5;
}

message BatchOptions {
  // Maximum events per batch
  uint32 max_events = 1;

  // Maximum time to wait for batch fill (milliseconds)
  uint32 max_wait_ms = 2;
}

// ═══════════════════════════════════════════════════════════════════════════
// WATCH EVENTS
// ═══════════════════════════════════════════════════════════════════════════

message WatchEvent {
  // Unique event ID (versionstamp from CDC)
  bytes event_id = 1;

  // Position for resume (opaque, pass back in resume_position)
  bytes position = 2;

  // Event type
  WatchEventType event_type = 3;

  // Timestamp (unix microseconds)
  int64 timestamp = 4;

  // Subscription that matched this event
  string subscription_id = 5;

  // Entity information
  string entity_type = 6;
  string node_id = 7;

  // Optional: full entity data (if requested)
  map<string, Value> data = 8;

  // Optional: changed properties only (for updates)
  map<string, Value> changed_properties = 9;

  // Optional: edge information (for edge events)
  EdgeEventData edge = 10;

  // For query watches: entry/exit metadata
  QueryWatchMeta query_meta = 11;

  // For subscription end events
  SubscriptionEndReason end_reason = 12;
}

enum WatchEventType {
  WATCH_EVENT_TYPE_UNSPECIFIED = 0;

  // Node events
  WATCH_EVENT_TYPE_NODE_CREATED = 1;
  WATCH_EVENT_TYPE_NODE_UPDATED = 2;
  WATCH_EVENT_TYPE_NODE_DELETED = 3;

  // Edge events
  WATCH_EVENT_TYPE_EDGE_CREATED = 4;
  WATCH_EVENT_TYPE_EDGE_DELETED = 5;

  // Query watch specific
  WATCH_EVENT_TYPE_QUERY_ENTER = 6;
  WATCH_EVENT_TYPE_QUERY_EXIT = 7;

  // System events
  WATCH_EVENT_TYPE_HEARTBEAT = 8;
  WATCH_EVENT_TYPE_SUBSCRIPTION_ENDED = 9;
}

message EdgeEventData {
  NodeRef source = 1;
  NodeRef target = 2;
  string label = 3;
  map<string, Value> properties = 4;
}

message QueryWatchMeta {
  // For QUERY_EXIT: why the entity exited
  ExitReason exit_reason = 1;

  // For QUERY_ENTER: was this initial load or live change
  bool initial_load = 2;
}

enum ExitReason {
  EXIT_REASON_UNSPECIFIED = 0;
  EXIT_REASON_DELETED = 1;
  EXIT_REASON_NO_LONGER_MATCHES = 2;
}

enum SubscriptionEndReason {
  SUBSCRIPTION_END_REASON_UNSPECIFIED = 0;
  SUBSCRIPTION_END_REASON_CLIENT_CANCELLED = 1;
  SUBSCRIPTION_END_REASON_TTL_EXPIRED = 2;
  SUBSCRIPTION_END_REASON_POSITION_EXPIRED = 3;
  SUBSCRIPTION_END_REASON_RESOURCE_LIMIT = 4;
  SUBSCRIPTION_END_REASON_SERVER_SHUTDOWN = 5;
}

// ═══════════════════════════════════════════════════════════════════════════
// SUBSCRIPTION MANAGEMENT
// ═══════════════════════════════════════════════════════════════════════════

message ListSubscriptionsRequest {
  RequestContext context = 1;
}

message ListSubscriptionsResponse {
  repeated SubscriptionInfo subscriptions = 1;
}

message SubscriptionInfo {
  string subscription_id = 1;
  SubscriptionType type = 2;
  string database = 3;
  string namespace = 4;
  int64 created_at = 5;
  int64 expires_at = 6;
  uint64 events_delivered = 7;
}

enum SubscriptionType {
  SUBSCRIPTION_TYPE_UNSPECIFIED = 0;
  SUBSCRIPTION_TYPE_POINT = 1;
  SUBSCRIPTION_TYPE_QUERY = 2;
  SUBSCRIPTION_TYPE_NAMESPACE = 3;
}

message CancelSubscriptionRequest {
  RequestContext context = 1;
  string subscription_id = 2;
}

message CancelSubscriptionResponse {
  bool cancelled = 1;
}
```

### 18.11 Error Codes

#### Watch Errors (ERR_WATCH_*)

| Code | Message | gRPC Code | Details |
|------|---------|-----------|---------|
| `ERR_WATCH_SUBSCRIPTION_LIMIT` | Maximum subscriptions reached | `RESOURCE_EXHAUSTED` | limit, current_count |
| `ERR_WATCH_QUERY_LIMIT` | Maximum query watches reached | `RESOURCE_EXHAUSTED` | limit, current_count |
| `ERR_WATCH_POSITION_EXPIRED` | Resume position expired | `OUT_OF_RANGE` | requested_position, oldest_available |
| `ERR_WATCH_INVALID_QUERY` | Invalid CEL or PQL query | `INVALID_ARGUMENT` | expression, error |
| `ERR_WATCH_QUERY_TOO_COMPLEX` | Query exceeds complexity limit | `INVALID_ARGUMENT` | complexity, limit |
| `ERR_WATCH_SUBSCRIPTION_NOT_FOUND` | Subscription ID not found | `NOT_FOUND` | subscription_id |
| `ERR_WATCH_TTL_EXCEEDED` | Requested TTL exceeds maximum | `INVALID_ARGUMENT` | requested, maximum |

### 18.12 Configuration

#### Server Configuration

| Environment Variable | CLI Flag | Default | Description |
|---------------------|----------|---------|-------------|
| `PELAGO_WATCH_MAX_SUBS_CONNECTION` | `--watch-max-subs-conn` | `100` | Max subscriptions per connection |
| `PELAGO_WATCH_MAX_SUBS_NAMESPACE` | `--watch-max-subs-ns` | `10000` | Max subscriptions per namespace |
| `PELAGO_WATCH_MAX_QUERY_WATCHES` | `--watch-max-queries` | `1000` | Max query watches per namespace |
| `PELAGO_WATCH_DEFAULT_TTL` | `--watch-default-ttl` | `3600` | Default TTL in seconds |
| `PELAGO_WATCH_MAX_TTL` | `--watch-max-ttl` | `86400` | Maximum TTL in seconds |
| `PELAGO_WATCH_HEARTBEAT_INTERVAL` | `--watch-heartbeat` | `30` | Heartbeat interval in seconds |
| `PELAGO_WATCH_EVENT_QUEUE_SIZE` | `--watch-queue-size` | `1000` | Event queue depth per subscription |

### 18.13 Use Case Examples

#### 18.13.1 Reactive UI: Watch a User Profile

```rust
// Client code
let mut stream = client.watch_point(WatchPointRequest {
    context: Some(RequestContext {
        database: "production".into(),
        namespace: "users".into(),
        ..default()
    }),
    nodes: vec![NodeWatch {
        entity_type: "User".into(),
        node_id: "u_12345".into(),
        properties: vec!["name".into(), "avatar".into(), "status".into()],
    }],
    ..default()
}).await?;

while let Some(event) = stream.message().await? {
    match event.event_type() {
        WatchEventType::NodeUpdated => {
            // Update UI with changed properties
            update_profile_ui(&event.changed_properties);
        }
        WatchEventType::NodeDeleted => {
            // User was deleted, show error
            show_user_deleted_error();
            break;
        }
        WatchEventType::Heartbeat => {
            // Connection alive
        }
        _ => {}
    }
}
```

#### 18.13.2 Materialized View: Active Orders

```rust
// Project all active orders to Redis
let mut stream = client.watch_query(WatchQueryRequest {
    context: Some(RequestContext {
        database: "production".into(),
        namespace: "orders".into(),
        ..default()
    }),
    cel_expression: Some("status == 'active' || status == 'processing'".into()),
    entity_type: "Order".into(),
    include_data: true,
    initial_snapshot: true, // Get current active orders first
    ..default()
}).await?;

while let Some(event) = stream.message().await? {
    match event.event_type() {
        WatchEventType::QueryEnter => {
            // New active order
            redis.hset("active_orders", &event.node_id, &serialize(&event.data));
        }
        WatchEventType::NodeUpdated => {
            // Order updated
            redis.hset("active_orders", &event.node_id, &serialize(&event.data));
        }
        WatchEventType::QueryExit => {
            // Order no longer active
            redis.hdel("active_orders", &event.node_id);
        }
        _ => {}
    }
}
```

#### 18.13.3 Coordination: Wait for Approval

```rust
// Wait for a document to be approved
async fn wait_for_approval(doc_id: &str, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;

    let mut stream = client.watch_point(WatchPointRequest {
        context: Some(RequestContext {
            database: "production".into(),
            namespace: "documents".into(),
            ..default()
        }),
        nodes: vec![NodeWatch {
            entity_type: "Document".into(),
            node_id: doc_id.into(),
            properties: vec!["status".into()],
        }],
        options: Some(WatchOptions {
            ttl_seconds: timeout.as_secs() as u32,
            ..default()
        }),
        ..default()
    }).await?;

    while Instant::now() < deadline {
        match tokio::time::timeout(
            deadline - Instant::now(),
            stream.message()
        ).await {
            Ok(Some(event)) => {
                if let Some(status) = event.changed_properties.get("status") {
                    if status.string_value == "approved" {
                        return Ok(true);
                    }
                    if status.string_value == "rejected" {
                        return Ok(false);
                    }
                }
            }
            Ok(None) => break,    // Stream ended
            Err(_) => break,       // Timeout
        }
    }

    Err(Error::Timeout)
}
```

#### 18.13.4 Audit Log: Namespace Watch

```rust
// Stream all changes to Kafka for audit
let mut stream = client.watch_namespace(WatchNamespaceRequest {
    context: Some(RequestContext {
        database: "production".into(),
        namespace: "financial".into(),
        ..default()
    }),
    // Watch all operations
    operation_types: vec![],
    // Resume from last position (stored in Kafka consumer group)
    resume_position: load_kafka_offset().await?,
    ..default()
}).await?;

while let Some(event) = stream.message().await? {
    if event.event_type() == WatchEventType::Heartbeat {
        continue;
    }

    // Transform to audit format
    let audit_record = AuditRecord {
        timestamp: event.timestamp,
        operation: format!("{:?}", event.event_type()),
        entity_type: event.entity_type,
        entity_id: event.node_id,
        data: event.data,
    };

    // Publish to Kafka
    kafka_producer.send(
        "audit.financial",
        &audit_record,
    ).await?;

    // Store position for resume
    save_kafka_offset(&event.position).await?;
}
```

---
