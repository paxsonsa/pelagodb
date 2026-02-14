//! Query plan types
//!
//! Query plans describe how to execute a query:
//! - PointLookup: Direct fetch by ID
//! - IndexScan: Use an index to find matching nodes
//! - FullScan: Scan all nodes of a type
//!
//! ExecutionPlan combines a primary plan with residual filtering,
//! projection, limits, and cursor state.

use pelago_core::schema::IndexType;
use std::collections::HashSet;

/// Primary query strategy
#[derive(Debug, Clone, PartialEq)]
pub enum QueryPlan {
    /// Direct lookup by node ID
    PointLookup {
        node_id: String,
    },
    /// Index scan (unique, equality, or range)
    IndexScan {
        property: String,
        index_type: IndexType,
        predicate: IndexPredicate,
    },
    /// Full scan of all nodes
    FullScan,
}

impl QueryPlan {
    /// Estimated selectivity for query planning
    pub fn selectivity(&self) -> f64 {
        match self {
            QueryPlan::PointLookup { .. } => 0.001, // Very selective
            QueryPlan::IndexScan { index_type, predicate, .. } => {
                let base = index_type.selectivity();
                // Range predicates are less selective than equality
                match predicate {
                    IndexPredicate::Equals(_) => base,
                    IndexPredicate::Range { .. } => base * 2.0,
                    IndexPredicate::In(values) => base * values.len() as f64,
                }
            }
            QueryPlan::FullScan => 1.0, // Least selective
        }
    }

    pub fn strategy_name(&self) -> &'static str {
        match self {
            QueryPlan::PointLookup { .. } => "point_lookup",
            QueryPlan::IndexScan { .. } => "index_scan",
            QueryPlan::FullScan => "full_scan",
        }
    }
}

/// Index predicate for scans
#[derive(Debug, Clone, PartialEq)]
pub enum IndexPredicate {
    /// Exact match: field == value
    Equals(PredicateValue),
    /// Range: field > lower AND field < upper
    Range {
        lower: Option<RangeBound>,
        upper: Option<RangeBound>,
    },
    /// IN clause: field IN (v1, v2, v3)
    In(Vec<PredicateValue>),
}

/// A value in a predicate
#[derive(Debug, Clone, PartialEq)]
pub enum PredicateValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Timestamp(i64),
}

impl PredicateValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            PredicateValue::String(_) => "string",
            PredicateValue::Int(_) => "int",
            PredicateValue::Float(_) => "float",
            PredicateValue::Bool(_) => "bool",
            PredicateValue::Timestamp(_) => "timestamp",
        }
    }
}

/// Range bound (inclusive or exclusive)
#[derive(Debug, Clone, PartialEq)]
pub struct RangeBound {
    pub value: PredicateValue,
    pub inclusive: bool,
}

/// Complete execution plan
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    /// Entity type being queried
    pub entity_type: String,
    /// Primary access strategy
    pub primary_plan: QueryPlan,
    /// Residual CEL filter (applied after primary plan)
    pub residual_filter: Option<String>,
    /// Fields to project (None = all fields)
    pub projection: Option<HashSet<String>>,
    /// Maximum results to return
    pub limit: Option<u32>,
    /// Cursor for pagination
    pub cursor: Option<Vec<u8>>,
}

impl ExecutionPlan {
    pub fn new(entity_type: impl Into<String>, primary_plan: QueryPlan) -> Self {
        Self {
            entity_type: entity_type.into(),
            primary_plan,
            residual_filter: None,
            projection: None,
            limit: None,
            cursor: None,
        }
    }

    pub fn with_residual(mut self, filter: impl Into<String>) -> Self {
        self.residual_filter = Some(filter.into());
        self
    }

    pub fn with_projection(mut self, fields: HashSet<String>) -> Self {
        self.projection = Some(fields);
        self
    }

    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_cursor(mut self, cursor: Vec<u8>) -> Self {
        self.cursor = Some(cursor);
        self
    }
}

/// Explanation of a query plan (for EXPLAIN)
#[derive(Debug, Clone)]
pub struct QueryExplanation {
    pub strategy: String,
    pub steps: Vec<String>,
    pub estimated_cost: f64,
    pub estimated_rows: f64,
    pub index_used: Option<IndexUsed>,
    pub residual_filter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IndexUsed {
    pub property: String,
    pub index_type: String,
    pub operator: String,
    pub value: String,
}

impl QueryExplanation {
    pub fn from_plan(plan: &ExecutionPlan, total_rows_estimate: f64) -> Self {
        let estimated_rows = total_rows_estimate * plan.primary_plan.selectivity();

        let mut steps = Vec::new();
        let index_used;

        match &plan.primary_plan {
            QueryPlan::PointLookup { node_id } => {
                steps.push(format!("Point lookup for node '{}'", node_id));
                index_used = None;
            }
            QueryPlan::IndexScan { property, index_type, predicate } => {
                let (op, val) = match predicate {
                    IndexPredicate::Equals(v) => ("==".to_string(), format!("{:?}", v)),
                    IndexPredicate::Range { lower, upper } => {
                        let op = match (lower, upper) {
                            (Some(_), Some(_)) => "BETWEEN",
                            (Some(b), None) => if b.inclusive { ">=" } else { ">" },
                            (None, Some(b)) => if b.inclusive { "<=" } else { "<" },
                            (None, None) => "ANY",
                        };
                        (op.to_string(), "range".to_string())
                    }
                    IndexPredicate::In(values) => ("IN".to_string(), format!("({} values)", values.len())),
                };
                steps.push(format!("Index scan on {} {} {}", property, op, val));
                index_used = Some(IndexUsed {
                    property: property.clone(),
                    index_type: format!("{:?}", index_type),
                    operator: op,
                    value: val,
                });
            }
            QueryPlan::FullScan => {
                steps.push("Full table scan".to_string());
                index_used = None;
            }
        }

        if let Some(ref filter) = plan.residual_filter {
            steps.push(format!("Apply residual filter: {}", filter));
        }

        if let Some(ref proj) = plan.projection {
            steps.push(format!("Project fields: {:?}", proj));
        }

        if let Some(limit) = plan.limit {
            steps.push(format!("Limit to {} rows", limit));
        }

        // Simple cost model: rows * 0.001 for scans, + 0.01 per residual filter application
        let base_cost = estimated_rows * 0.001;
        let filter_cost = if plan.residual_filter.is_some() { estimated_rows * 0.01 } else { 0.0 };

        QueryExplanation {
            strategy: plan.primary_plan.strategy_name().to_string(),
            steps,
            estimated_cost: base_cost + filter_cost,
            estimated_rows,
            index_used,
            residual_filter: plan.residual_filter.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_plan_selectivity() {
        assert!(QueryPlan::PointLookup { node_id: "1".into() }.selectivity() < 0.01);

        let idx_scan = QueryPlan::IndexScan {
            property: "email".into(),
            index_type: IndexType::Unique,
            predicate: IndexPredicate::Equals(PredicateValue::String("a@b.com".into())),
        };
        assert!(idx_scan.selectivity() < 0.1);

        assert_eq!(QueryPlan::FullScan.selectivity(), 1.0);
    }

    #[test]
    fn test_execution_plan_builder() {
        let plan = ExecutionPlan::new("Person", QueryPlan::FullScan)
            .with_residual("age >= 30")
            .with_limit(100);

        assert_eq!(plan.entity_type, "Person");
        assert_eq!(plan.residual_filter, Some("age >= 30".into()));
        assert_eq!(plan.limit, Some(100));
    }

    #[test]
    fn test_query_explanation() {
        let plan = ExecutionPlan::new(
            "Person",
            QueryPlan::IndexScan {
                property: "age".into(),
                index_type: IndexType::Range,
                predicate: IndexPredicate::Range {
                    lower: Some(RangeBound {
                        value: PredicateValue::Int(30),
                        inclusive: true,
                    }),
                    upper: None,
                },
            },
        )
        .with_residual("status == 'active'");

        let explanation = QueryExplanation::from_plan(&plan, 10000.0);

        assert_eq!(explanation.strategy, "index_scan");
        assert!(explanation.steps.len() >= 2);
        assert!(explanation.index_used.is_some());
    }
}
