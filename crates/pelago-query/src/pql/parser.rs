use pest::Parser;
use pest_derive::Parser;

use super::ast::*;

#[derive(Parser)]
#[grammar = "pql/grammar.pest"]
struct PqlParser;

#[derive(Debug, thiserror::Error)]
pub enum PqlParseError {
    #[error("Parse error at line {line}, column {col}: {message}")]
    Syntax {
        line: usize,
        col: usize,
        message: String,
    },
    #[error("Invalid root function: {0}")]
    InvalidRootFunction(String),
    #[error("Invalid directive: {0}")]
    InvalidDirective(String),
    #[error("Empty query")]
    EmptyQuery,
}

impl From<pest::error::Error<Rule>> for PqlParseError {
    fn from(e: pest::error::Error<Rule>) -> Self {
        let (line, col) = match e.line_col {
            pest::error::LineColLocation::Pos((l, c)) => (l, c),
            pest::error::LineColLocation::Span((l, c), _) => (l, c),
        };
        PqlParseError::Syntax {
            line,
            col,
            message: e.variant.message().to_string(),
        }
    }
}

pub fn parse_pql(input: &str) -> Result<PqlQuery, PqlParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(PqlParseError::EmptyQuery);
    }

    let pairs = PqlParser::parse(Rule::query, trimmed)?;

    let query_pair = pairs.into_iter().next().unwrap();
    let inner = query_pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::short_form => parse_short_form(inner),
        Rule::full_query => parse_full_query(inner),
        _ => Err(PqlParseError::Syntax {
            line: 1,
            col: 1,
            message: "Expected short-form or full query".to_string(),
        }),
    }
}

fn parse_short_form(pair: pest::iterators::Pair<Rule>) -> Result<PqlQuery, PqlParseError> {
    let mut inner = pair.into_inner();

    let qt = parse_qualified_type(inner.next().unwrap())?;

    let mut directives = Vec::new();
    let mut selections = Vec::new();

    for p in inner {
        match p.as_rule() {
            Rule::directive => directives.push(parse_directive(p)?),
            Rule::selection_block => selections = parse_selection_block(p)?,
            _ => {}
        }
    }

    Ok(PqlQuery {
        name: None,
        default_namespace: None,
        blocks: vec![QueryBlock {
            name: "result".to_string(),
            root: RootFunction::Type(qt),
            directives,
            selections,
            capture_as: None,
        }],
    })
}

fn parse_full_query(pair: pest::iterators::Pair<Rule>) -> Result<PqlQuery, PqlParseError> {
    let inner = pair.into_inner();
    let mut name = None;
    let mut blocks = Vec::new();

    for p in inner {
        match p.as_rule() {
            Rule::ident => name = Some(p.as_str().to_string()),
            Rule::query_block => blocks.push(parse_query_block(p)?),
            _ => {}
        }
    }

    if blocks.is_empty() {
        return Err(PqlParseError::EmptyQuery);
    }

    Ok(PqlQuery {
        name,
        default_namespace: None,
        blocks,
    })
}

fn parse_query_block(pair: pest::iterators::Pair<Rule>) -> Result<QueryBlock, PqlParseError> {
    let inner = pair.into_inner();

    // Check for "varname as blockname" pattern
    // We collect all idents before the root_function
    let mut idents: Vec<String> = Vec::new();
    let mut root = None;
    let mut directives = Vec::new();
    let mut selections = Vec::new();

    for p in inner {
        match p.as_rule() {
            Rule::ident => idents.push(p.as_str().to_string()),
            Rule::root_function => root = Some(parse_root_function(p)?),
            Rule::directive => directives.push(parse_directive(p)?),
            Rule::selection_block => selections = parse_selection_block(p)?,
            _ => {}
        }
    }

    let root = root.ok_or_else(|| PqlParseError::Syntax {
        line: 0,
        col: 0,
        message: "Missing root function".to_string(),
    })?;

    // If we have 2 idents, the first is capture_as and second is block name
    // If we have 1 ident, it's just the block name
    let (capture_as, name) = match idents.len() {
        2 => (Some(idents[0].clone()), idents[1].clone()),
        1 => (None, idents[0].clone()),
        _ => {
            return Err(PqlParseError::Syntax {
                line: 0,
                col: 0,
                message: "Expected block name".to_string(),
            })
        }
    };

    Ok(QueryBlock {
        name,
        root,
        directives,
        selections,
        capture_as,
    })
}

fn parse_root_function(pair: pest::iterators::Pair<Rule>) -> Result<RootFunction, PqlParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::uid_func => {
            let qref = parse_qualified_ref(inner.into_inner().next().unwrap())?;
            Ok(RootFunction::Uid(qref))
        }
        Rule::uid_var_func => {
            let var = inner.into_inner().next().unwrap().as_str().to_string();
            Ok(RootFunction::UidVar(var))
        }
        Rule::uid_set_func => {
            let vars: Vec<String> = inner
                .into_inner()
                .filter(|p| p.as_rule() == Rule::ident)
                .map(|p| p.as_str().to_string())
                .collect();
            Ok(RootFunction::UidSet(vars, SetOp::Union))
        }
        Rule::type_func => {
            let qt = parse_qualified_type(inner.into_inner().next().unwrap())?;
            Ok(RootFunction::Type(qt))
        }
        Rule::eq_func => {
            let mut it = inner.into_inner();
            let field = it.next().unwrap().as_str().to_string();
            let val = parse_literal(it.next().unwrap())?;
            Ok(RootFunction::Eq(field, val))
        }
        Rule::ge_func => {
            let mut it = inner.into_inner();
            let field = it.next().unwrap().as_str().to_string();
            let val = parse_literal(it.next().unwrap())?;
            Ok(RootFunction::Ge(field, val))
        }
        Rule::le_func => {
            let mut it = inner.into_inner();
            let field = it.next().unwrap().as_str().to_string();
            let val = parse_literal(it.next().unwrap())?;
            Ok(RootFunction::Le(field, val))
        }
        Rule::gt_func => {
            let mut it = inner.into_inner();
            let field = it.next().unwrap().as_str().to_string();
            let val = parse_literal(it.next().unwrap())?;
            Ok(RootFunction::Gt(field, val))
        }
        Rule::lt_func => {
            let mut it = inner.into_inner();
            let field = it.next().unwrap().as_str().to_string();
            let val = parse_literal(it.next().unwrap())?;
            Ok(RootFunction::Lt(field, val))
        }
        Rule::between_func => {
            let mut it = inner.into_inner();
            let field = it.next().unwrap().as_str().to_string();
            let val1 = parse_literal(it.next().unwrap())?;
            let val2 = parse_literal(it.next().unwrap())?;
            Ok(RootFunction::Between(field, val1, val2))
        }
        Rule::has_func => {
            let field = inner.into_inner().next().unwrap().as_str().to_string();
            Ok(RootFunction::Has(field))
        }
        Rule::allofterms_func => {
            let mut it = inner.into_inner();
            let field = it.next().unwrap().as_str().to_string();
            let raw = it.next().unwrap().as_str();
            // Strip quotes from string literal
            let terms = raw[1..raw.len() - 1].to_string();
            Ok(RootFunction::AllOfTerms(field, terms))
        }
        _ => Err(PqlParseError::InvalidRootFunction(
            inner.as_str().to_string(),
        )),
    }
}

fn parse_qualified_ref(pair: pest::iterators::Pair<Rule>) -> Result<QualifiedRef, PqlParseError> {
    let raw = pair.as_str();
    let parts: Vec<&str> = raw.split(':').collect();
    match parts.as_slice() {
        [entity_type, node_id] => Ok(QualifiedRef {
            namespace: None,
            entity_type: (*entity_type).to_string(),
            node_id: (*node_id).to_string(),
        }),
        [namespace, entity_type, node_id] => Ok(QualifiedRef {
            namespace: Some((*namespace).to_string()),
            entity_type: (*entity_type).to_string(),
            node_id: (*node_id).to_string(),
        }),
        _ => Err(PqlParseError::Syntax {
            line: 0,
            col: 0,
            message: format!("Invalid qualified reference: {}", raw),
        }),
    }
}

fn parse_qualified_type(pair: pest::iterators::Pair<Rule>) -> Result<QualifiedType, PqlParseError> {
    let raw = pair.as_str();
    let parts: Vec<&str> = raw.split(':').collect();
    match parts.as_slice() {
        [entity_type] => Ok(QualifiedType {
            namespace: None,
            entity_type: (*entity_type).to_string(),
        }),
        [namespace, entity_type] => Ok(QualifiedType {
            namespace: Some((*namespace).to_string()),
            entity_type: (*entity_type).to_string(),
        }),
        _ => Err(PqlParseError::Syntax {
            line: 0,
            col: 0,
            message: format!("Invalid qualified type: {}", raw),
        }),
    }
}

fn parse_selection_block(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Vec<Selection>, PqlParseError> {
    let mut selections = Vec::new();
    for p in pair.into_inner() {
        if p.as_rule() == Rule::selection {
            selections.push(parse_selection(p)?);
        }
    }
    Ok(selections)
}

fn parse_selection(pair: pest::iterators::Pair<Rule>) -> Result<Selection, PqlParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::field_name => Ok(Selection::Field(inner.as_str().to_string())),
        Rule::edge_traversal => Ok(Selection::Edge(parse_edge_traversal(inner)?)),
        Rule::aggregate_expr => Ok(Selection::Aggregate(parse_aggregate_expr(inner)?)),
        Rule::value_var => {
            let mut it = inner.into_inner();
            let var_name = it.next().unwrap().as_str().to_string();
            let agg = parse_aggregate_call(it.next().unwrap())?;
            Ok(Selection::ValueVar(var_name, agg))
        }
        _ => Err(PqlParseError::Syntax {
            line: 0,
            col: 0,
            message: format!("Unexpected selection: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_edge_traversal(pair: pest::iterators::Pair<Rule>) -> Result<EdgeTraversal, PqlParseError> {
    let mut capture_as = None;
    let mut edge_namespace = None;
    let mut edge_type = String::new();
    let mut direction = PqlEdgeDirection::Outgoing;
    let mut target_type = None;
    let mut directives = Vec::new();
    let mut selections = Vec::new();

    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::ident => {
                // This is the capture var name (before "as")
                capture_as = Some(p.as_str().to_string());
            }
            Rule::edge_spec => {
                for ep in p.into_inner() {
                    match ep.as_rule() {
                        Rule::edge_label => {
                            let label_parts: Vec<_> = ep.into_inner().collect();
                            if label_parts.len() == 2 {
                                // ns:TYPE
                                edge_namespace = Some(label_parts[0].as_str().to_string());
                                edge_type = label_parts[1].as_str().to_string();
                            } else {
                                edge_type = label_parts[0].as_str().to_string();
                            }
                        }
                        Rule::edge_direction => {
                            let dir = ep.into_inner().next().unwrap();
                            direction = match dir.as_rule() {
                                Rule::outgoing => PqlEdgeDirection::Outgoing,
                                Rule::incoming => PqlEdgeDirection::Incoming,
                                Rule::bidirectional => PqlEdgeDirection::Both,
                                _ => PqlEdgeDirection::Outgoing,
                            };
                        }
                        _ => {}
                    }
                }
            }
            Rule::target_spec => {
                let qt = parse_qualified_type(p.into_inner().next().unwrap())?;
                target_type = Some(TargetSpec {
                    namespace: qt.namespace,
                    entity_type: qt.entity_type,
                });
            }
            Rule::directive => directives.push(parse_directive(p)?),
            Rule::selection_block => selections = parse_selection_block(p)?,
            _ => {}
        }
    }

    Ok(EdgeTraversal {
        edge_namespace,
        edge_type,
        direction,
        target_type,
        directives,
        selections,
        capture_as,
    })
}

fn parse_directive(pair: pest::iterators::Pair<Rule>) -> Result<Directive, PqlParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::filter_directive => {
            let expr = inner.into_inner().next().unwrap().as_str().to_string();
            Ok(Directive::Filter(expr))
        }
        Rule::edge_directive => {
            let expr = inner.into_inner().next().unwrap().as_str().to_string();
            Ok(Directive::Edge(expr))
        }
        Rule::cascade_directive => Ok(Directive::Cascade),
        Rule::limit_directive => {
            let args = inner.into_inner().next().unwrap();
            let mut first = 0u32;
            let mut offset = None;
            for p in args.into_inner() {
                match p.as_rule() {
                    Rule::first_arg => {
                        let num = p.into_inner().next().unwrap().as_str();
                        first = num.parse().unwrap_or(0);
                    }
                    Rule::offset_arg => {
                        let num = p.into_inner().next().unwrap().as_str();
                        offset = Some(num.parse().unwrap_or(0));
                    }
                    _ => {}
                }
            }
            Ok(Directive::Limit { first, offset })
        }
        Rule::sort_directive => {
            let args = inner.into_inner().next().unwrap();
            let mut field = String::new();
            let mut desc = false;
            let mut on_edge = false;
            for p in args.into_inner() {
                match p.as_rule() {
                    Rule::ident => field = p.as_str().to_string(),
                    Rule::sort_order => desc = p.as_str() == "desc",
                    Rule::bool_literal => on_edge = p.as_str() == "true",
                    _ => {}
                }
            }
            Ok(Directive::Sort {
                field,
                desc,
                on_edge,
            })
        }
        Rule::facets_directive => {
            let fields: Vec<String> = inner
                .into_inner()
                .next()
                .unwrap()
                .into_inner()
                .filter(|p| p.as_rule() == Rule::ident)
                .map(|p| p.as_str().to_string())
                .collect();
            Ok(Directive::Facets(fields))
        }
        Rule::recurse_directive => {
            let depth: u32 = inner
                .into_inner()
                .next()
                .unwrap()
                .as_str()
                .parse()
                .unwrap_or(1);
            Ok(Directive::Recurse { depth })
        }
        Rule::groupby_directive => {
            let fields: Vec<String> = inner
                .into_inner()
                .next()
                .unwrap()
                .into_inner()
                .filter(|p| p.as_rule() == Rule::ident)
                .map(|p| p.as_str().to_string())
                .collect();
            Ok(Directive::GroupBy(fields))
        }
        Rule::explain_directive => Ok(Directive::Explain),
        _ => Err(PqlParseError::InvalidDirective(inner.as_str().to_string())),
    }
}

fn parse_aggregate_expr(pair: pest::iterators::Pair<Rule>) -> Result<AggregateExpr, PqlParseError> {
    let call = pair.into_inner().next().unwrap();
    parse_aggregate_call(call)
}

fn parse_aggregate_call(pair: pest::iterators::Pair<Rule>) -> Result<AggregateExpr, PqlParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::count_edge => {
            let edge_spec = inner.into_inner().next().unwrap();
            let mut edge_ns = None;
            let mut edge_type = String::new();
            let mut direction = PqlEdgeDirection::Outgoing;

            for ep in edge_spec.into_inner() {
                match ep.as_rule() {
                    Rule::edge_label => {
                        let parts: Vec<_> = ep.into_inner().collect();
                        if parts.len() == 2 {
                            edge_ns = Some(parts[0].as_str().to_string());
                            edge_type = parts[1].as_str().to_string();
                        } else {
                            edge_type = parts[0].as_str().to_string();
                        }
                    }
                    Rule::edge_direction => {
                        let dir = ep.into_inner().next().unwrap();
                        direction = match dir.as_rule() {
                            Rule::outgoing => PqlEdgeDirection::Outgoing,
                            Rule::incoming => PqlEdgeDirection::Incoming,
                            Rule::bidirectional => PqlEdgeDirection::Both,
                            _ => PqlEdgeDirection::Outgoing,
                        };
                    }
                    _ => {}
                }
            }

            Ok(AggregateExpr::Count(Box::new(Selection::Edge(
                EdgeTraversal {
                    edge_namespace: edge_ns,
                    edge_type,
                    direction,
                    target_type: None,
                    directives: vec![],
                    selections: vec![],
                    capture_as: None,
                },
            ))))
        }
        Rule::count_func => {
            let field = inner.into_inner().next().unwrap().as_str().to_string();
            Ok(AggregateExpr::Count(Box::new(Selection::Field(field))))
        }
        Rule::sum_func => {
            let field = inner.into_inner().next().unwrap().as_str().to_string();
            Ok(AggregateExpr::Sum(field))
        }
        Rule::avg_func => {
            let field = inner.into_inner().next().unwrap().as_str().to_string();
            Ok(AggregateExpr::Avg(field))
        }
        Rule::min_func => {
            let field = inner.into_inner().next().unwrap().as_str().to_string();
            Ok(AggregateExpr::Min(field))
        }
        Rule::max_func => {
            let field = inner.into_inner().next().unwrap().as_str().to_string();
            Ok(AggregateExpr::Max(field))
        }
        _ => Err(PqlParseError::Syntax {
            line: 0,
            col: 0,
            message: format!("Unexpected aggregate: {:?}", inner.as_rule()),
        }),
    }
}

fn parse_literal(pair: pest::iterators::Pair<Rule>) -> Result<LiteralValue, PqlParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::string_literal => {
            let raw = inner.as_str();
            Ok(LiteralValue::String(raw[1..raw.len() - 1].to_string()))
        }
        Rule::float_literal => {
            let val: f64 = inner.as_str().parse().map_err(|_| PqlParseError::Syntax {
                line: 0,
                col: 0,
                message: "Invalid float".to_string(),
            })?;
            Ok(LiteralValue::Float(val))
        }
        Rule::integer_literal => {
            let val: i64 = inner.as_str().parse().map_err(|_| PqlParseError::Syntax {
                line: 0,
                col: 0,
                message: "Invalid integer".to_string(),
            })?;
            Ok(LiteralValue::Int(val))
        }
        Rule::bool_literal => Ok(LiteralValue::Bool(inner.as_str() == "true")),
        _ => Err(PqlParseError::Syntax {
            line: 0,
            col: 0,
            message: format!("Unexpected literal: {:?}", inner.as_rule()),
        }),
    }
}
