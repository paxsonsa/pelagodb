use super::{PointEdgeTarget, PointNodeTarget, PqlPredicate, QueryFilter, WatchFilter};
use pelago_core::Value;
use pelago_proto::EdgeDirection;
use pelago_query::cel::CelEnvironment;
use pelago_query::pql::{parse_pql, Directive, LiteralValue, RootFunction};
use pelago_storage::SchemaRegistry;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::Status;

pub(super) fn matches_node_scope(filter: &WatchFilter, entity_type: &str, node_id: &str) -> bool {
    match filter {
        WatchFilter::Namespace => true,
        WatchFilter::Point { nodes, .. } => nodes
            .iter()
            .any(|target| target.entity_type == entity_type && target.node_id == node_id),
        WatchFilter::Query(q) => query_scope_matches(q, entity_type),
    }
}

pub(super) fn matches_node_update_scope(
    filter: &WatchFilter,
    entity_type: &str,
    node_id: &str,
    changed_properties: &HashMap<String, Value>,
) -> bool {
    match filter {
        WatchFilter::Point { nodes, .. } => {
            let targets: Vec<&PointNodeTarget> = nodes
                .iter()
                .filter(|target| target.entity_type == entity_type && target.node_id == node_id)
                .collect();
            if targets.is_empty() {
                return false;
            }
            targets.iter().any(|target| {
                target.properties.is_empty()
                    || changed_properties
                        .keys()
                        .any(|prop| target.properties.contains(prop))
            })
        }
        _ => matches_node_scope(filter, entity_type, node_id),
    }
}

pub(super) fn matches_edge_scope(
    filter: &WatchFilter,
    source_type: &str,
    source_id: &str,
    target_type: &str,
    target_id: &str,
    label: &str,
) -> bool {
    match filter {
        WatchFilter::Namespace => true,
        WatchFilter::Point { nodes, edges } => {
            let node_touch = nodes.iter().any(|target| {
                (target.entity_type == source_type && target.node_id == source_id)
                    || (target.entity_type == target_type && target.node_id == target_id)
            });
            let edge_match = edges.iter().any(|target| {
                point_edge_target_matches(
                    target,
                    source_type,
                    source_id,
                    target_type,
                    target_id,
                    label,
                )
            });
            node_touch || edge_match
        }
        WatchFilter::Query(q) => {
            query_scope_matches(q, source_type) || query_scope_matches(q, target_type)
        }
    }
}

pub(super) fn point_edge_target_matches(
    target: &PointEdgeTarget,
    source_type: &str,
    source_id: &str,
    target_type: &str,
    target_id: &str,
    label: &str,
) -> bool {
    if !target.label.is_empty() && target.label != label {
        return false;
    }

    let dir = if target.direction == EdgeDirection::Incoming as i32 {
        EdgeDirection::Incoming
    } else if target.direction == EdgeDirection::Both as i32 {
        EdgeDirection::Both
    } else {
        EdgeDirection::Outgoing
    };

    let outbound_match = target.source_entity_type == source_type
        && target.source_node_id == source_id
        && target
            .target
            .as_ref()
            .map(|(et, id)| et == target_type && id == target_id)
            .unwrap_or(true);

    let inbound_match = target.source_entity_type == target_type
        && target.source_node_id == target_id
        && target
            .target
            .as_ref()
            .map(|(et, id)| et == source_type && id == source_id)
            .unwrap_or(true);

    match dir {
        EdgeDirection::Incoming => inbound_match,
        EdgeDirection::Both => outbound_match || inbound_match,
        _ => outbound_match,
    }
}

pub(super) fn query_scope_matches(filter: &QueryFilter, entity_type: &str) -> bool {
    if !filter.entity_type.is_empty() && filter.entity_type != entity_type {
        return false;
    }
    if let Some(pql) = &filter.pql_predicate {
        if let Some(root_et) = &pql.root_entity_type {
            if root_et != entity_type {
                return false;
            }
        }
    }
    true
}

pub(super) async fn query_matches_node(
    filter: &QueryFilter,
    database: &str,
    namespace: &str,
    entity_type: &str,
    properties: &HashMap<String, Value>,
    schema_registry: &Arc<SchemaRegistry>,
) -> bool {
    if !query_scope_matches(filter, entity_type) {
        return false;
    }

    if let Some(cel) = &filter.cel_expression {
        if !evaluate_cel(
            schema_registry,
            database,
            namespace,
            entity_type,
            cel,
            properties,
        )
        .await
        {
            return false;
        }
    }

    if let Some(pql) = &filter.pql_predicate {
        if let Some(cel) = &pql.cel_expression {
            if !evaluate_cel(
                schema_registry,
                database,
                namespace,
                entity_type,
                cel,
                properties,
            )
            .await
            {
                return false;
            }
        }
    }

    true
}

async fn evaluate_cel(
    schema_registry: &Arc<SchemaRegistry>,
    database: &str,
    namespace: &str,
    entity_type: &str,
    cel_expression: &str,
    properties: &HashMap<String, Value>,
) -> bool {
    if cel_expression.trim().is_empty() {
        return true;
    }

    let schema = match schema_registry
        .get_schema(database, namespace, entity_type)
        .await
    {
        Ok(Some(schema)) => schema,
        _ => return false,
    };

    let env = CelEnvironment::new(schema);
    let compiled = match env.compile(cel_expression) {
        Ok(compiled) => compiled,
        Err(_) => return false,
    };
    compiled.evaluate(properties).unwrap_or(false)
}

pub(super) fn build_query_filter(
    entity_type: &str,
    cel_expression: &str,
    pql_query: &str,
) -> Result<QueryFilter, Status> {
    let cel_expression = if cel_expression.trim().is_empty() {
        None
    } else {
        Some(cel_expression.trim().to_string())
    };

    let pql_predicate = if pql_query.trim().is_empty() {
        None
    } else {
        Some(build_pql_predicate(pql_query.trim())?)
    };

    Ok(QueryFilter {
        entity_type: entity_type.to_string(),
        cel_expression,
        pql_predicate,
    })
}

pub(super) fn build_pql_predicate(query: &str) -> Result<PqlPredicate, Status> {
    let ast = parse_pql(query).map_err(|e| Status::invalid_argument(e.to_string()))?;
    let block = ast.blocks.first().ok_or_else(|| {
        Status::invalid_argument("PQL watch query must include at least one block")
    })?;

    let root_entity_type = root_entity_type(&block.root);
    let mut filters = Vec::new();
    if let Some(root_expr) = root_to_cel_expression(&block.root) {
        filters.push(root_expr);
    }
    for directive in &block.directives {
        if let Directive::Filter(expr) = directive {
            if !expr.trim().is_empty() {
                filters.push(expr.trim().to_string());
            }
        }
    }

    let cel_expression = if filters.is_empty() {
        None
    } else {
        Some(filters.join(" && "))
    };

    Ok(PqlPredicate {
        root_entity_type,
        cel_expression,
    })
}

fn root_entity_type(root: &RootFunction) -> Option<String> {
    match root {
        RootFunction::Type(qt) => Some(qt.entity_type.clone()),
        RootFunction::Uid(qr) => Some(qr.entity_type.clone()),
        _ => None,
    }
}

pub(super) fn root_to_cel_expression(root: &RootFunction) -> Option<String> {
    match root {
        RootFunction::Eq(field, value) => Some(format!("{} == {}", field, literal_to_cel(value))),
        RootFunction::Ge(field, value) => Some(format!("{} >= {}", field, literal_to_cel(value))),
        RootFunction::Le(field, value) => Some(format!("{} <= {}", field, literal_to_cel(value))),
        RootFunction::Gt(field, value) => Some(format!("{} > {}", field, literal_to_cel(value))),
        RootFunction::Lt(field, value) => Some(format!("{} < {}", field, literal_to_cel(value))),
        RootFunction::Between(field, lo, hi) => Some(format!(
            "({} >= {}) && ({} <= {})",
            field,
            literal_to_cel(lo),
            field,
            literal_to_cel(hi)
        )),
        RootFunction::Has(field) => Some(format!("{} != null", field)),
        _ => None,
    }
}

pub(super) fn literal_to_cel(value: &LiteralValue) -> String {
    match value {
        LiteralValue::String(s) => format!("'{}'", s.replace('\'', "\\'")),
        LiteralValue::Int(i) => i.to_string(),
        LiteralValue::Float(f) => {
            if f.fract() == 0.0 {
                format!("{:.1}", f)
            } else {
                f.to_string()
            }
        }
        LiteralValue::Bool(b) => b.to_string(),
    }
}
