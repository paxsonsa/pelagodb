use crate::connection::GrpcConnection;
use crate::output::OutputFormat;
use pelago_proto::{
    ExecutePqlRequest, FindNodesRequest, Node, NodeRef, ReadConsistency, RequestContext,
    TraversalStep, TraverseRequest,
};

#[derive(clap::Args)]
pub struct QueryArgs {
    #[command(subcommand)]
    pub command: QueryCommand,
}

#[derive(clap::Subcommand)]
pub enum QueryCommand {
    /// Find nodes by CEL filter
    Find {
        entity_type: String,
        #[arg(long, default_value = "")]
        filter: String,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    /// Traverse one hop from a start node
    Traverse {
        /// Start ref in Type:node_id format
        start: String,
        /// Edge label
        edge_type: String,
        #[arg(long, default_value_t = 1)]
        max_depth: u32,
        #[arg(long, default_value_t = 100)]
        max_results: u32,
    },
    /// Execute PQL query text
    Pql {
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long, default_value_t = false)]
        explain: bool,
    },
}

pub async fn run(
    args: QueryArgs,
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
        QueryCommand::Find {
            entity_type,
            filter,
            limit,
        } => {
            let mut client = conn.query_client();
            let mut stream = client
                .find_nodes(FindNodesRequest {
                    context: Some(context),
                    entity_type,
                    cel_expression: filter,
                    consistency: ReadConsistency::Strong as i32,
                    fields: vec![],
                    limit,
                    cursor: vec![],
                })
                .await?
                .into_inner();

            let mut nodes = Vec::new();
            while let Some(item) = stream.message().await? {
                if let Some(node) = item.node {
                    nodes.push(node);
                }
            }
            format_nodes(&nodes, format);
        }
        QueryCommand::Traverse {
            start,
            edge_type,
            max_depth,
            max_results,
        } => {
            let (entity_type, node_id) = parse_node_ref(&start)?;
            let mut client = conn.query_client();
            let mut stream = client
                .traverse(TraverseRequest {
                    context: Some(context),
                    start: Some(NodeRef {
                        entity_type,
                        node_id,
                        database: String::new(),
                        namespace: String::new(),
                    }),
                    steps: vec![TraversalStep {
                        edge_type,
                        direction: pelago_proto::EdgeDirection::Outgoing as i32,
                        edge_filter: String::new(),
                        node_filter: String::new(),
                        fields: vec![],
                        per_node_limit: 0,
                        edge_fields: vec![],
                        sort: None,
                    }],
                    max_depth,
                    timeout_ms: 5000,
                    max_results,
                    consistency: ReadConsistency::Strong as i32,
                    cascade: false,
                    cursor: vec![],
                })
                .await?
                .into_inner();

            let mut nodes = Vec::new();
            while let Some(item) = stream.message().await? {
                if let Some(node) = item.node {
                    nodes.push(node);
                }
            }
            format_nodes(&nodes, format);
        }
        QueryCommand::Pql {
            query,
            file,
            explain,
        } => {
            let pql = match (query, file) {
                (Some(q), None) => q,
                (None, Some(path)) => std::fs::read_to_string(path)?,
                (Some(_), Some(_)) => {
                    return Err("provide either --query or --file, not both".into())
                }
                (None, None) => return Err("missing --query or --file".into()),
            };

            let mut client = conn.query_client();
            let mut stream = client
                .execute_pql(ExecutePqlRequest {
                    context: Some(context),
                    pql,
                    params: std::collections::HashMap::new(),
                    explain,
                })
                .await?
                .into_inner();

            let mut nodes = Vec::new();
            let mut explains = Vec::new();
            while let Some(item) = stream.message().await? {
                if !item.explain.is_empty() {
                    explains.push(item.explain);
                }
                if let Some(node) = item.node {
                    nodes.push(node);
                }
            }

            if !explains.is_empty() {
                for e in explains {
                    println!("{}", e);
                }
            }
            if !nodes.is_empty() {
                format_nodes(&nodes, format);
            }
        }
    }

    Ok(())
}

fn parse_node_ref(input: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    let (entity_type, node_id) = input
        .split_once(':')
        .ok_or("expected node ref format Type:node_id")?;
    Ok((entity_type.to_string(), node_id.to_string()))
}

fn format_nodes(nodes: &[Node], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<_> = nodes
                .iter()
                .map(|n| {
                    let properties = n
                        .properties
                        .iter()
                        .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
                        .collect::<serde_json::Map<String, serde_json::Value>>();
                    serde_json::json!({
                        "id": n.id,
                        "entity_type": n.entity_type,
                        "locality": n.locality,
                        "properties": properties,
                    })
                })
                .collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["ID", "Type", "Locality", "Properties"];
            let rows: Vec<Vec<String>> = nodes
                .iter()
                .map(|n| {
                    vec![
                        n.id.clone(),
                        n.entity_type.clone(),
                        n.locality.clone(),
                        format!("{:?}", n.properties),
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

fn proto_value_to_json(v: &pelago_proto::Value) -> serde_json::Value {
    use pelago_proto::value::Kind;
    match &v.kind {
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::IntValue(i)) => serde_json::json!(i),
        Some(Kind::FloatValue(f)) => serde_json::json!(f),
        Some(Kind::BoolValue(b)) => serde_json::json!(b),
        Some(Kind::TimestampValue(t)) => serde_json::json!(t),
        Some(Kind::BytesValue(b)) => serde_json::json!(b),
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        None => serde_json::Value::Null,
    }
}
