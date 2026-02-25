//! Shared parsing helpers for simple comparison predicates.
//!
//! These helpers intentionally handle only a subset of CEL:
//! - `field OP literal` predicates
//! - conjunctions with `&&`
//! - disjunctions with `||`
//! - no explicit parenthesized boolean grouping support

#[derive(Debug, Clone, PartialEq)]
pub struct ComparisonPredicate {
    pub field: String,
    pub operator: ComparisonOperator,
    pub value: PredicateLiteral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOperator {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl ComparisonOperator {
    pub fn as_str(&self) -> &'static str {
        match self {
            ComparisonOperator::Eq => "==",
            ComparisonOperator::Ne => "!=",
            ComparisonOperator::Lt => "<",
            ComparisonOperator::Le => "<=",
            ComparisonOperator::Gt => ">",
            ComparisonOperator::Ge => ">=",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PredicateLiteral {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// Split a simple boolean expression into OR-groups of AND-terms.
///
/// Returns `None` for unsupported expression forms (empty terms).
pub fn split_boolean_expression(expression: &str) -> Option<Vec<Vec<String>>> {
    let expression = expression.trim();
    if expression.is_empty() {
        return None;
    }

    let mut groups = Vec::new();
    for or_part in expression.split("||") {
        let or_part = or_part.trim();
        if or_part.is_empty() {
            return None;
        }

        let mut group = Vec::new();
        for and_part in or_part.split("&&") {
            let and_part = and_part.trim();
            if and_part.is_empty() {
                return None;
            }
            group.push(and_part.to_string());
        }

        groups.push(group);
    }

    if groups.is_empty() {
        None
    } else {
        Some(groups)
    }
}

/// Parse a single `field OP literal` predicate.
pub fn parse_comparison(expr: &str) -> Option<ComparisonPredicate> {
    let expr = expr.trim();
    for (op_str, op) in [
        ("==", ComparisonOperator::Eq),
        ("!=", ComparisonOperator::Ne),
        (">=", ComparisonOperator::Ge),
        ("<=", ComparisonOperator::Le),
        (">", ComparisonOperator::Gt),
        ("<", ComparisonOperator::Lt),
    ] {
        if let Some(pos) = expr.find(op_str) {
            let field = expr[..pos].trim();
            let value_str = expr[pos + op_str.len()..].trim();

            if field.is_empty() || field.contains(' ') {
                return None;
            }

            let value = parse_literal(value_str)?;
            return Some(ComparisonPredicate {
                field: field.to_string(),
                operator: op,
                value,
            });
        }
    }
    None
}

fn parse_literal(raw: &str) -> Option<PredicateLiteral> {
    let raw = raw.trim();

    if (raw.starts_with('"') && raw.ends_with('"'))
        || (raw.starts_with('\'') && raw.ends_with('\''))
    {
        return Some(PredicateLiteral::String(raw[1..raw.len() - 1].to_string()));
    }

    if raw == "true" {
        return Some(PredicateLiteral::Bool(true));
    }
    if raw == "false" {
        return Some(PredicateLiteral::Bool(false));
    }

    if raw.contains('.') {
        if let Ok(f) = raw.parse::<f64>() {
            return Some(PredicateLiteral::Float(f));
        }
    }

    if let Ok(i) = raw.parse::<i64>() {
        return Some(PredicateLiteral::Int(i));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_boolean_expression_and_or() {
        let groups = split_boolean_expression("a == 1 && b == 2 || c == 3").unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], vec!["a == 1".to_string(), "b == 2".to_string()]);
        assert_eq!(groups[1], vec!["c == 3".to_string()]);
    }

    #[test]
    fn test_split_boolean_expression_keeps_parenthesized_terms() {
        let groups = split_boolean_expression("(a == 1) && b == 2").unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0],
            vec!["(a == 1)".to_string(), "b == 2".to_string()]
        );
    }

    #[test]
    fn test_parse_comparison_value_types() {
        assert_eq!(
            parse_comparison("name == 'alice'"),
            Some(ComparisonPredicate {
                field: "name".to_string(),
                operator: ComparisonOperator::Eq,
                value: PredicateLiteral::String("alice".to_string())
            })
        );
        assert_eq!(
            parse_comparison("age >= 42"),
            Some(ComparisonPredicate {
                field: "age".to_string(),
                operator: ComparisonOperator::Ge,
                value: PredicateLiteral::Int(42)
            })
        );
        assert_eq!(
            parse_comparison("score < 3.14"),
            Some(ComparisonPredicate {
                field: "score".to_string(),
                operator: ComparisonOperator::Lt,
                value: PredicateLiteral::Float(3.14)
            })
        );
        assert_eq!(
            parse_comparison("active == true"),
            Some(ComparisonPredicate {
                field: "active".to_string(),
                operator: ComparisonOperator::Eq,
                value: PredicateLiteral::Bool(true)
            })
        );
    }
}
