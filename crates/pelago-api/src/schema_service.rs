//! SchemaService gRPC handler
//!
//! Handles schema registration and retrieval:
//! - RegisterSchema: Create or update an entity type schema
//! - GetSchema: Retrieve a schema by type name and version
//! - ListSchemas: List all registered schemas

use crate::error::IntoStatus;
use pelago_core::schema::{EntitySchema as CoreSchema, IndexType as CoreIndexType, PropertyDef as CorePropertyDef};
use pelago_core::{PelagoError, PropertyType as CorePropertyType, Value as CoreValue};
use pelago_proto::{
    schema_service_server::SchemaService, EntitySchema, GetSchemaRequest, GetSchemaResponse,
    IndexType, ListSchemasRequest, ListSchemasResponse, PropertyDef, PropertyType,
    RegisterSchemaRequest, RegisterSchemaResponse, Value,
};
use pelago_storage::PelagoDb;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// Schema service implementation
pub struct SchemaServiceImpl {
    db: Arc<PelagoDb>,
}

impl SchemaServiceImpl {
    pub fn new(db: Arc<PelagoDb>) -> Self {
        Self { db }
    }
}

#[tonic::async_trait]
impl SchemaService for SchemaServiceImpl {
    async fn register_schema(
        &self,
        request: Request<RegisterSchemaRequest>,
    ) -> Result<Response<RegisterSchemaResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let proto_schema = req.schema.ok_or_else(|| Status::invalid_argument("missing schema"))?;

        // Convert proto schema to core schema
        let _core_schema = proto_to_core_schema(&proto_schema)?;

        // TODO: Store schema in FDB when integration is ready
        // For now, return success with version 1
        Ok(Response::new(RegisterSchemaResponse {
            version: 1,
            created: true,
        }))
    }

    async fn get_schema(
        &self,
        request: Request<GetSchemaRequest>,
    ) -> Result<Response<GetSchemaResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _entity_type = req.entity_type;
        let _version = req.version;

        // TODO: Fetch schema from FDB when integration is ready
        Err(Status::not_found("schema not found"))
    }

    async fn list_schemas(
        &self,
        request: Request<ListSchemasRequest>,
    ) -> Result<Response<ListSchemasResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;

        // TODO: List schemas from FDB when integration is ready
        Ok(Response::new(ListSchemasResponse { schemas: vec![] }))
    }
}

/// Convert proto EntitySchema to core EntitySchema
fn proto_to_core_schema(proto: &EntitySchema) -> Result<CoreSchema, Status> {
    let mut schema = CoreSchema::new(&proto.name);

    for (name, prop_def) in &proto.properties {
        let core_prop = proto_to_core_property_def(prop_def)?;
        schema = schema.with_property(name, core_prop);
    }

    Ok(schema)
}

/// Convert proto PropertyDef to core PropertyDef
fn proto_to_core_property_def(proto: &PropertyDef) -> Result<CorePropertyDef, Status> {
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

    let index_type = match IndexType::try_from(proto.index) {
        Ok(IndexType::Unique) => CoreIndexType::Unique,
        Ok(IndexType::Equality) => CoreIndexType::Equality,
        Ok(IndexType::Range) => CoreIndexType::Range,
        _ => CoreIndexType::None,
    };
    def = def.with_index(index_type);

    if let Some(default) = &proto.default_value {
        if let Some(core_default) = proto_to_core_value(default) {
            def = def.with_default(core_default);
        }
    }

    Ok(def)
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
