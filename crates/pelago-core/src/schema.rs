//! Schema types for entity type definitions
//!
//! Schemas define the structure of nodes and edges in PelagoDB:
//! - Property definitions with types, indexes, and defaults
//! - Edge definitions with targets, directions, and ownership
//! - Schema metadata controlling validation behavior

use crate::PropertyType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Complete entity schema definition
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntitySchema {
    /// Entity type name (e.g., "Person", "Company")
    pub name: String,
    /// Schema version (incremented on each update)
    pub version: u32,
    /// Property definitions
    pub properties: HashMap<String, PropertyDef>,
    /// Edge definitions
    pub edges: HashMap<String, EdgeDef>,
    /// Schema metadata
    pub meta: SchemaMeta,
    /// Creation timestamp (Unix microseconds)
    pub created_at: i64,
    /// Creator identifier
    pub created_by: String,
}

impl EntitySchema {
    /// Create a new schema with default metadata
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: 1,
            properties: HashMap::new(),
            edges: HashMap::new(),
            meta: SchemaMeta::default(),
            created_at: 0,
            created_by: String::new(),
        }
    }

    /// Add a property definition
    pub fn with_property(mut self, name: impl Into<String>, prop: PropertyDef) -> Self {
        self.properties.insert(name.into(), prop);
        self
    }

    /// Add an edge definition
    pub fn with_edge(mut self, name: impl Into<String>, edge: EdgeDef) -> Self {
        self.edges.insert(name.into(), edge);
        self
    }

    /// Set the schema metadata
    pub fn with_meta(mut self, meta: SchemaMeta) -> Self {
        self.meta = meta;
        self
    }
}

/// Property definition within a schema
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropertyDef {
    /// Property type
    pub property_type: PropertyType,
    /// Whether this property is required
    pub required: bool,
    /// Index type for this property
    pub index: IndexType,
    /// Default value (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<crate::Value>,
}

impl PropertyDef {
    pub fn new(property_type: PropertyType) -> Self {
        Self {
            property_type,
            required: false,
            index: IndexType::None,
            default_value: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn with_index(mut self, index: IndexType) -> Self {
        self.index = index;
        self
    }

    pub fn with_default(mut self, value: crate::Value) -> Self {
        self.default_value = Some(value);
        self
    }
}

/// Edge definition within a schema
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeDef {
    /// Target node type constraint
    pub target: EdgeTarget,
    /// Edge direction
    pub direction: EdgeDirection,
    /// Edge properties
    pub properties: HashMap<String, PropertyDef>,
    /// Sort key property (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_key: Option<String>,
    /// Ownership mode
    pub ownership: OwnershipMode,
}

impl EdgeDef {
    pub fn new(target: EdgeTarget) -> Self {
        Self {
            target,
            direction: EdgeDirection::Outgoing,
            properties: HashMap::new(),
            sort_key: None,
            ownership: OwnershipMode::SourceSite,
        }
    }

    pub fn bidirectional(mut self) -> Self {
        self.direction = EdgeDirection::Bidirectional;
        self
    }

    pub fn with_sort_key(mut self, key: impl Into<String>) -> Self {
        self.sort_key = Some(key.into());
        self
    }

    pub fn with_property(mut self, name: impl Into<String>, prop: PropertyDef) -> Self {
        self.properties.insert(name.into(), prop);
        self
    }
}

/// Target type constraint for edges
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EdgeTarget {
    /// Specific entity type (e.g., "Company")
    Specific(String),
    /// Any entity type (polymorphic)
    Polymorphic,
}

impl EdgeTarget {
    pub fn specific(name: impl Into<String>) -> Self {
        EdgeTarget::Specific(name.into())
    }

    pub fn polymorphic() -> Self {
        EdgeTarget::Polymorphic
    }

    pub fn is_polymorphic(&self) -> bool {
        matches!(self, EdgeTarget::Polymorphic)
    }
}

/// Edge direction
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeDirection {
    /// One-way edge (OUT only)
    Outgoing,
    /// Two-way edge (creates reverse edge automatically)
    Bidirectional,
}

impl Default for EdgeDirection {
    fn default() -> Self {
        EdgeDirection::Outgoing
    }
}

/// Index type for properties
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexType {
    /// No index
    #[default]
    None,
    /// Unique index (point lookup, enforces uniqueness)
    Unique,
    /// Equality index (point lookup, allows duplicates)
    Equality,
    /// Range index (supports >, <, >=, <= queries)
    Range,
}

impl IndexType {
    /// Heuristic selectivity for query planning
    pub fn selectivity(&self) -> f64 {
        match self {
            IndexType::None => 1.0,       // Full scan
            IndexType::Unique => 0.01,    // ~1% selectivity
            IndexType::Equality => 0.10,  // ~10% selectivity
            IndexType::Range => 0.50,     // ~50% selectivity
        }
    }

    pub fn is_indexed(&self) -> bool {
        !matches!(self, IndexType::None)
    }
}

/// Ownership mode for edges (affects multi-site replication)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipMode {
    /// Edge owned by source node's site
    #[default]
    SourceSite,
    /// Edge independently owned (can be modified at any site)
    Independent,
}

/// Schema metadata controlling validation behavior
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SchemaMeta {
    /// Allow edges not declared in schema
    #[serde(default)]
    pub allow_undeclared_edges: bool,
    /// Policy for extra properties not in schema
    #[serde(default)]
    pub extras_policy: ExtrasPolicy,
}

impl SchemaMeta {
    pub fn strict() -> Self {
        Self {
            allow_undeclared_edges: false,
            extras_policy: ExtrasPolicy::Reject,
        }
    }

    pub fn permissive() -> Self {
        Self {
            allow_undeclared_edges: true,
            extras_policy: ExtrasPolicy::Allow,
        }
    }
}

/// Policy for handling extra properties
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtrasPolicy {
    /// Reject nodes with extra properties
    Reject,
    /// Allow extra properties (stored but not validated)
    #[default]
    Allow,
    /// Allow but log a warning
    Warn,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    #[test]
    fn test_schema_builder() {
        let schema = EntitySchema::new("Person")
            .with_property(
                "name",
                PropertyDef::new(PropertyType::String)
                    .required()
                    .with_index(IndexType::Equality),
            )
            .with_property(
                "email",
                PropertyDef::new(PropertyType::String).with_index(IndexType::Unique),
            )
            .with_property(
                "age",
                PropertyDef::new(PropertyType::Int)
                    .with_index(IndexType::Range)
                    .with_default(Value::Int(0)),
            )
            .with_edge(
                "WORKS_AT",
                EdgeDef::new(EdgeTarget::specific("Company"))
                    .with_sort_key("since")
                    .with_property("since", PropertyDef::new(PropertyType::Timestamp)),
            )
            .with_meta(SchemaMeta::strict());

        assert_eq!(schema.name, "Person");
        assert_eq!(schema.properties.len(), 3);
        assert!(schema.properties.get("name").unwrap().required);
        assert_eq!(
            schema.properties.get("email").unwrap().index,
            IndexType::Unique
        );
        assert!(schema.edges.contains_key("WORKS_AT"));
    }

    #[test]
    fn test_index_selectivity() {
        assert_eq!(IndexType::Unique.selectivity(), 0.01);
        assert_eq!(IndexType::Equality.selectivity(), 0.10);
        assert_eq!(IndexType::Range.selectivity(), 0.50);
        assert_eq!(IndexType::None.selectivity(), 1.0);
    }
}
