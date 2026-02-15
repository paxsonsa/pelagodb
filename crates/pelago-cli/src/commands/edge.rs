use crate::connection::GrpcConnection;
use crate::output::OutputFormat;
use pelago_proto::{
    CreateEdgeRequest, DeleteEdgeRequest, Edge, EdgeDirection, ListEdgesRequest, NodeRef,
    RequestContext, Value,
};

#[derive(clap::Args)]
pub struct EdgeArgs {
    #[command(subcommand)]
    pub command: EdgeCommand,
}

#[derive(clap::Subcommand)]
pub enum EdgeCommand {
    /// Create an edge between nodes
    Create {
        /// Source node (Type:id)
        source: String,
        /// Edge label
        label: String,
        /// Target node (Type:id)
        target: String,
        /// Edge properties as JSON
        #[arg(long)]
        props: Option<String>,
    },
    /// Delete an edge
    Delete {
        /// Source node (Type:id)
        source: String,
        /// Edge label
        label: String,
        /// Target node (Type:id)
        target: String,
    },
    /// List edges for a node
    List {
        /// Entity type
        entity_type: String,
        /// Node ID
        node_id: String,
        /// Direction filter
        #[arg(long, default_value = "out")]
        dir: String,
        /// Edge label filter
        #[arg(long)]
        label: Option<String>,
    },
}

pub async fn run(
    args: EdgeArgs,
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
        EdgeCommand::Create {
            source,
            label,
            target,
            props,
        } => {
            let source_ref = parse_node_ref(&source)?;
            let target_ref = parse_node_ref(&target)?;
            let properties = if let Some(ref p) = props {
                parse_properties(p)?
            } else {
                std::collections::HashMap::new()
            };

            let mut client = conn.edge_client();
            let resp = client
                .create_edge(CreateEdgeRequest {
                    context: Some(context),
                    source: Some(source_ref),
                    target: Some(target_ref),
                    label,
                    properties,
                })
                .await?
                .into_inner();

            if let Some(edge) = resp.edge {
                format_edge(&edge, format);
            } else {
                println!("Edge created");
            }
        }
        EdgeCommand::Delete {
            source,
            label,
            target,
        } => {
            let source_ref = parse_node_ref(&source)?;
            let target_ref = parse_node_ref(&target)?;

            let mut client = conn.edge_client();
            let resp = client
                .delete_edge(DeleteEdgeRequest {
                    context: Some(context),
                    source: Some(source_ref),
                    target: Some(target_ref),
                    label,
                })
                .await?
                .into_inner();

            match format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({ "deleted": resp.deleted }));
                }
                _ => {
                    println!("Deleted: {}", resp.deleted);
                }
            }
        }
        EdgeCommand::List {
            entity_type,
            node_id,
            dir,
            label,
        } => {
            let direction = match dir.to_lowercase().as_str() {
                "out" | "outgoing" => EdgeDirection::Outgoing as i32,
                "in" | "incoming" => EdgeDirection::Incoming as i32,
                "both" => EdgeDirection::Both as i32,
                _ => EdgeDirection::Outgoing as i32,
            };

            let mut client = conn.edge_client();
            let mut stream = client
                .list_edges(ListEdgesRequest {
                    context: Some(context),
                    entity_type,
                    node_id,
                    label: label.unwrap_or_default(),
                    direction,
                    consistency: 0,
                    limit: 0,
                    cursor: vec![],
                })
                .await?
                .into_inner();

            let mut edges = Vec::new();
            while let Some(result) = stream.message().await? {
                if let Some(edge) = result.edge {
                    edges.push(edge);
                }
            }

            format_edge_list(&edges, format);
        }
    }
    Ok(())
}

fn parse_node_ref(s: &str) -> Result<NodeRef, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(
            format!("Invalid node reference '{}'. Expected format: Type:id", s).into(),
        );
    }
    Ok(NodeRef {
        entity_type: parts[0].to_string(),
        node_id: parts[1].to_string(),
        database: String::new(),
        namespace: String::new(),
    })
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

fn node_ref_str(nr: &Option<NodeRef>) -> String {
    match nr {
        Some(r) => format!("{}:{}", r.entity_type, r.node_id),
        None => "<unknown>".to_string(),
    }
}

fn edge_to_json(edge: &Edge) -> serde_json::Value {
    let mut props = serde_json::Map::new();
    for (k, v) in &edge.properties {
        props.insert(k.clone(), proto_value_to_json(v));
    }
    serde_json::json!({
        "edge_id": edge.edge_id,
        "source": node_ref_str(&edge.source),
        "target": node_ref_str(&edge.target),
        "label": edge.label,
        "properties": props,
        "created_at": edge.created_at,
    })
}

fn format_edge(edge: &Edge, format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            crate::output::print_json(&edge_to_json(edge));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Edge ID", "Source", "Label", "Target"];
            let rows = vec![vec![
                edge.edge_id.clone(),
                node_ref_str(&edge.source),
                edge.label.clone(),
                node_ref_str(&edge.target),
            ]];
            match format {
                OutputFormat::Table => crate::output::print_table(&headers, &rows),
                OutputFormat::Csv => crate::output::print_csv(&headers, &rows),
                _ => unreachable!(),
            }
        }
    }
}

fn format_edge_list(edges: &[Edge], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = edges.iter().map(edge_to_json).collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Edge ID", "Source", "Label", "Target"];
            let rows: Vec<Vec<String>> = edges
                .iter()
                .map(|e| {
                    vec![
                        e.edge_id.clone(),
                        node_ref_str(&e.source),
                        e.label.clone(),
                        node_ref_str(&e.target),
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
