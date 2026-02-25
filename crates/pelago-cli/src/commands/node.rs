use crate::connection::GrpcConnection;
use crate::output::OutputFormat;
use pelago_proto::{
    CreateNodeRequest, DeleteNodeRequest, FindNodesRequest, GetNodeRequest, Node, RequestContext,
    UpdateNodeRequest, Value,
};

#[derive(clap::Args)]
pub struct NodeArgs {
    #[command(subcommand)]
    pub command: NodeCommand,
}

#[derive(clap::Subcommand)]
pub enum NodeCommand {
    /// Create a new node
    Create {
        /// Entity type
        entity_type: String,
        /// Properties as JSON
        #[arg(long)]
        props: Option<String>,
        /// Properties as key=value pairs
        #[arg(value_name = "key=value")]
        kv: Vec<String>,
    },
    /// Get a node by ID
    Get {
        /// Entity type
        entity_type: String,
        /// Node ID
        node_id: String,
        /// Fields to return
        #[arg(long, value_delimiter = ',')]
        fields: Option<Vec<String>>,
    },
    /// Update a node
    Update {
        /// Entity type
        entity_type: String,
        /// Node ID
        node_id: String,
        /// Properties to update as JSON
        #[arg(long)]
        props: String,
    },
    /// Delete a node
    Delete {
        /// Entity type
        entity_type: String,
        /// Node ID
        node_id: String,
        /// Skip confirmation
        #[arg(long)]
        force: bool,
    },
    /// List nodes of a type
    List {
        /// Entity type
        entity_type: String,
        /// Maximum results
        #[arg(long, default_value = "100")]
        limit: u32,
        /// CEL filter expression
        #[arg(long)]
        filter: Option<String>,
    },
}

pub async fn run(
    args: NodeArgs,
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
        NodeCommand::Create {
            entity_type,
            props,
            kv,
        } => {
            let properties = if let Some(ref p) = props {
                parse_properties(p)?
            } else if !kv.is_empty() {
                parse_key_value_properties(&kv)?
            } else {
                std::collections::HashMap::new()
            };
            let mut client = conn.node_client();
            let resp = client
                .create_node(CreateNodeRequest {
                    context: Some(context),
                    entity_type,
                    properties,
                })
                .await?
                .into_inner();

            if let Some(node) = resp.node {
                format_node(&node, format);
            } else {
                println!("Node created (no details returned)");
            }
        }
        NodeCommand::Get {
            entity_type,
            node_id,
            fields,
        } => {
            let mut client = conn.node_client();
            let resp = client
                .get_node(GetNodeRequest {
                    context: Some(context),
                    entity_type,
                    node_id,
                    consistency: 0,
                    fields: fields.unwrap_or_default(),
                })
                .await?
                .into_inner();

            if let Some(node) = resp.node {
                format_node(&node, format);
            } else {
                println!("Node not found");
            }
        }
        NodeCommand::Update {
            entity_type,
            node_id,
            props,
        } => {
            let properties = parse_properties(&props)?;
            let mut client = conn.node_client();
            let resp = client
                .update_node(UpdateNodeRequest {
                    context: Some(context),
                    entity_type,
                    node_id,
                    properties,
                })
                .await?
                .into_inner();

            if let Some(node) = resp.node {
                format_node(&node, format);
            } else {
                println!("Node updated (no details returned)");
            }
        }
        NodeCommand::Delete {
            entity_type,
            node_id,
            force,
        } => {
            if !force {
                eprintln!("Deleting {} node {}...", entity_type, node_id);
            }
            let mut client = conn.node_client();
            let resp = client
                .delete_node(DeleteNodeRequest {
                    context: Some(context),
                    entity_type,
                    node_id,
                    mutation_mode: pelago_proto::MutationExecutionMode::AsyncAllowed as i32,
                })
                .await?
                .into_inner();

            match format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "deleted": resp.deleted,
                        "cleanup_job_id": resp.cleanup_job_id,
                    }));
                }
                _ => {
                    println!("Deleted: {}", resp.deleted);
                    if !resp.cleanup_job_id.is_empty() {
                        println!("Cleanup job: {}", resp.cleanup_job_id);
                    }
                }
            }
        }
        NodeCommand::List {
            entity_type,
            limit,
            filter,
        } => {
            let mut client = conn.query_client();
            let mut stream = client
                .find_nodes(FindNodesRequest {
                    context: Some(context),
                    entity_type,
                    cel_expression: filter.unwrap_or_default(),
                    consistency: 0,
                    fields: vec![],
                    limit,
                    cursor: vec![],
                    snapshot_mode: pelago_proto::SnapshotMode::Strict as i32,
                    allow_degrade_to_best_effort: false,
                })
                .await?
                .into_inner();

            let mut nodes = Vec::new();
            while let Some(result) = stream.message().await? {
                if let Some(node) = result.node {
                    nodes.push(node);
                }
            }

            format_node_list(&nodes, format);
        }
    }
    Ok(())
}

fn parse_properties(
    json_str: &str,
) -> Result<std::collections::HashMap<String, Value>, Box<dyn std::error::Error>> {
    let json: serde_json::Value = serde_json::from_str(json_str)?;
    let mut props = std::collections::HashMap::new();
    if let Some(obj) = json.as_object() {
        for (key, val) in obj {
            props.insert(key.clone(), json_to_proto_value(val));
        }
    }
    Ok(props)
}

fn parse_key_value_properties(
    pairs: &[String],
) -> Result<std::collections::HashMap<String, Value>, Box<dyn std::error::Error>> {
    let mut props = std::collections::HashMap::new();
    for pair in pairs {
        let (key, raw) = pair
            .split_once('=')
            .ok_or_else(|| format!("Invalid property '{}', expected key=value", pair))?;
        let value = if raw.eq_ignore_ascii_case("true") {
            Value {
                kind: Some(pelago_proto::value::Kind::BoolValue(true)),
            }
        } else if raw.eq_ignore_ascii_case("false") {
            Value {
                kind: Some(pelago_proto::value::Kind::BoolValue(false)),
            }
        } else if let Ok(i) = raw.parse::<i64>() {
            Value {
                kind: Some(pelago_proto::value::Kind::IntValue(i)),
            }
        } else if let Ok(f) = raw.parse::<f64>() {
            Value {
                kind: Some(pelago_proto::value::Kind::FloatValue(f)),
            }
        } else {
            Value {
                kind: Some(pelago_proto::value::Kind::StringValue(raw.to_string())),
            }
        };
        props.insert(key.to_string(), value);
    }
    Ok(props)
}

fn json_to_proto_value(val: &serde_json::Value) -> Value {
    use pelago_proto::value::Kind;
    Value {
        kind: Some(match val {
            serde_json::Value::String(s) => Kind::StringValue(s.clone()),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Kind::IntValue(i)
                } else {
                    Kind::FloatValue(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::Bool(b) => Kind::BoolValue(*b),
            serde_json::Value::Null => Kind::NullValue(true),
            _ => Kind::StringValue(val.to_string()),
        }),
    }
}

fn proto_value_to_json(val: &Value) -> serde_json::Value {
    use pelago_proto::value::Kind;
    match &val.kind {
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::IntValue(n)) => serde_json::json!(n),
        Some(Kind::FloatValue(f)) => serde_json::json!(f),
        Some(Kind::BoolValue(b)) => serde_json::json!(b),
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::TimestampValue(t)) => serde_json::json!(t),
        Some(Kind::BytesValue(b)) => serde_json::json!(format!("<{} bytes>", b.len())),
        None => serde_json::Value::Null,
    }
}

fn node_to_json(node: &Node) -> serde_json::Value {
    let mut props = serde_json::Map::new();
    for (k, v) in &node.properties {
        props.insert(k.clone(), proto_value_to_json(v));
    }
    serde_json::json!({
        "id": node.id,
        "entity_type": node.entity_type,
        "properties": props,
        "created_at": node.created_at,
        "updated_at": node.updated_at,
    })
}

fn format_props(properties: &std::collections::HashMap<String, Value>) -> String {
    let parts: Vec<String> = properties
        .iter()
        .map(|(k, v)| {
            let val_str = match &v.kind {
                Some(pelago_proto::value::Kind::StringValue(s)) => format!("\"{}\"", s),
                Some(pelago_proto::value::Kind::IntValue(n)) => n.to_string(),
                Some(pelago_proto::value::Kind::FloatValue(f)) => f.to_string(),
                Some(pelago_proto::value::Kind::BoolValue(b)) => b.to_string(),
                Some(pelago_proto::value::Kind::NullValue(_)) => "null".to_string(),
                Some(pelago_proto::value::Kind::TimestampValue(t)) => t.to_string(),
                Some(pelago_proto::value::Kind::BytesValue(b)) => {
                    format!("<{} bytes>", b.len())
                }
                None => "null".to_string(),
            };
            format!("{}={}", k, val_str)
        })
        .collect();
    parts.join(", ")
}

fn format_node(node: &Node, format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            crate::output::print_json(&node_to_json(node));
        }
        OutputFormat::Table => {
            let headers = vec!["ID", "Type", "Properties"];
            let props_str = format_props(&node.properties);
            let rows = vec![vec![node.id.clone(), node.entity_type.clone(), props_str]];
            crate::output::print_table(&headers, &rows);
        }
        OutputFormat::Csv => {
            let headers = vec!["ID", "Type", "Properties"];
            let props_str = format_props(&node.properties);
            let rows = vec![vec![node.id.clone(), node.entity_type.clone(), props_str]];
            crate::output::print_csv(&headers, &rows);
        }
    }
}

fn format_node_list(nodes: &[Node], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = nodes.iter().map(node_to_json).collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["ID", "Type", "Properties"];
            let rows: Vec<Vec<String>> = nodes
                .iter()
                .map(|n| {
                    vec![
                        n.id.clone(),
                        n.entity_type.clone(),
                        format_props(&n.properties),
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
