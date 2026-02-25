//! CEL expression parsing and type-checking
//!
//! - Build type environment from EntitySchema
//! - Parse CEL → AST
//! - Type-check against schema
//! - Null semantics: comparisons return false
//!
//! This module uses cel_interpreter for expression evaluation.

use cel_interpreter::{Context, Program, Value as CelValue};
use pelago_core::schema::EntitySchema;
use pelago_core::{PelagoError, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// CEL type environment built from an entity schema
pub struct CelEnvironment {
    /// The entity schema this environment is for
    schema: Arc<EntitySchema>,
}

impl CelEnvironment {
    /// Create a new CEL environment from an entity schema
    pub fn new(schema: Arc<EntitySchema>) -> Self {
        Self { schema }
    }

    /// Compile a CEL expression for this environment
    pub fn compile(&self, expression: &str) -> Result<CelExpression, PelagoError> {
        let program = Program::compile(expression).map_err(|e| PelagoError::CelSyntax {
            expression: expression.to_string(),
            message: e.to_string(),
        })?;

        Ok(CelExpression {
            program,
            expression: expression.to_string(),
            schema: Arc::clone(&self.schema),
        })
    }

    /// Validate that a CEL expression is valid for this schema
    /// Returns the expression string if valid
    pub fn validate(&self, expression: &str) -> Result<String, PelagoError> {
        // Try to compile - this catches syntax errors
        let _program = Program::compile(expression).map_err(|e| PelagoError::CelSyntax {
            expression: expression.to_string(),
            message: e.to_string(),
        })?;

        // The expression is currently considered valid if it compiles.
        // Full schema-aware type checking requires walking the CEL AST.

        Ok(expression.to_string())
    }
}

/// A compiled CEL expression
pub struct CelExpression {
    program: Program,
    expression: String,
    schema: Arc<EntitySchema>,
}

impl CelExpression {
    /// Get the original expression string
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Evaluate the expression against a set of properties
    /// Returns true if the node matches the filter, false otherwise
    /// Null semantics: comparisons with null return false
    pub fn evaluate(&self, properties: &HashMap<String, Value>) -> Result<bool, PelagoError> {
        let mut context = Context::default();

        // Add all properties to the CEL context
        for (key, value) in properties {
            let cel_value = value_to_cel(value);
            context.add_variable(key, cel_value).ok();
        }

        // Add missing schema properties as null
        for prop_name in self.schema.properties.keys() {
            if !properties.contains_key(prop_name) {
                context.add_variable(prop_name, CelValue::Null).ok();
            }
        }

        // Evaluate the expression
        match self.program.execute(&context) {
            Ok(CelValue::Bool(b)) => Ok(b),
            Ok(CelValue::Null) => Ok(false), // Null comparison result is false
            Ok(other) => Err(PelagoError::CelType {
                field: "expression".to_string(),
                message: format!("Expected bool, got {}", cel_type_name(&other)),
            }),
            Err(e) => {
                // CEL evaluation errors (like null comparisons) return false
                // per the spec's null semantics
                tracing::debug!("CEL evaluation error (returning false): {}", e);
                Ok(false)
            }
        }
    }

    /// Check if the expression matches null for a specific field
    /// Used for queries like `age == null` to find nodes without that property
    pub fn matches_null_field(&self, field: &str) -> bool {
        // Simple heuristic: check if the expression is exactly "field == null"
        let null_check = format!("{} == null", field);
        self.expression.trim() == null_check
    }
}

/// Convert PelagoDB Value to CEL Value
fn value_to_cel(value: &Value) -> CelValue {
    match value {
        Value::String(s) => CelValue::String(Arc::new(s.clone())),
        Value::Int(n) => CelValue::Int(*n),
        Value::Float(f) => CelValue::Float(*f),
        Value::Bool(b) => CelValue::Bool(*b),
        Value::Timestamp(t) => CelValue::Int(*t), // Timestamps as int micros
        Value::Bytes(b) => CelValue::Bytes(Arc::new(b.clone())),
        Value::Null => CelValue::Null,
    }
}

/// Get a type name for a CEL value (for error messages)
fn cel_type_name(value: &CelValue) -> String {
    match value {
        CelValue::Bool(_) => "bool".to_string(),
        CelValue::Int(_) => "int".to_string(),
        CelValue::UInt(_) => "uint".to_string(),
        CelValue::Float(_) => "float".to_string(),
        CelValue::String(_) => "string".to_string(),
        CelValue::Bytes(_) => "bytes".to_string(),
        CelValue::List(_) => "list".to_string(),
        CelValue::Map(_) => "map".to_string(),
        CelValue::Null => "null".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Quick filter check without full CEL compilation
/// Used for simple predicates that can be evaluated directly
pub fn quick_filter(
    properties: &HashMap<String, Value>,
    property: &str,
    operator: &str,
    target: &Value,
) -> bool {
    let actual = match properties.get(property) {
        Some(v) => v,
        None => return operator == "==" && matches!(target, Value::Null),
    };

    // Null comparisons
    if matches!(actual, Value::Null) {
        return operator == "==" && matches!(target, Value::Null);
    }

    match operator {
        "==" => actual == target,
        "!=" => actual != target,
        ">" => compare_values(actual, target) == Some(std::cmp::Ordering::Greater),
        ">=" => {
            matches!(
                compare_values(actual, target),
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            )
        }
        "<" => compare_values(actual, target) == Some(std::cmp::Ordering::Less),
        "<=" => {
            matches!(
                compare_values(actual, target),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            )
        }
        _ => false,
    }
}

/// Compare two values of compatible types
fn compare_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        (Value::Timestamp(a), Value::Timestamp(b)) => Some(a.cmp(b)),
        (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
        // Cross-type numeric comparison
        (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
        (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_core::schema::PropertyDef;
    use pelago_core::PropertyType;

    fn make_test_schema() -> Arc<EntitySchema> {
        Arc::new(
            EntitySchema::new("Person")
                .with_property("name", PropertyDef::new(PropertyType::String))
                .with_property("age", PropertyDef::new(PropertyType::Int))
                .with_property("score", PropertyDef::new(PropertyType::Float))
                .with_property("active", PropertyDef::new(PropertyType::Bool)),
        )
    }

    #[test]
    fn test_compile_valid_expression() {
        let schema = make_test_schema();
        let env = CelEnvironment::new(schema);

        let expr = env.compile("age >= 30").unwrap();
        assert_eq!(expr.expression(), "age >= 30");
    }

    #[test]
    fn test_compile_invalid_syntax() {
        let schema = make_test_schema();
        let env = CelEnvironment::new(schema);

        let result = env.compile("age >== 30");
        assert!(result.is_err());
    }

    #[test]
    fn test_evaluate_simple_comparison() {
        let schema = make_test_schema();
        let env = CelEnvironment::new(schema);
        let expr = env.compile("age >= 30").unwrap();

        let mut props = HashMap::new();
        props.insert("age".to_string(), Value::Int(35));

        assert!(expr.evaluate(&props).unwrap());

        props.insert("age".to_string(), Value::Int(25));
        assert!(!expr.evaluate(&props).unwrap());
    }

    #[test]
    fn test_evaluate_string_equality() {
        let schema = make_test_schema();
        let env = CelEnvironment::new(schema);
        let expr = env.compile("name == 'Alice'").unwrap();

        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".to_string()));

        assert!(expr.evaluate(&props).unwrap());

        props.insert("name".to_string(), Value::String("Bob".to_string()));
        assert!(!expr.evaluate(&props).unwrap());
    }

    #[test]
    fn test_null_semantics() {
        let schema = make_test_schema();
        let env = CelEnvironment::new(schema);

        // Comparing with null field returns false
        let expr = env.compile("age >= 30").unwrap();
        let props = HashMap::new(); // No age property

        // Should return false (not error) for missing/null field
        assert!(!expr.evaluate(&props).unwrap());
    }

    #[test]
    fn test_quick_filter() {
        let mut props = HashMap::new();
        props.insert("age".to_string(), Value::Int(30));

        assert!(quick_filter(&props, "age", "==", &Value::Int(30)));
        assert!(quick_filter(&props, "age", ">=", &Value::Int(25)));
        assert!(quick_filter(&props, "age", "<=", &Value::Int(35)));
        assert!(!quick_filter(&props, "age", ">", &Value::Int(30)));
    }

    #[test]
    fn test_quick_filter_null() {
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::Null);

        assert!(quick_filter(&props, "name", "==", &Value::Null));
        assert!(quick_filter(&props, "missing", "==", &Value::Null));
    }
}
