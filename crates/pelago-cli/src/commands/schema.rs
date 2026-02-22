use crate::connection::GrpcConnection;
use crate::output::OutputFormat;
use pelago_proto::{
    EntitySchema, ExtrasPolicy, GetSchemaRequest, IndexType, ListSchemasRequest, PropertyDef,
    PropertyType, RegisterSchemaRequest, RequestContext, SchemaMeta,
};

#[derive(clap::Args)]
pub struct SchemaArgs {
    #[command(subcommand)]
    pub command: SchemaCommand,
}

#[derive(clap::Subcommand)]
pub enum SchemaCommand {
    /// Register a schema from a JSON file
    Register {
        /// Path to schema JSON file
        file: Option<String>,
        /// Inline JSON schema
        #[arg(long)]
        inline: Option<String>,
    },
    /// Get a schema by entity type
    Get {
        /// Entity type name
        entity_type: String,
        /// Schema version
        #[arg(long)]
        version: Option<u32>,
    },
    /// List all registered schemas
    List,
    /// Diff two schema versions
    Diff {
        /// Entity type name
        entity_type: String,
        /// First version
        v1: u32,
        /// Second version
        v2: u32,
    },
}

pub async fn run(
    args: SchemaArgs,
    server: &str,
    database: &str,
    namespace: &str,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = GrpcConnection::connect(server).await?;
    let context = RequestContext {
        database: database.to_string(),
        namespace: namespace.to_string(),
        site_id: String::new(),
        request_id: String::new(),
    };

    match args.command {
        SchemaCommand::Register { file, inline } => {
            let json_str = if let Some(path) = file {
                std::fs::read_to_string(&path)?
            } else if let Some(json) = inline {
                json
            } else {
                return Err("Either a file path or --inline JSON is required".into());
            };

            let json_val: serde_json::Value = serde_json::from_str(&json_str)?;
            let schema = json_to_proto_schema(&json_val)?;

            let mut client = conn.schema_client();
            let resp = client
                .register_schema(RegisterSchemaRequest {
                    context: Some(context),
                    schema: Some(schema),
                })
                .await?
                .into_inner();

            match format {
                OutputFormat::Json => {
                    let json = serde_json::json!({
                        "version": resp.version,
                        "created": resp.created,
                    });
                    crate::output::print_json(&json);
                }
                _ => {
                    println!(
                        "Schema registered (version: {}, new: {})",
                        resp.version, resp.created
                    );
                }
            }
        }
        SchemaCommand::Get {
            entity_type,
            version,
        } => {
            let mut client = conn.schema_client();
            let resp = client
                .get_schema(GetSchemaRequest {
                    context: Some(context),
                    entity_type,
                    version: version.unwrap_or(0),
                })
                .await?
                .into_inner();

            if let Some(schema) = resp.schema {
                format_schema(&schema, format);
            } else {
                println!("Schema not found");
            }
        }
        SchemaCommand::List => {
            let mut client = conn.schema_client();
            let resp = client
                .list_schemas(ListSchemasRequest {
                    context: Some(context),
                })
                .await?
                .into_inner();

            format_schema_list(&resp.schemas, format);
        }
        SchemaCommand::Diff {
            entity_type,
            v1,
            v2,
        } => {
            let mut client = conn.schema_client();
            let left = client
                .get_schema(GetSchemaRequest {
                    context: Some(context.clone()),
                    entity_type: entity_type.clone(),
                    version: v1,
                })
                .await?
                .into_inner();
            let right = client
                .get_schema(GetSchemaRequest {
                    context: Some(context),
                    entity_type: entity_type.clone(),
                    version: v2,
                })
                .await?
                .into_inner();

            match (left.schema, right.schema) {
                (Some(a), Some(b)) => format_schema_diff(&entity_type, &a, &b, format),
                _ => println!(
                    "Could not diff '{}' versions {} and {}: one or both versions were not found.",
                    entity_type, v1, v2
                ),
            }
        }
    }
    Ok(())
}

fn json_to_proto_schema(
    json: &serde_json::Value,
) -> Result<EntitySchema, Box<dyn std::error::Error>> {
    let name = json
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'name' field")?;

    let mut properties = std::collections::HashMap::new();
    if let Some(props) = json.get("properties").and_then(|v| v.as_object()) {
        for (key, val) in props {
            let type_str = val.get("type").and_then(|v| v.as_str()).unwrap_or("string");
            let required = val
                .get("required")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let index_str = val.get("index").and_then(|v| v.as_str()).unwrap_or("none");

            properties.insert(
                key.clone(),
                PropertyDef {
                    r#type: property_type_from_str(type_str) as i32,
                    required,
                    index: index_type_from_str(index_str) as i32,
                    default_value: None,
                },
            );
        }
    }

    let meta = json
        .get("meta")
        .and_then(|value| value.as_object())
        .map(|obj| {
            let allow_undeclared_edges = obj
                .get("allow_undeclared_edges")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let extras_policy = obj
                .get("extras_policy")
                .and_then(|v| v.as_str())
                .map(extras_policy_from_str)
                .unwrap_or(ExtrasPolicy::Unspecified) as i32;
            SchemaMeta {
                allow_undeclared_edges,
                extras_policy,
            }
        });

    Ok(EntitySchema {
        name: name.to_string(),
        version: 0,
        properties,
        edges: std::collections::HashMap::new(),
        meta,
        created_at: 0,
        created_by: String::new(),
    })
}

fn property_type_from_str(s: &str) -> PropertyType {
    match s.to_lowercase().as_str() {
        "string" | "str" => PropertyType::String,
        "int" | "integer" | "i64" => PropertyType::Int,
        "float" | "double" | "f64" => PropertyType::Float,
        "bool" | "boolean" => PropertyType::Bool,
        "timestamp" | "time" => PropertyType::Timestamp,
        "bytes" | "binary" => PropertyType::Bytes,
        _ => PropertyType::Unspecified,
    }
}

fn index_type_from_str(s: &str) -> IndexType {
    match s.to_lowercase().as_str() {
        "unique" => IndexType::Unique,
        "equality" | "eq" => IndexType::Equality,
        "range" => IndexType::Range,
        _ => IndexType::None,
    }
}

fn extras_policy_from_str(s: &str) -> ExtrasPolicy {
    match s.to_lowercase().as_str() {
        "reject" => ExtrasPolicy::Reject,
        "allow" => ExtrasPolicy::Allow,
        "warn" => ExtrasPolicy::Warn,
        _ => ExtrasPolicy::Unspecified,
    }
}

fn property_type_name(val: i32) -> &'static str {
    match PropertyType::try_from(val) {
        Ok(PropertyType::String) => "string",
        Ok(PropertyType::Int) => "int",
        Ok(PropertyType::Float) => "float",
        Ok(PropertyType::Bool) => "bool",
        Ok(PropertyType::Timestamp) => "timestamp",
        Ok(PropertyType::Bytes) => "bytes",
        _ => "unknown",
    }
}

fn index_type_name(val: i32) -> &'static str {
    match IndexType::try_from(val) {
        Ok(IndexType::None) => "none",
        Ok(IndexType::Unique) => "unique",
        Ok(IndexType::Equality) => "equality",
        Ok(IndexType::Range) => "range",
        _ => "none",
    }
}

fn schema_to_json(schema: &EntitySchema) -> serde_json::Value {
    let mut props = serde_json::Map::new();
    for (k, v) in &schema.properties {
        props.insert(
            k.clone(),
            serde_json::json!({
                "type": property_type_name(v.r#type),
                "required": v.required,
                "index": index_type_name(v.index),
            }),
        );
    }
    serde_json::json!({
        "name": schema.name,
        "version": schema.version,
        "properties": props,
        "created_at": schema.created_at,
        "created_by": schema.created_by,
    })
}

fn format_schema(schema: &EntitySchema, format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            crate::output::print_json(&schema_to_json(schema));
        }
        OutputFormat::Table => {
            println!("Entity: {} (v{})", schema.name, schema.version);
            if schema.properties.is_empty() {
                println!("  No properties defined");
            } else {
                let headers = vec!["Property", "Type", "Required", "Index"];
                let mut rows = Vec::new();
                for (name, prop) in &schema.properties {
                    rows.push(vec![
                        name.clone(),
                        property_type_name(prop.r#type).to_string(),
                        prop.required.to_string(),
                        index_type_name(prop.index).to_string(),
                    ]);
                }
                crate::output::print_table(&headers, &rows);
            }
        }
        OutputFormat::Csv => {
            let headers = vec!["Property", "Type", "Required", "Index"];
            let mut rows = Vec::new();
            for (name, prop) in &schema.properties {
                rows.push(vec![
                    name.clone(),
                    property_type_name(prop.r#type).to_string(),
                    prop.required.to_string(),
                    index_type_name(prop.index).to_string(),
                ]);
            }
            crate::output::print_csv(&headers, &rows);
        }
    }
}

fn format_schema_list(schemas: &[EntitySchema], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = schemas.iter().map(schema_to_json).collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Name", "Version", "Properties", "Created"];
            let rows: Vec<Vec<String>> = schemas
                .iter()
                .map(|s| {
                    vec![
                        s.name.clone(),
                        s.version.to_string(),
                        s.properties.len().to_string(),
                        s.created_at.to_string(),
                    ]
                })
                .collect();
            match format {
                OutputFormat::Table => crate::output::print_table(&headers, &rows),
                OutputFormat::Csv => crate::output::print_csv(&headers, &rows),
                _ => unreachable!(),
            }
        }
    }
}

fn format_schema_diff(
    entity_type: &str,
    left: &EntitySchema,
    right: &EntitySchema,
    format: &OutputFormat,
) {
    let mut changes: Vec<(String, String, String, String)> = Vec::new();

    for (name, old_prop) in &left.properties {
        match right.properties.get(name) {
            None => changes.push((
                "removed".to_string(),
                name.clone(),
                format!(
                    "type={}, required={}, index={}",
                    property_type_name(old_prop.r#type),
                    old_prop.required,
                    index_type_name(old_prop.index)
                ),
                String::new(),
            )),
            Some(new_prop) => {
                if old_prop.r#type != new_prop.r#type
                    || old_prop.required != new_prop.required
                    || old_prop.index != new_prop.index
                {
                    changes.push((
                        "changed".to_string(),
                        name.clone(),
                        format!(
                            "type={}, required={}, index={}",
                            property_type_name(old_prop.r#type),
                            old_prop.required,
                            index_type_name(old_prop.index)
                        ),
                        format!(
                            "type={}, required={}, index={}",
                            property_type_name(new_prop.r#type),
                            new_prop.required,
                            index_type_name(new_prop.index)
                        ),
                    ));
                }
            }
        }
    }

    for (name, new_prop) in &right.properties {
        if !left.properties.contains_key(name) {
            changes.push((
                "added".to_string(),
                name.clone(),
                String::new(),
                format!(
                    "type={}, required={}, index={}",
                    property_type_name(new_prop.r#type),
                    new_prop.required,
                    index_type_name(new_prop.index)
                ),
            ));
        }
    }

    match format {
        OutputFormat::Json => {
            let payload = serde_json::json!({
                "entity_type": entity_type,
                "from_version": left.version,
                "to_version": right.version,
                "changes": changes.iter().map(|(kind, field, old, new)| {
                    serde_json::json!({
                        "kind": kind,
                        "field": field,
                        "old": old,
                        "new": new,
                    })
                }).collect::<Vec<_>>(),
            });
            crate::output::print_json(&payload);
        }
        OutputFormat::Table => {
            println!(
                "Schema diff for {} (v{} -> v{})",
                entity_type, left.version, right.version
            );
            if changes.is_empty() {
                println!("No changes.");
                return;
            }
            let headers = vec!["Change", "Field", "Old", "New"];
            let rows: Vec<Vec<String>> = changes
                .into_iter()
                .map(|(kind, field, old, new)| vec![kind, field, old, new])
                .collect();
            crate::output::print_table(&headers, &rows);
        }
        OutputFormat::Csv => {
            let headers = vec!["Change", "Field", "Old", "New"];
            let rows: Vec<Vec<String>> = changes
                .into_iter()
                .map(|(kind, field, old, new)| vec![kind, field, old, new])
                .collect();
            crate::output::print_csv(&headers, &rows);
        }
    }
}
