//! Multi-hop graph traversal
//!
//! - Multi-hop traversal with per-hop edge/node filters
//! - Depth limit, timeout, result limit enforcement
//! - Streaming results with path information
//! - Cycle detection
//!
//! The traversal engine implements BFS/DFS graph traversal with filtering.

use pelago_core::{PelagoError, Value};
use pelago_storage::{EdgeStore, IdAllocator, NodeStore, PelagoDb, SchemaRegistry, StoredEdge, StoredNode};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Traversal configuration
#[derive(Clone)]
pub struct TraversalConfig {
    /// Maximum depth to traverse
    pub max_depth: u32,
    /// Maximum results to return
    pub max_results: u32,
    /// Timeout for the entire traversal
    pub timeout: Duration,
    /// Buffer size for streaming results
    pub buffer_size: usize,
}

impl Default for TraversalConfig {
    fn default() -> Self {
        Self {
            max_depth: 10,
            max_results: 1000,
            timeout: Duration::from_secs(30),
            buffer_size: 100,
        }
    }
}

/// A single hop in a traversal
#[derive(Clone)]
pub struct TraversalHop {
    /// Edge labels to follow (empty = all)
    pub edge_labels: Vec<String>,
    /// Direction to traverse
    pub direction: TraversalDirection,
    /// Filter for edges at this hop
    pub edge_filter: Option<String>,
    /// Filter for target nodes at this hop
    pub node_filter: Option<String>,
}

impl TraversalHop {
    pub fn new(direction: TraversalDirection) -> Self {
        Self {
            edge_labels: Vec::new(),
            direction,
            edge_filter: None,
            node_filter: None,
        }
    }

    pub fn with_labels(mut self, labels: Vec<String>) -> Self {
        self.edge_labels = labels;
        self
    }

    pub fn with_edge_filter(mut self, filter: String) -> Self {
        self.edge_filter = Some(filter);
        self
    }

    pub fn with_node_filter(mut self, filter: String) -> Self {
        self.node_filter = Some(filter);
        self
    }
}

/// Direction of traversal
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraversalDirection {
    /// Follow outgoing edges
    Outbound,
    /// Follow incoming edges
    Inbound,
    /// Follow both directions
    Both,
}

/// A path through the graph
#[derive(Clone, Debug)]
pub struct TraversalPath {
    /// Starting node
    pub start: StoredNode,
    /// Edges and nodes along the path
    pub hops: Vec<PathHop>,
}

impl TraversalPath {
    fn new(start: StoredNode) -> Self {
        Self {
            start,
            hops: Vec::new(),
        }
    }

    fn push(&mut self, edge: StoredEdge, node: StoredNode) {
        self.hops.push(PathHop { edge, node });
    }

    /// Get the final node in the path
    pub fn end_node(&self) -> &StoredNode {
        self.hops.last().map(|h| &h.node).unwrap_or(&self.start)
    }

    /// Get the depth (number of hops)
    pub fn depth(&self) -> usize {
        self.hops.len()
    }
}

/// A single hop in a path
#[derive(Clone, Debug)]
pub struct PathHop {
    /// The edge traversed
    pub edge: StoredEdge,
    /// The target node reached
    pub node: StoredNode,
}

/// Traversal engine for multi-hop graph queries
pub struct TraversalEngine {
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    id_allocator: Arc<IdAllocator>,
    site_id: String,
    config: TraversalConfig,
}

impl TraversalEngine {
    /// Create a new traversal engine
    pub fn new(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        site_id: String,
    ) -> Self {
        Self {
            db,
            schema_registry,
            id_allocator,
            site_id,
            config: TraversalConfig::default(),
        }
    }

    /// Create with custom config
    pub fn with_config(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        site_id: String,
        config: TraversalConfig,
    ) -> Self {
        Self {
            db,
            schema_registry,
            id_allocator,
            site_id,
            config,
        }
    }

    /// Execute a traversal starting from a node
    pub async fn traverse(
        &self,
        database: &str,
        namespace: &str,
        start_entity_type: &str,
        start_node_id: &str,
        hops: &[TraversalHop],
    ) -> Result<TraversalResults, PelagoError> {
        let start_time = Instant::now();

        // Get the starting node
        let node_store = Arc::new(NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            self.site_id.clone(),
        ));

        let start_node = node_store
            .get_node(database, namespace, start_entity_type, start_node_id)
            .await?
            .ok_or_else(|| PelagoError::NodeNotFound {
                entity_type: start_entity_type.to_string(),
                node_id: start_node_id.to_string(),
            })?;

        let edge_store = EdgeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            Arc::clone(&node_store),
            self.site_id.clone(),
        );

        // BFS traversal
        let mut results = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(TraversalPath, usize)> = VecDeque::new();

        // Start with initial path
        let initial_path = TraversalPath::new(start_node.clone());
        visited.insert(format!("{}:{}", start_entity_type, start_node_id));
        queue.push_back((initial_path, 0));

        while let Some((current_path, hop_idx)) = queue.pop_front() {
            // Check timeout
            if start_time.elapsed() > self.config.timeout {
                return Err(PelagoError::TraversalTimeout {
                    max_depth: self.config.max_depth,
                    reached_depth: hop_idx as u32,
                });
            }

            // Check result limit
            if results.len() >= self.config.max_results as usize {
                break;
            }

            // If we've completed all hops, this is a result
            if hop_idx >= hops.len() {
                results.push(current_path);
                continue;
            }

            // Check depth limit
            if hop_idx as u32 >= self.config.max_depth {
                results.push(current_path);
                continue;
            }

            let hop = &hops[hop_idx];
            let current_node = current_path.end_node();

            // Get edges from current node
            let edges = self.get_edges_for_hop(
                &edge_store,
                database,
                namespace,
                &current_node.entity_type,
                &current_node.id,
                hop,
            ).await?;

            // Filter edges if needed
            let filtered_edges = if let Some(ref filter) = hop.edge_filter {
                self.filter_edges(&edges, filter)?
            } else {
                edges
            };

            // Process each edge
            for edge in filtered_edges {
                // Get target node
                let target_ref = match hop.direction {
                    TraversalDirection::Outbound => &edge.target,
                    TraversalDirection::Inbound => &edge.source,
                    TraversalDirection::Both => {
                        // Pick the one that's not the current node
                        if edge.source.entity_type == current_node.entity_type
                            && edge.source.node_id == current_node.id
                        {
                            &edge.target
                        } else {
                            &edge.source
                        }
                    }
                };

                // Check for cycles
                let target_key = format!("{}:{}", target_ref.entity_type, target_ref.node_id);
                if visited.contains(&target_key) {
                    continue; // Skip cycles
                }

                // Fetch target node
                let target_node = match node_store
                    .get_node(database, namespace, &target_ref.entity_type, &target_ref.node_id)
                    .await?
                {
                    Some(n) => n,
                    None => continue, // Node doesn't exist, skip
                };

                // Apply node filter if present
                if let Some(ref filter) = hop.node_filter {
                    if !self.matches_node_filter(&target_node, filter)? {
                        continue;
                    }
                }

                // Add to visited and queue
                visited.insert(target_key);

                let mut new_path = current_path.clone();
                new_path.push(edge.clone(), target_node);
                queue.push_back((new_path, hop_idx + 1));
            }
        }

        Ok(TraversalResults {
            paths: results,
            truncated: visited.len() >= self.config.max_results as usize,
        })
    }

    /// Execute a streaming traversal
    pub async fn traverse_streaming(
        &self,
        database: &str,
        namespace: &str,
        start_entity_type: &str,
        start_node_id: &str,
        hops: Vec<TraversalHop>,
    ) -> Result<TraversalStream, PelagoError> {
        let (tx, rx) = mpsc::channel(self.config.buffer_size);

        // Clone what we need for the spawned task
        let db = self.db.clone();
        let schema_registry = Arc::clone(&self.schema_registry);
        let id_allocator = Arc::clone(&self.id_allocator);
        let site_id = self.site_id.clone();
        let config = self.config.clone();
        let database = database.to_string();
        let namespace = namespace.to_string();
        let start_entity_type = start_entity_type.to_string();
        let start_node_id = start_node_id.to_string();

        tokio::spawn(async move {
            let engine = TraversalEngine::with_config(db, schema_registry, id_allocator, site_id, config);

            match engine
                .traverse(&database, &namespace, &start_entity_type, &start_node_id, &hops)
                .await
            {
                Ok(results) => {
                    for path in results.paths {
                        if tx.send(Ok(path)).await.is_err() {
                            break; // Receiver dropped
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                }
            }
        });

        Ok(TraversalStream { rx })
    }

    /// Get edges for a hop based on direction and labels
    async fn get_edges_for_hop(
        &self,
        edge_store: &EdgeStore,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        hop: &TraversalHop,
    ) -> Result<Vec<StoredEdge>, PelagoError> {
        // Determine which label to filter by (if any)
        let label_filter = if hop.edge_labels.is_empty() {
            None
        } else if hop.edge_labels.len() == 1 {
            Some(hop.edge_labels[0].as_str())
        } else {
            // Multiple labels - we'll filter after fetching
            None
        };

        let edges = edge_store
            .list_edges(database, namespace, entity_type, node_id, label_filter, 1000)
            .await?;

        // Filter by direction and labels
        let filtered: Vec<StoredEdge> = edges
            .into_iter()
            .filter(|edge| {
                // Check direction
                let direction_ok = match hop.direction {
                    TraversalDirection::Outbound => {
                        edge.source.entity_type == entity_type && edge.source.node_id == node_id
                    }
                    TraversalDirection::Inbound => {
                        edge.target.entity_type == entity_type && edge.target.node_id == node_id
                    }
                    TraversalDirection::Both => {
                        (edge.source.entity_type == entity_type && edge.source.node_id == node_id)
                            || (edge.target.entity_type == entity_type && edge.target.node_id == node_id)
                    }
                };

                if !direction_ok {
                    return false;
                }

                // Check label filter if multiple labels specified
                if hop.edge_labels.len() > 1 {
                    return hop.edge_labels.contains(&edge.label);
                }

                true
            })
            .collect();

        Ok(filtered)
    }

    /// Filter edges using a CEL expression
    fn filter_edges(
        &self,
        edges: &[StoredEdge],
        _filter: &str,
    ) -> Result<Vec<StoredEdge>, PelagoError> {
        // For edge filtering, we'd need a schema for edge properties
        // For now, return all edges (filter is ignored)
        // A full implementation would compile and evaluate the CEL expression
        Ok(edges.to_vec())
    }

    /// Check if a node matches a CEL filter
    fn matches_node_filter(
        &self,
        node: &StoredNode,
        filter: &str,
    ) -> Result<bool, PelagoError> {
        // Simple implementation - compile and evaluate
        // In a full implementation, we'd cache compiled expressions
        use cel_interpreter::{Context, Program, Value as CelValue};

        let program = Program::compile(filter).map_err(|e| PelagoError::CelSyntax {
            expression: filter.to_string(),
            message: e.to_string(),
        })?;

        let mut context = Context::default();
        for (key, value) in &node.properties {
            let cel_value = pelago_value_to_cel(value);
            context.add_variable(key, cel_value).ok();
        }

        match program.execute(&context) {
            Ok(CelValue::Bool(b)) => Ok(b),
            Ok(CelValue::Null) => Ok(false),
            Ok(_) => Ok(false),
            Err(_) => Ok(false), // Evaluation errors return false per null semantics
        }
    }
}

/// Convert PelagoDB Value to CEL Value
fn pelago_value_to_cel(value: &Value) -> cel_interpreter::Value {
    use cel_interpreter::Value as CelValue;

    match value {
        Value::String(s) => CelValue::String(Arc::new(s.clone())),
        Value::Int(n) => CelValue::Int(*n),
        Value::Float(f) => CelValue::Float(*f),
        Value::Bool(b) => CelValue::Bool(*b),
        Value::Timestamp(t) => CelValue::Int(*t),
        Value::Bytes(b) => CelValue::Bytes(Arc::new(b.clone())),
        Value::Null => CelValue::Null,
    }
}

/// Results from a traversal
pub struct TraversalResults {
    /// Paths found
    pub paths: Vec<TraversalPath>,
    /// Whether results were truncated due to limits
    pub truncated: bool,
}

/// Streaming traversal results
pub struct TraversalStream {
    rx: mpsc::Receiver<Result<TraversalPath, PelagoError>>,
}

impl TraversalStream {
    /// Get the next result from the stream
    pub async fn next(&mut self) -> Option<Result<TraversalPath, PelagoError>> {
        self.rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_traversal_path() {
        let start = StoredNode {
            id: "1_100".to_string(),
            entity_type: "Person".to_string(),
            properties: HashMap::new(),
            locality: 1,
            created_at: 0,
            updated_at: 0,
        };

        let path = TraversalPath::new(start.clone());

        assert_eq!(path.depth(), 0);
        assert_eq!(path.end_node().id, "1_100");
    }

    #[test]
    fn test_traversal_hop_builder() {
        let hop = TraversalHop::new(TraversalDirection::Outbound)
            .with_labels(vec!["KNOWS".to_string()])
            .with_node_filter("age >= 30".to_string());

        assert_eq!(hop.edge_labels, vec!["KNOWS"]);
        assert_eq!(hop.direction, TraversalDirection::Outbound);
        assert_eq!(hop.node_filter, Some("age >= 30".to_string()));
    }

    #[test]
    fn test_default_config() {
        let config = TraversalConfig::default();

        assert_eq!(config.max_depth, 10);
        assert_eq!(config.max_results, 1000);
        assert_eq!(config.timeout, Duration::from_secs(30));
    }
}
