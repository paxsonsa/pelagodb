//! Node CRUD operations
//!
//! FDB Key Layout:
//! ```text
//! Data:     (db, ns, data, entity_type, node_id) → CBOR properties
//! Locality: (db, ns, loc, entity_type, site_id, node_id) → empty
//! ```
//!
//! Operations:
//! - create_node: Validate, allocate ID, write data + indexes + CDC
//! - get_node: Point lookup by ID
//! - update_node: Read-modify-write with index diff
//! - delete_node: Remove data, indexes, cascade edges, emit CDC

use pelago_core::schema::{EntitySchema, ExtrasPolicy};
use pelago_core::{PelagoError, Value};
use std::collections::HashMap;

/// Validate node properties against a schema
pub fn validate_properties(
    entity_type: &str,
    schema: &EntitySchema,
    properties: &HashMap<String, Value>,
) -> Result<(), PelagoError> {
    // Check required properties
    for (prop_name, prop_def) in &schema.properties {
        if prop_def.required {
            match properties.get(prop_name) {
                None => {
                    return Err(PelagoError::MissingRequired {
                        entity_type: entity_type.to_string(),
                        field: prop_name.clone(),
                    });
                }
                Some(Value::Null) => {
                    return Err(PelagoError::MissingRequired {
                        entity_type: entity_type.to_string(),
                        field: prop_name.clone(),
                    });
                }
                _ => {}
            }
        }
    }

    // Check property types
    for (prop_name, value) in properties {
        if let Some(prop_def) = schema.properties.get(prop_name) {
            if !prop_def.property_type.matches(value) {
                return Err(PelagoError::TypeMismatch {
                    field: prop_name.clone(),
                    expected: prop_def.property_type.to_string(),
                    actual: value.type_name().to_string(),
                });
            }
        } else {
            // Extra property handling
            match schema.meta.extras_policy {
                ExtrasPolicy::Reject => {
                    return Err(PelagoError::ExtraProperty {
                        field: prop_name.clone(),
                    });
                }
                ExtrasPolicy::Warn => {
                    // In a real implementation, we'd log a warning
                    tracing::warn!(
                        entity_type = entity_type,
                        field = prop_name,
                        "Extra property not in schema"
                    );
                }
                ExtrasPolicy::Allow => {
                    // Silently allow
                }
            }
        }
    }

    Ok(())
}

/// Apply default values from schema to properties
pub fn apply_defaults(
    schema: &EntitySchema,
    properties: &mut HashMap<String, Value>,
) {
    for (prop_name, prop_def) in &schema.properties {
        if !properties.contains_key(prop_name) {
            if let Some(ref default) = prop_def.default_value {
                properties.insert(prop_name.clone(), default.clone());
            }
        }
    }
}

/// Node data structure for storage
#[derive(Debug, Clone)]
pub struct StoredNode {
    pub id: String,
    pub entity_type: String,
    pub properties: HashMap<String, Value>,
    pub locality: u8,
    pub created_at: i64,
    pub updated_at: i64,
}

impl StoredNode {
    pub fn new(
        id: String,
        entity_type: String,
        properties: HashMap<String, Value>,
        site_id: u8,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        Self {
            id,
            entity_type,
            properties,
            locality: site_id,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_core::schema::{IndexType, PropertyDef, SchemaMeta};
    use pelago_core::PropertyType;

    fn make_person_schema() -> EntitySchema {
        EntitySchema::new("Person")
            .with_property(
                "name",
                PropertyDef::new(PropertyType::String).required(),
            )
            .with_property(
                "age",
                PropertyDef::new(PropertyType::Int),
            )
            .with_property(
                "email",
                PropertyDef::new(PropertyType::String)
                    .with_index(IndexType::Unique),
            )
            .with_meta(SchemaMeta::strict())
    }

    #[test]
    fn test_validate_valid_properties() {
        let schema = make_person_schema();
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));
        props.insert("age".to_string(), Value::Int(30));
        props.insert("email".to_string(), Value::String("alice@example.com".into()));

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_missing_required() {
        let schema = make_person_schema();
        let props = HashMap::new(); // Missing "name"

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, PelagoError::MissingRequired { field, .. } if field == "name"));
    }

    #[test]
    fn test_validate_null_required() {
        let schema = make_person_schema();
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::Null);

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PelagoError::MissingRequired { .. }));
    }

    #[test]
    fn test_validate_type_mismatch() {
        let schema = make_person_schema();
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));
        props.insert("age".to_string(), Value::String("thirty".into())); // Should be Int

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, PelagoError::TypeMismatch { field, .. } if field == "age"));
    }

    #[test]
    fn test_validate_extra_property_rejected() {
        let schema = make_person_schema();
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));
        props.insert("unknown".to_string(), Value::String("value".into()));

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PelagoError::ExtraProperty { field } if field == "unknown"));
    }

    #[test]
    fn test_validate_extra_property_allowed() {
        let schema = EntitySchema::new("Person")
            .with_property("name", PropertyDef::new(PropertyType::String).required())
            .with_meta(SchemaMeta::permissive());

        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));
        props.insert("unknown".to_string(), Value::String("value".into()));

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_ok());
    }

    #[test]
    fn test_apply_defaults() {
        let schema = EntitySchema::new("Person")
            .with_property(
                "name",
                PropertyDef::new(PropertyType::String).required(),
            )
            .with_property(
                "status",
                PropertyDef::new(PropertyType::String)
                    .with_default(Value::String("active".into())),
            );

        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));

        apply_defaults(&schema, &mut props);

        assert_eq!(props.get("status"), Some(&Value::String("active".into())));
    }

    #[test]
    fn test_apply_defaults_does_not_override() {
        let schema = EntitySchema::new("Person")
            .with_property(
                "status",
                PropertyDef::new(PropertyType::String)
                    .with_default(Value::String("active".into())),
            );

        let mut props = HashMap::new();
        props.insert("status".to_string(), Value::String("inactive".into()));

        apply_defaults(&schema, &mut props);

        // Should NOT override existing value
        assert_eq!(props.get("status"), Some(&Value::String("inactive".into())));
    }
}
