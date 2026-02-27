//! SchemaService gRPC handler
//!
//! Handles schema registration and retrieval:
//! - RegisterSchema: Create or update an entity type schema
//! - GetSchema: Retrieve a schema by type name and version
//! - ListSchemas: List all registered schemas

use crate::authz::{authorize, principal_from_request};
use crate::error::ToStatus;
use pelago_core::schema::{
    EdgeDef as CoreEdgeDef, EdgeDirection as CoreEdgeDirection, EdgeTarget as CoreEdgeTarget,
    EntitySchema as CoreSchema, ExtrasPolicy as CoreExtrasPolicy, IndexType as CoreIndexType,
    OwnershipMode as CoreOwnershipMode, PropertyDef as CorePropertyDef,
    SchemaMeta as CoreSchemaMeta,
};
use pelago_core::{PropertyType as CorePropertyType, Value as CoreValue};
use pelago_proto::{
    schema_service_server::SchemaService, EdgeDef, EdgeDirectionDef, EdgeTarget, EntitySchema,
    ExtrasPolicy, GetSchemaRequest, GetSchemaResponse, IndexDefaultMode, IndexType,
    ListSchemasRequest, ListSchemasResponse, OwnershipMode, PropertyDef, PropertyType,
    RegisterSchemaRequest, RegisterSchemaResponse, SchemaMeta, Value,
};
use pelago_storage::{PelagoDb, SchemaRegistry};
use std::collections::HashMap;
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// Schema service implementation
pub struct SchemaServiceImpl {
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
}

impl SchemaServiceImpl {
    pub fn new(db: PelagoDb, schema_registry: Arc<SchemaRegistry>) -> Self {
        Self {
            db,
            schema_registry,
        }
    }
}

#[tonic::async_trait]
impl SchemaService for SchemaServiceImpl {
    async fn register_schema(
        &self,
        request: Request<RegisterSchemaRequest>,
    ) -> Result<Response<RegisterSchemaResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let proto_schema = req
            .schema
            .ok_or_else(|| Status::invalid_argument("missing schema"))?;
        let index_default_mode = IndexDefaultMode::try_from(req.index_default_mode)
            .unwrap_or(IndexDefaultMode::Unspecified);
        if index_default_mode != IndexDefaultMode::AutoByTypeV1 {
            return Err(Status::invalid_argument(
                "index_default_mode must be INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1",
            ));
        }
        authorize(
            &self.db,
            principal.as_ref(),
            "schema.register",
            &ctx.database,
            &ctx.namespace,
            &proto_schema.name,
        )
        .await?;

        // Convert proto schema to core schema
        let core_schema = proto_to_core_schema(&proto_schema, index_default_mode)?;

        // Register schema in FDB
        let version = self
            .schema_registry
            .register_schema(&ctx.database, &ctx.namespace, core_schema)
            .await
            .map_err(|e| e.into_status())?;

        Ok(Response::new(RegisterSchemaResponse {
            version,
            created: true,
        }))
    }

    async fn get_schema(
        &self,
        request: Request<GetSchemaRequest>,
    ) -> Result<Response<GetSchemaResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let version = req.version;
        authorize(
            &self.db,
            principal.as_ref(),
            "schema.read",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;

        // Fetch schema from FDB
        if version > 0 {
            // Get specific version - returns EntitySchema directly
            let schema = self
                .schema_registry
                .get_schema_version(&ctx.database, &ctx.namespace, &entity_type, version)
                .await
                .map_err(|e| e.into_status())?;

            match schema {
                Some(schema) => {
                    let proto_schema = core_to_proto_schema(&schema);
                    Ok(Response::new(GetSchemaResponse {
                        schema: Some(proto_schema),
                    }))
                }
                None => Err(Status::not_found(format!(
                    "schema '{}' version {} not found",
                    entity_type, version
                ))),
            }
        } else {
            // Get latest version - returns Arc<EntitySchema>
            let schema = self
                .schema_registry
                .get_schema(&ctx.database, &ctx.namespace, &entity_type)
                .await
                .map_err(|e| e.into_status())?;

            match schema {
                Some(schema) => {
                    let proto_schema = core_to_proto_schema(&*schema);
                    Ok(Response::new(GetSchemaResponse {
                        schema: Some(proto_schema),
                    }))
                }
                None => Err(Status::not_found(format!(
                    "schema '{}' not found",
                    entity_type
                ))),
            }
        }
    }

    async fn list_schemas(
        &self,
        request: Request<ListSchemasRequest>,
    ) -> Result<Response<ListSchemasResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "schema.read",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;

        // List schema names from FDB
        let schema_names = self
            .schema_registry
            .list_schemas(&ctx.database, &ctx.namespace)
            .await
            .map_err(|e| e.into_status())?;

        // Fetch full schemas for each name
        let mut proto_schemas = Vec::with_capacity(schema_names.len());
        for name in schema_names {
            if let Some(schema) = self
                .schema_registry
                .get_schema(&ctx.database, &ctx.namespace, &name)
                .await
                .map_err(|e| e.into_status())?
            {
                proto_schemas.push(core_to_proto_schema(&*schema));
            }
        }

        Ok(Response::new(ListSchemasResponse {
            schemas: proto_schemas,
        }))
    }
}

/// Convert proto EntitySchema to core EntitySchema
pub fn proto_to_core_schema(
    proto: &EntitySchema,
    index_default_mode: IndexDefaultMode,
) -> Result<CoreSchema, Status> {
    let mut schema = CoreSchema::new(&proto.name);

    for (name, prop_def) in &proto.properties {
        let core_prop = proto_to_core_property_def(prop_def, index_default_mode)?;
        schema = schema.with_property(name, core_prop);
    }

    for (name, edge_def) in &proto.edges {
        let core_edge = proto_to_core_edge_def(edge_def, index_default_mode)?;
        schema = schema.with_edge(name, core_edge);
    }

    if let Some(meta) = &proto.meta {
        schema = schema.with_meta(proto_to_core_schema_meta(meta));
    }

    // Storage assigns canonical versioning on register; preserve metadata fields for completeness.
    schema.created_at = proto.created_at;
    schema.created_by = proto.created_by.clone();

    Ok(schema)
}

/// Convert core EntitySchema to proto EntitySchema
pub fn core_to_proto_schema(core: &CoreSchema) -> EntitySchema {
    let mut properties = HashMap::new();
    let mut edges = HashMap::new();

    for (name, prop_def) in &core.properties {
        properties.insert(name.clone(), core_to_proto_property_def(prop_def));
    }

    for (name, edge_def) in &core.edges {
        edges.insert(name.clone(), core_to_proto_edge_def(edge_def));
    }

    EntitySchema {
        name: core.name.clone(),
        version: core.version,
        properties,
        edges,
        meta: Some(core_to_proto_schema_meta(&core.meta)),
        created_at: core.created_at,
        created_by: core.created_by.clone(),
    }
}

fn proto_to_core_edge_def(
    proto: &EdgeDef,
    index_default_mode: IndexDefaultMode,
) -> Result<CoreEdgeDef, Status> {
    let target_proto = proto
        .target
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("edge target is required"))?;
    let mut edge = CoreEdgeDef::new(proto_to_core_edge_target(target_proto)?);

    edge.direction = match EdgeDirectionDef::try_from(proto.direction) {
        Ok(EdgeDirectionDef::Outgoing) => CoreEdgeDirection::Outgoing,
        Ok(EdgeDirectionDef::Bidirectional) => CoreEdgeDirection::Bidirectional,
        _ => CoreEdgeDirection::Outgoing,
    };

    edge.ownership = match OwnershipMode::try_from(proto.ownership) {
        Ok(OwnershipMode::SourceSite) => CoreOwnershipMode::SourceSite,
        Ok(OwnershipMode::Independent) => CoreOwnershipMode::Independent,
        _ => CoreOwnershipMode::SourceSite,
    };

    if !proto.sort_key.is_empty() {
        edge.sort_key = Some(proto.sort_key.clone());
    }

    for (name, prop_def) in &proto.properties {
        let core_prop = proto_to_core_property_def(prop_def, index_default_mode)?;
        edge.properties.insert(name.clone(), core_prop);
    }

    Ok(edge)
}

fn core_to_proto_edge_def(core: &CoreEdgeDef) -> EdgeDef {
    let mut properties = HashMap::new();
    for (name, prop_def) in &core.properties {
        properties.insert(name.clone(), core_to_proto_property_def(prop_def));
    }

    EdgeDef {
        target: Some(core_to_proto_edge_target(&core.target)),
        direction: match core.direction {
            CoreEdgeDirection::Outgoing => EdgeDirectionDef::Outgoing as i32,
            CoreEdgeDirection::Bidirectional => EdgeDirectionDef::Bidirectional as i32,
        },
        properties,
        sort_key: core.sort_key.clone().unwrap_or_default(),
        ownership: match core.ownership {
            CoreOwnershipMode::SourceSite => OwnershipMode::SourceSite as i32,
            CoreOwnershipMode::Independent => OwnershipMode::Independent as i32,
        },
    }
}

fn proto_to_core_edge_target(proto: &EdgeTarget) -> Result<CoreEdgeTarget, Status> {
    match proto.kind.as_ref() {
        Some(pelago_proto::edge_target::Kind::SpecificType(name)) => {
            Ok(CoreEdgeTarget::specific(name.clone()))
        }
        Some(pelago_proto::edge_target::Kind::Polymorphic(_)) => Ok(CoreEdgeTarget::polymorphic()),
        None => Err(Status::invalid_argument("edge target kind is required")),
    }
}

fn core_to_proto_edge_target(core: &CoreEdgeTarget) -> EdgeTarget {
    let kind = match core {
        CoreEdgeTarget::Specific(name) => {
            pelago_proto::edge_target::Kind::SpecificType(name.clone())
        }
        CoreEdgeTarget::Polymorphic => pelago_proto::edge_target::Kind::Polymorphic(true),
    };
    EdgeTarget { kind: Some(kind) }
}

fn proto_to_core_schema_meta(proto: &SchemaMeta) -> CoreSchemaMeta {
    CoreSchemaMeta {
        allow_undeclared_edges: proto.allow_undeclared_edges,
        extras_policy: match ExtrasPolicy::try_from(proto.extras_policy) {
            Ok(ExtrasPolicy::Allow) => CoreExtrasPolicy::Allow,
            Ok(ExtrasPolicy::Warn) => CoreExtrasPolicy::Warn,
            _ => CoreExtrasPolicy::Reject,
        },
    }
}

fn core_to_proto_schema_meta(core: &CoreSchemaMeta) -> SchemaMeta {
    SchemaMeta {
        allow_undeclared_edges: core.allow_undeclared_edges,
        extras_policy: match core.extras_policy {
            CoreExtrasPolicy::Reject => ExtrasPolicy::Reject as i32,
            CoreExtrasPolicy::Allow => ExtrasPolicy::Allow as i32,
            CoreExtrasPolicy::Warn => ExtrasPolicy::Warn as i32,
        },
    }
}

/// Convert proto PropertyDef to core PropertyDef
fn proto_to_core_property_def(
    proto: &PropertyDef,
    index_default_mode: IndexDefaultMode,
) -> Result<CorePropertyDef, Status> {
    let prop_type = match PropertyType::try_from(proto.r#type) {
        Ok(PropertyType::String) => CorePropertyType::String,
        Ok(PropertyType::Int) => CorePropertyType::Int,
        Ok(PropertyType::Float) => CorePropertyType::Float,
        Ok(PropertyType::Bool) => CorePropertyType::Bool,
        Ok(PropertyType::Timestamp) => CorePropertyType::Timestamp,
        Ok(PropertyType::Bytes) => CorePropertyType::Bytes,
        _ => return Err(Status::invalid_argument("invalid property type")),
    };

    let mut def = CorePropertyDef::new(prop_type);

    if proto.required {
        def = def.required();
    }

    let index_type = match proto.index {
        Some(raw_index) => match IndexType::try_from(raw_index) {
            Ok(IndexType::None) => CoreIndexType::None,
            Ok(IndexType::Unique) => CoreIndexType::Unique,
            Ok(IndexType::Equality) => CoreIndexType::Equality,
            Ok(IndexType::Range) => CoreIndexType::Range,
            _ => return Err(Status::invalid_argument("invalid index type")),
        },
        None => infer_index_type(prop_type, index_default_mode)?,
    };
    def = def.with_index(index_type);

    if let Some(default) = &proto.default_value {
        if let Some(core_default) = proto_to_core_value(default) {
            def = def.with_default(core_default);
        }
    }

    Ok(def)
}

fn infer_index_type(
    prop_type: CorePropertyType,
    index_default_mode: IndexDefaultMode,
) -> Result<CoreIndexType, Status> {
    match index_default_mode {
        IndexDefaultMode::AutoByTypeV1 => Ok(match prop_type {
            CorePropertyType::Int | CorePropertyType::Float | CorePropertyType::Timestamp => {
                CoreIndexType::Range
            }
            CorePropertyType::Bool => CoreIndexType::Equality,
            CorePropertyType::String | CorePropertyType::Bytes => CoreIndexType::None,
        }),
        _ => Err(Status::invalid_argument(
            "index_default_mode must be INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1",
        )),
    }
}

/// Convert core PropertyDef to proto PropertyDef
fn core_to_proto_property_def(core: &CorePropertyDef) -> PropertyDef {
    let prop_type = match core.property_type {
        CorePropertyType::String => PropertyType::String,
        CorePropertyType::Int => PropertyType::Int,
        CorePropertyType::Float => PropertyType::Float,
        CorePropertyType::Bool => PropertyType::Bool,
        CorePropertyType::Timestamp => PropertyType::Timestamp,
        CorePropertyType::Bytes => PropertyType::Bytes,
    };

    let index = match core.index {
        CoreIndexType::Unique => IndexType::Unique,
        CoreIndexType::Equality => IndexType::Equality,
        CoreIndexType::Range => IndexType::Range,
        CoreIndexType::None => IndexType::None,
    };

    PropertyDef {
        r#type: prop_type as i32,
        required: core.required,
        index: Some(index as i32),
        default_value: core.default_value.as_ref().map(core_to_proto_value),
    }
}

/// Convert proto Value to core Value
fn proto_to_core_value(proto: &Value) -> Option<CoreValue> {
    use pelago_proto::value::Kind;

    proto.kind.as_ref().map(|kind| match kind {
        Kind::StringValue(s) => CoreValue::String(s.clone()),
        Kind::IntValue(i) => CoreValue::Int(*i),
        Kind::FloatValue(f) => CoreValue::Float(*f),
        Kind::BoolValue(b) => CoreValue::Bool(*b),
        Kind::TimestampValue(t) => CoreValue::Timestamp(*t),
        Kind::BytesValue(b) => CoreValue::Bytes(b.clone()),
        Kind::NullValue(_) => CoreValue::Null,
    })
}

/// Convert core Value to proto Value
fn core_to_proto_value(core: &CoreValue) -> Value {
    use pelago_proto::value::Kind;

    let kind = match core {
        CoreValue::String(s) => Kind::StringValue(s.clone()),
        CoreValue::Int(i) => Kind::IntValue(*i),
        CoreValue::Float(f) => Kind::FloatValue(*f),
        CoreValue::Bool(b) => Kind::BoolValue(*b),
        CoreValue::Timestamp(t) => Kind::TimestampValue(*t),
        CoreValue::Bytes(b) => Kind::BytesValue(b.clone()),
        CoreValue::Null => Kind::NullValue(true),
    };

    Value { kind: Some(kind) }
}

/// Convert core properties to proto properties
pub fn core_to_proto_properties(props: &HashMap<String, CoreValue>) -> HashMap<String, Value> {
    props
        .iter()
        .map(|(k, v)| (k.clone(), core_to_proto_value(v)))
        .collect()
}

/// Convert proto properties to core properties
pub fn proto_to_core_properties(props: &HashMap<String, Value>) -> HashMap<String, CoreValue> {
    props
        .iter()
        .filter_map(|(k, v)| proto_to_core_value(v).map(|cv| (k.clone(), cv)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_query::plan::QueryPlan;
    use pelago_query::planner::QueryPlanner;
    use std::collections::HashMap;

    fn prop(r#type: PropertyType, index: Option<IndexType>) -> PropertyDef {
        PropertyDef {
            r#type: r#type as i32,
            required: false,
            index: index.map(|i| i as i32),
            default_value: None,
        }
    }

    fn entity_with_single_prop(name: &str, prop_name: &str, prop_def: PropertyDef) -> EntitySchema {
        let mut properties = HashMap::new();
        properties.insert(prop_name.to_string(), prop_def);
        EntitySchema {
            name: name.to_string(),
            version: 0,
            properties,
            edges: HashMap::new(),
            meta: None,
            created_at: 0,
            created_by: String::new(),
        }
    }

    #[test]
    fn infers_range_for_int_when_index_omitted() {
        let core = proto_to_core_property_def(
            &prop(PropertyType::Int, None),
            IndexDefaultMode::AutoByTypeV1,
        )
        .expect("int property should parse");
        assert_eq!(core.index, CoreIndexType::Range);
    }

    #[test]
    fn infers_equality_for_bool_when_index_omitted() {
        let core = proto_to_core_property_def(
            &prop(PropertyType::Bool, None),
            IndexDefaultMode::AutoByTypeV1,
        )
        .expect("bool property should parse");
        assert_eq!(core.index, CoreIndexType::Equality);
    }

    #[test]
    fn infers_none_for_string_when_index_omitted() {
        let core = proto_to_core_property_def(
            &prop(PropertyType::String, None),
            IndexDefaultMode::AutoByTypeV1,
        )
        .expect("string property should parse");
        assert_eq!(core.index, CoreIndexType::None);
    }

    #[test]
    fn explicit_none_overrides_type_default() {
        let core = proto_to_core_property_def(
            &prop(PropertyType::Int, Some(IndexType::None)),
            IndexDefaultMode::AutoByTypeV1,
        )
        .expect("int property with explicit none should parse");
        assert_eq!(core.index, CoreIndexType::None);
    }

    #[test]
    fn reject_unspecified_index_default_mode() {
        let err = proto_to_core_property_def(
            &prop(PropertyType::Int, None),
            IndexDefaultMode::Unspecified,
        )
        .expect_err("unspecified mode must be rejected");
        assert!(err.message().contains("INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1"));
    }

    #[test]
    fn inferred_range_index_drives_index_scan_planning() {
        let schema = entity_with_single_prop("Person", "age", prop(PropertyType::Int, None));
        let core = proto_to_core_schema(&schema, IndexDefaultMode::AutoByTypeV1)
            .expect("schema conversion should succeed");
        let plan = QueryPlanner::plan("Person", "age >= 30", &core, None, None)
            .expect("planning should succeed");

        assert!(matches!(plan.primary_plan, QueryPlan::IndexScan { .. }));
    }

    #[test]
    fn explicit_none_keeps_full_scan_planning() {
        let schema = entity_with_single_prop(
            "Person",
            "age",
            prop(PropertyType::Int, Some(IndexType::None)),
        );
        let core = proto_to_core_schema(&schema, IndexDefaultMode::AutoByTypeV1)
            .expect("schema conversion should succeed");
        let plan = QueryPlanner::plan("Person", "age >= 30", &core, None, None)
            .expect("planning should succeed");

        assert!(matches!(plan.primary_plan, QueryPlan::FullScan));
    }
}
