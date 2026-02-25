use crate::connection::GrpcConnection;
use crate::output::OutputFormat;
use pelago_proto::{
    ExecutePqlRequest, FindNodesRequest, Node, NodeRef, ReadConsistency, RequestContext,
    TraversalStep, TraverseRequest,
};
use std::fmt::Write as _;

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
        /// Hex-encoded cursor for keyset pagination
        #[arg(long, default_value = "")]
        cursor_hex: String,
        /// Print only the response next cursor as hex (for benchmarking/scripts)
        #[arg(long, default_value_t = false)]
        next_cursor_only: bool,
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
            cursor_hex,
            next_cursor_only,
        } => {
            let mut client = conn.query_client();
            let cursor = decode_hex_cursor(&cursor_hex)?;
            let mut stream = client
                .find_nodes(FindNodesRequest {
                    context: Some(context),
                    entity_type,
                    cel_expression: filter,
                    consistency: ReadConsistency::Strong as i32,
                    fields: vec![],
                    limit,
                    cursor,
                    snapshot_mode: pelago_proto::SnapshotMode::Strict as i32,
                    allow_degrade_to_best_effort: false,
                })
                .await?
                .into_inner();

            let mut nodes = Vec::new();
            let mut next_cursor = Vec::new();
            while let Some(item) = stream.message().await? {
                if !item.next_cursor.is_empty() {
                    next_cursor = item.next_cursor.clone();
                }
                if let Some(node) = item.node {
                    nodes.push(node);
                }
            }
            if next_cursor_only {
                println!("{}", encode_hex_cursor(&next_cursor));
            } else {
                format_nodes(&nodes, format);
            }
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
                    snapshot_mode: pelago_proto::SnapshotMode::BestEffort as i32,
                    allow_degrade_to_best_effort: true,
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
                    snapshot_mode: pelago_proto::SnapshotMode::BestEffort as i32,
                    allow_degrade_to_best_effort: false,
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

fn decode_hex_cursor(input: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let raw = input.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    if raw.len() % 2 != 0 {
        return Err("cursor hex must have even length".into());
    }

    let mut out = Vec::with_capacity(raw.len() / 2);
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = decode_hex_nibble(bytes[i])?;
        let lo = decode_hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn encode_hex_cursor(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for byte in input {
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

fn decode_hex_nibble(ch: u8) -> Result<u8, Box<dyn std::error::Error>> {
    match ch {
        b'0'..=b'9' => Ok(ch - b'0'),
        b'a'..=b'f' => Ok(ch - b'a' + 10),
        b'A'..=b'F' => Ok(ch - b'A' + 10),
        _ => Err("cursor hex contains invalid character".into()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_hex_roundtrip() {
        let raw = vec![0x01, 0x02, 0xab, 0xcd, 0x00];
        let encoded = encode_hex_cursor(&raw);
        let decoded = decode_hex_cursor(&encoded).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn test_decode_hex_cursor_empty() {
        assert_eq!(decode_hex_cursor("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_decode_hex_cursor_invalid_len() {
        assert!(decode_hex_cursor("abc").is_err());
    }
}
