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
