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
use crate::predicate::{
    parse_comparison, split_boolean_expression, ComparisonOperator, PredicateLiteral,
};
use pelago_core::schema::{EntitySchema, IndexType};
use pelago_core::PelagoError;
use std::collections::HashSet;

/// Extracted predicate from CEL expression
#[derive(Debug, Clone)]
pub struct ExtractedPredicate {
    pub property: String,
    pub operator: ComparisonOp,
    pub value: PredicateValue,
    raw_index: usize,
}

pub type ComparisonOp = ComparisonOperator;

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
        let (primary_plan, used_predicate_idx) = Self::select_best_index(schema, &predicates)?;

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
        let groups = match split_boolean_expression(cel_expression) {
            Some(groups) => groups,
            None => return Ok(Vec::new()),
        };

        // Keep planner index extraction constrained to conjunctions. OR expressions
        // continue through the residual/full-scan path unless/until disjunctive index
        // planning is implemented.
        if groups.len() != 1 {
            return Ok(Vec::new());
        }

        let mut predicates = Vec::new();
        for (raw_index, part) in groups[0].iter().enumerate() {
            let Some(parsed) = parse_comparison(part) else {
                continue;
            };

            let value = match parsed.value {
                PredicateLiteral::String(v) => PredicateValue::String(v),
                PredicateLiteral::Int(v) => PredicateValue::Int(v),
                PredicateLiteral::Float(v) => PredicateValue::Float(v),
                PredicateLiteral::Bool(v) => PredicateValue::Bool(v),
            };

            predicates.push(ExtractedPredicate {
                property: parsed.field,
                operator: parsed.operator,
                value,
                raw_index,
            });
        }

        Ok(predicates)
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

        match used_idx {
            Some(idx) => {
                if predicates.len() == 1 {
                    return None;
                }

                // Preserve simple conjunction semantics by dropping the
                // predicate chosen as the primary index scan.
                let conjuncts: Vec<String> = original_expression
                    .split("&&")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .collect();
                if idx >= predicates.len() {
                    return Some(original_expression.to_string());
                }
                let raw_idx = predicates[idx].raw_index;
                if raw_idx < conjuncts.len() {
                    let residual_parts: Vec<String> = conjuncts
                        .into_iter()
                        .enumerate()
                        .filter_map(|(i, part)| (i != raw_idx).then_some(part))
                        .collect();
                    if residual_parts.is_empty() {
                        None
                    } else {
                        Some(residual_parts.join(" && "))
                    }
                } else {
                    // If we cannot safely map back to conjunction parts, keep
                    // the original expression to preserve correctness.
                    Some(original_expression.to_string())
                }
            }
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
    fn test_extract_predicates_skips_non_comparison_terms() {
        let predicates =
            QueryPlanner::extract_predicates("age >= 30 && bio.contains('writer')").unwrap();
        assert_eq!(predicates.len(), 1);
        assert_eq!(predicates[0].property, "age");
        assert_eq!(predicates[0].operator, ComparisonOp::Ge);
    }

    #[test]
    fn test_plan_unique_index() {
        let schema = make_test_schema();
        let plan = QueryPlanner::plan(
            "Person",
            "email == 'alice@example.com'",
            &schema,
            None,
            None,
        )
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
        if let QueryPlan::IndexScan {
            property,
            index_type,
            ..
        } = &plan.primary_plan
        {
            assert_eq!(property, "email");
            assert_eq!(*index_type, IndexType::Unique);
        } else {
            panic!("Expected IndexScan");
        }
    }

    #[test]
    fn test_residual_filter_kept_for_mixed_indexed_and_non_indexed_predicates() {
        let schema = make_test_schema();
        let expression = "age >= 30 && bio == 'writer'";
        let plan = QueryPlanner::plan("Person", expression, &schema, None, None).unwrap();

        assert!(matches!(
            plan.primary_plan,
            QueryPlan::IndexScan { property, .. } if property == "age"
        ));
        assert_eq!(plan.residual_filter.as_deref(), Some("bio == 'writer'"));
    }

    #[test]
    fn test_residual_filter_absent_when_single_indexed_predicate_is_fully_covered() {
        let schema = make_test_schema();
        let plan = QueryPlanner::plan("Person", "status == 'active'", &schema, None, None).unwrap();

        assert!(matches!(
            plan.primary_plan,
            QueryPlan::IndexScan { property, .. } if property == "status"
        ));
        assert!(plan.residual_filter.is_none());
    }

    #[test]
    fn test_residual_filter_kept_for_full_scan() {
        let schema = make_test_schema();
        let expression = "bio == 'writer' && unknown == 5";
        let plan = QueryPlanner::plan("Person", expression, &schema, None, None).unwrap();

        assert!(matches!(plan.primary_plan, QueryPlan::FullScan));
        assert_eq!(plan.residual_filter.as_deref(), Some(expression));
    }

    #[test]
    fn test_residual_filter_removes_only_primary_index_predicate() {
        let schema = make_test_schema();
        let expression = "email == 'a@b.com' && status == 'active' && bio == 'writer'";
        let plan = QueryPlanner::plan("Person", expression, &schema, None, None).unwrap();

        // Unique email should be the primary path; status+bio stay residual.
        assert!(matches!(
            plan.primary_plan,
            QueryPlan::IndexScan { property, .. } if property == "email"
        ));
        assert_eq!(
            plan.residual_filter.as_deref(),
            Some("status == 'active' && bio == 'writer'")
        );
    }
}
