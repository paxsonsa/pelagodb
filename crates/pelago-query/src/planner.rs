//! Query planning
//!
//! The planner takes a CEL expression and schema, extracts indexable predicates,
//! and chooses the most selective index to use as the primary access path.
//!
//! Planning steps:
//! 1. Extract conjuncts from CEL AST
//! 2. Match predicates to available indexes
//! 3. Select most selective index
//! 4. Build execution plan with residual filter

use crate::plan::{ExecutionPlan, IndexPredicate, PredicateValue, QueryPlan, RangeBound};
use pelago_core::schema::{EntitySchema, IndexType};
use pelago_core::PelagoError;
use std::collections::HashSet;

/// Extracted predicate from CEL expression
#[derive(Debug, Clone)]
pub struct ExtractedPredicate {
    pub property: String,
    pub operator: ComparisonOp,
    pub value: PredicateValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl ComparisonOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            ComparisonOp::Eq => "==",
            ComparisonOp::Ne => "!=",
            ComparisonOp::Lt => "<",
            ComparisonOp::Le => "<=",
            ComparisonOp::Gt => ">",
            ComparisonOp::Ge => ">=",
        }
    }
}

/// Query planner
pub struct QueryPlanner;

impl QueryPlanner {
    /// Plan a query from a CEL expression
    pub fn plan(
        entity_type: &str,
        cel_expression: &str,
        schema: &EntitySchema,
        projection: Option<HashSet<String>>,
        limit: Option<u32>,
    ) -> Result<ExecutionPlan, PelagoError> {
        // Parse simple predicates from the expression
        // For now, we use a simple regex-based approach
        // Full CEL integration would use the cel_interpreter crate
        let predicates = Self::extract_predicates(cel_expression)?;

        // Find the best index to use
        let (primary_plan, used_predicate_idx) =
            Self::select_best_index(schema, &predicates)?;

        // Build residual filter from remaining predicates
        let residual = Self::build_residual(&predicates, used_predicate_idx, cel_expression);

        let mut plan = ExecutionPlan::new(entity_type, primary_plan);

        if let Some(filter) = residual {
            plan = plan.with_residual(filter);
        }

        if let Some(fields) = projection {
            plan = plan.with_projection(fields);
        }

        if let Some(l) = limit {
            plan = plan.with_limit(l);
        }

        Ok(plan)
    }

    /// Extract simple predicates from a CEL expression
    /// This is a simplified parser - full CEL support requires cel_interpreter
    fn extract_predicates(cel_expression: &str) -> Result<Vec<ExtractedPredicate>, PelagoError> {
        let mut predicates = Vec::new();

        // Split by && to get conjuncts
        for part in cel_expression.split("&&") {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            if let Some(pred) = Self::parse_comparison(part) {
                predicates.push(pred);
            }
        }

        Ok(predicates)
    }

    /// Parse a single comparison expression
    fn parse_comparison(expr: &str) -> Option<ExtractedPredicate> {
        // Try different operators
        for (op_str, op) in [
            ("==", ComparisonOp::Eq),
            ("!=", ComparisonOp::Ne),
            (">=", ComparisonOp::Ge),
            ("<=", ComparisonOp::Le),
            (">", ComparisonOp::Gt),
            ("<", ComparisonOp::Lt),
        ] {
            if let Some(pos) = expr.find(op_str) {
                let property = expr[..pos].trim().to_string();
                let value_str = expr[pos + op_str.len()..].trim();

                // Skip if property has spaces (probably a complex expression)
                if property.contains(' ') {
                    continue;
                }

                if let Some(value) = Self::parse_value(value_str) {
                    return Some(ExtractedPredicate {
                        property,
                        operator: op,
                        value,
                    });
                }
            }
        }
        None
    }

    /// Parse a value literal
    fn parse_value(s: &str) -> Option<PredicateValue> {
        let s = s.trim();

        // String literal
        if (s.starts_with('"') && s.ends_with('"'))
            || (s.starts_with('\'') && s.ends_with('\''))
        {
            return Some(PredicateValue::String(s[1..s.len() - 1].to_string()));
        }

        // Boolean
        if s == "true" {
            return Some(PredicateValue::Bool(true));
        }
        if s == "false" {
            return Some(PredicateValue::Bool(false));
        }

        // Float (contains decimal point)
        if s.contains('.') {
            if let Ok(f) = s.parse::<f64>() {
                return Some(PredicateValue::Float(f));
            }
        }

        // Integer
        if let Ok(i) = s.parse::<i64>() {
            return Some(PredicateValue::Int(i));
        }

        None
    }

    /// Select the best index based on predicate selectivity
    fn select_best_index(
        schema: &EntitySchema,
        predicates: &[ExtractedPredicate],
    ) -> Result<(QueryPlan, Option<usize>), PelagoError> {
        let mut best_plan: Option<(QueryPlan, usize, f64)> = None;

        for (idx, pred) in predicates.iter().enumerate() {
            // Check if this property has an index
            if let Some(prop_def) = schema.properties.get(&pred.property) {
                if prop_def.index == IndexType::None {
                    continue;
                }

                let plan = match pred.operator {
                    ComparisonOp::Eq => {
                        // Equality - can use any index type
                        QueryPlan::IndexScan {
                            property: pred.property.clone(),
                            index_type: prop_def.index,
                            predicate: IndexPredicate::Equals(pred.value.clone()),
                        }
                    }
                    ComparisonOp::Gt | ComparisonOp::Ge | ComparisonOp::Lt | ComparisonOp::Le => {
                        // Range - only use range index
                        if prop_def.index != IndexType::Range {
                            continue;
                        }

                        let (lower, upper) = match pred.operator {
                            ComparisonOp::Gt => (
                                Some(RangeBound {
                                    value: pred.value.clone(),
                                    inclusive: false,
                                }),
                                None,
                            ),
                            ComparisonOp::Ge => (
                                Some(RangeBound {
                                    value: pred.value.clone(),
                                    inclusive: true,
                                }),
                                None,
                            ),
                            ComparisonOp::Lt => (
                                None,
                                Some(RangeBound {
                                    value: pred.value.clone(),
                                    inclusive: false,
                                }),
                            ),
                            ComparisonOp::Le => (
                                None,
                                Some(RangeBound {
                                    value: pred.value.clone(),
                                    inclusive: true,
                                }),
                            ),
                            _ => continue,
                        };

                        QueryPlan::IndexScan {
                            property: pred.property.clone(),
                            index_type: IndexType::Range,
                            predicate: IndexPredicate::Range { lower, upper },
                        }
                    }
                    ComparisonOp::Ne => {
                        // != cannot use index effectively
                        continue;
                    }
                };

                let selectivity = plan.selectivity();

                // Keep the most selective plan
                if best_plan.is_none() || selectivity < best_plan.as_ref().unwrap().2 {
                    best_plan = Some((plan, idx, selectivity));
                }
            }
        }

        match best_plan {
            Some((plan, idx, _)) => Ok((plan, Some(idx))),
            None => Ok((QueryPlan::FullScan, None)),
        }
    }

    /// Build residual filter from predicates not used by the index
    fn build_residual(
        predicates: &[ExtractedPredicate],
        used_idx: Option<usize>,
        original_expression: &str,
    ) -> Option<String> {
        if predicates.is_empty() {
            return None;
        }

        // If we used an index, the remaining predicates become residual
        // For simplicity, if we used one predicate, return the original expression
        // A proper implementation would reconstruct the expression without the used predicate
        match used_idx {
            Some(_idx) if predicates.len() > 1 => {
                // Multiple predicates - return original (simplified approach)
                Some(original_expression.to_string())
            }
            Some(_) => None, // Single predicate fully covered by index
            None => Some(original_expression.to_string()), // Full scan needs all predicates
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_core::schema::PropertyDef;
    use pelago_core::PropertyType;

    fn make_test_schema() -> EntitySchema {
        EntitySchema::new("Person")
            .with_property(
                "email",
                PropertyDef::new(PropertyType::String).with_index(IndexType::Unique),
            )
            .with_property(
                "age",
                PropertyDef::new(PropertyType::Int).with_index(IndexType::Range),
            )
            .with_property(
                "status",
                PropertyDef::new(PropertyType::String).with_index(IndexType::Equality),
            )
            .with_property(
                "bio",
                PropertyDef::new(PropertyType::String), // No index
            )
    }

    #[test]
    fn test_extract_predicates() {
        let predicates =
            QueryPlanner::extract_predicates("age >= 30 && status == 'active'").unwrap();

        assert_eq!(predicates.len(), 2);
        assert_eq!(predicates[0].property, "age");
        assert_eq!(predicates[0].operator, ComparisonOp::Ge);
        assert_eq!(predicates[1].property, "status");
        assert_eq!(predicates[1].operator, ComparisonOp::Eq);
    }

    #[test]
    fn test_parse_value_types() {
        assert_eq!(
            QueryPlanner::parse_value("\"hello\""),
            Some(PredicateValue::String("hello".into()))
        );
        assert_eq!(
            QueryPlanner::parse_value("'world'"),
            Some(PredicateValue::String("world".into()))
        );
        assert_eq!(
            QueryPlanner::parse_value("42"),
            Some(PredicateValue::Int(42))
        );
        assert_eq!(
            QueryPlanner::parse_value("3.14"),
            Some(PredicateValue::Float(3.14))
        );
        assert_eq!(
            QueryPlanner::parse_value("true"),
            Some(PredicateValue::Bool(true))
        );
    }

    #[test]
    fn test_plan_unique_index() {
        let schema = make_test_schema();
        let plan =
            QueryPlanner::plan("Person", "email == 'alice@example.com'", &schema, None, None)
                .unwrap();

        assert!(matches!(
            plan.primary_plan,
            QueryPlan::IndexScan {
                index_type: IndexType::Unique,
                ..
            }
        ));
    }

    #[test]
    fn test_plan_range_index() {
        let schema = make_test_schema();
        let plan = QueryPlanner::plan("Person", "age >= 30", &schema, None, None).unwrap();

        assert!(matches!(
            plan.primary_plan,
            QueryPlan::IndexScan {
                index_type: IndexType::Range,
                ..
            }
        ));
    }

    #[test]
    fn test_plan_no_index() {
        let schema = make_test_schema();
        let plan =
            QueryPlanner::plan("Person", "bio.contains('developer')", &schema, None, None).unwrap();

        // bio has no index, so full scan
        assert!(matches!(plan.primary_plan, QueryPlan::FullScan));
    }

    #[test]
    fn test_plan_selects_most_selective() {
        let schema = make_test_schema();
        // Both email and status have indexes, but unique is more selective
        let plan = QueryPlanner::plan(
            "Person",
            "email == 'test@example.com' && status == 'active'",
            &schema,
            None,
            None,
        )
        .unwrap();

        // Should use the unique index on email
        if let QueryPlan::IndexScan { property, index_type, .. } = &plan.primary_plan {
            assert_eq!(property, "email");
            assert_eq!(*index_type, IndexType::Unique);
        } else {
            panic!("Expected IndexScan");
        }
    }
}
