//! PelagoDB Query System
//!
//! This crate provides query capabilities:
//! - CEL expression parsing and type-checking
//! - Query planning with index selection
//! - Query execution with streaming results
//! - Multi-hop graph traversal

pub mod cel;
pub mod executor;
pub mod plan;
pub mod planner;
pub mod traversal;

// Re-export main types
pub use cel::{CelEnvironment, CelExpression};
pub use executor::{ExecutorConfig, QueryExecutor, QueryResults, QueryResultStream};
pub use plan::{ExecutionPlan, IndexPredicate, PredicateValue, QueryExplanation, QueryPlan, RangeBound};
pub use planner::QueryPlanner;
pub use traversal::{
    PathHop, TraversalConfig, TraversalDirection, TraversalEngine, TraversalHop,
    TraversalPath, TraversalResults, TraversalStream,
};
