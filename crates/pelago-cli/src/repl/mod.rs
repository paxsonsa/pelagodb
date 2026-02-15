use crate::connection::GrpcConnection;
use crate::output::OutputFormat;
use pelago_proto::{
    FindNodesRequest, GetNodeRequest, ListSchemasRequest, Node, NodeRef, RequestContext, SortSpec,
    TraversalStep, TraverseRequest, TraverseResult, Value,
};
use pelago_query::pql::{
    explain_query, parse_pql, CompiledBlock, InMemorySchemaProvider, PqlCompiler, PqlEdgeDirection,
    PqlResolver, SchemaInfo,
};
use rustyline::DefaultEditor;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

#[derive(clap::Args)]
pub struct ReplArgs {
    /// Initial namespace
    #[arg(long)]
    pub namespace: Option<String>,
}

pub struct ReplContext {
    pub database: String,
    pub namespace: String,
    pub format: OutputFormat,
    pub params: HashMap<String, String>,
}

struct MetaCommandOutcome {
    exit: bool,
    refresh_schemas: bool,
    explain_query: Option<String>,
}

pub async fn run(
    args: ReplArgs,
    server: &str,
    database: &str,
    namespace: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ns = args.namespace.as_deref().unwrap_or(namespace);

    let mut context = ReplContext {
        database: database.to_string(),
        namespace: ns.to_string(),
        format: OutputFormat::Table,
        params: HashMap::new(),
    };

    // Connect to server
    let conn = match GrpcConnection::connect(server).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Warning: Could not connect to server at {}: {}",
                server, e
            );
            eprintln!("REPL will start in offline mode (parse/explain only).\n");
            return run_offline_repl(&mut context).await;
        }
    };

    // Print banner
    println!("PelagoDB PQL REPL v{}", env!("CARGO_PKG_VERSION"));
    println!("Connected to {} (db: {}, ns: {})", server, database, ns);
    println!("Type :help for commands, :quit to exit.\n");

    // Setup rustyline
    let history_path = get_history_path();
    let mut rl = DefaultEditor::new()?;
    let mut session_history: Vec<String> = Vec::new();
    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }

    // Fetch schemas for resolution (cache them)
    let mut schemas = fetch_schemas(&conn, &context)
        .await
        .unwrap_or_default();

    loop {
        let prompt = format!("{}:{}> ", context.database, context.namespace);
        let line = match rl.readline(&prompt) {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let _ = rl.add_history_entry(trimmed);
        session_history.push(trimmed.to_string());

        // Handle meta-commands
        if trimmed.starts_with(':') {
            let outcome = handle_meta_command(trimmed, &mut context, &session_history);
            if let Some(explain_query) = outcome.explain_query {
                execute_explain_input(&explain_query, &context, &schemas);
            }
            if outcome.refresh_schemas {
                schemas = fetch_schemas(&conn, &context).await.unwrap_or_default();
            }
            if outcome.exit {
                break; // :quit
            }
            continue;
        }

        // Handle multiline input
        let input = if needs_continuation(trimmed) {
            collect_multiline(&mut rl, trimmed)?
        } else {
            trimmed.to_string()
        };

        // Execute PQL
        execute_pql_input(&apply_params(&input, &context.params), &conn, &context, &schemas).await;
    }

    // Save history
    if let Some(ref path) = history_path {
        let _ = rl.save_history(path);
    }

    println!("Goodbye!");
    Ok(())
}

async fn run_offline_repl(context: &mut ReplContext) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "PelagoDB PQL REPL v{} (offline mode)",
        env!("CARGO_PKG_VERSION")
    );
    println!("Type :help for commands, :quit to exit.\n");

    let history_path = get_history_path();
    let mut rl = DefaultEditor::new()?;
    let mut session_history: Vec<String> = Vec::new();
    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }
    let schemas = InMemorySchemaProvider::new();

    loop {
        let prompt = format!("{}:{}> ", context.database, context.namespace);
        let line = match rl.readline(&prompt) {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(trimmed);
        session_history.push(trimmed.to_string());

        if trimmed.starts_with(':') {
            let outcome = handle_meta_command(trimmed, context, &session_history);
            if let Some(explain_query) = outcome.explain_query {
                execute_explain_input(&explain_query, context, &schemas);
            }
            if outcome.exit {
                break;
            }
            continue;
        }

        // In offline mode, we can only parse and explain
        let input = if needs_continuation(trimmed) {
            collect_multiline(&mut rl, trimmed)?
        } else {
            trimmed.to_string()
        };

        match parse_pql(&apply_params(&input, &context.params)) {
            Ok(ast) => {
                println!("Parsed successfully ({} block(s))", ast.blocks.len());
                // Try to compile/explain without schemas
                let resolver = PqlResolver::new();
                match resolver.resolve(&ast, &schemas) {
                    Ok(resolved) => {
                        let compiler = PqlCompiler::new();
                        match compiler.compile(&resolved) {
                            Ok(blocks) => {
                                println!("{}", explain_query(&blocks));
                            }
                            Err(e) => println!("Compile: {}", e),
                        }
                    }
                    Err(e) => println!("(Schema validation skipped in offline mode: {})", e),
                }
            }
            Err(e) => eprintln!("Parse error: {}", e),
        }
    }

    if let Some(ref path) = history_path {
        let _ = rl.save_history(path);
    }

    println!("Goodbye!");
    Ok(())
}

/// Handle a meta-command.
fn handle_meta_command(
    cmd: &str,
    context: &mut ReplContext,
    history: &[String],
) -> MetaCommandOutcome {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
    let command = parts[0];
    let arg = parts.get(1).map(|s| s.trim());
    let mut out = MetaCommandOutcome {
        exit: false,
        refresh_schemas: false,
        explain_query: None,
    };

    match command {
        ":quit" | ":q" | ":exit" => out.exit = true,
        ":help" | ":h" => print_help(),
        ":use" => {
            if let Some(ns) = arg {
                context.namespace = ns.to_string();
                out.refresh_schemas = true;
                println!("Switched to namespace: {}", ns);
            } else {
                println!("Current namespace: {}", context.namespace);
                println!("Usage: :use <namespace>");
            }
        }
        ":db" => {
            if let Some(db) = arg {
                context.database = db.to_string();
                out.refresh_schemas = true;
                println!("Switched to database: {}", db);
            } else {
                println!("Current database: {}", context.database);
                println!("Usage: :db <database>");
            }
        }
        ":format" => {
            if let Some(fmt) = arg {
                match fmt {
                    "json" => {
                        context.format = OutputFormat::Json;
                        println!("Output format: json");
                    }
                    "table" => {
                        context.format = OutputFormat::Table;
                        println!("Output format: table");
                    }
                    "csv" => {
                        context.format = OutputFormat::Csv;
                        println!("Output format: csv");
                    }
                    _ => println!("Unknown format: {}. Use: json, table, csv", fmt),
                }
            } else {
                println!("Current format: {}", context.format);
                println!("Usage: :format <json|table|csv>");
            }
        }
        ":param" => {
            if let Some(assignment) = arg {
                if let Some((name, value)) = assignment.split_once('=') {
                    let name = name.trim().trim_start_matches('$');
                    let value = value.trim();
                    context
                        .params
                        .insert(name.to_string(), value.to_string());
                    println!("Set ${} = {}", name, value);
                } else {
                    println!("Usage: :param $name = value");
                }
            } else {
                println!("Usage: :param $name = value");
            }
        }
        ":params" => {
            if context.params.is_empty() {
                println!("No parameters set.");
            } else {
                println!("Parameters:");
                for (k, v) in &context.params {
                    println!("  ${} = {}", k, v);
                }
            }
        }
        ":clear" => {
            context.params.clear();
            println!("Parameters cleared.");
        }
        ":explain" => {
            if let Some(pql) = arg {
                out.explain_query = Some(apply_params(pql, &context.params));
            } else {
                println!("Usage: :explain <PQL query>");
            }
        }
        ":history" => {
            if history.is_empty() {
                println!("No history entries.");
            } else {
                for (idx, item) in history.iter().enumerate() {
                    println!("{:>4}: {}", idx + 1, item);
                }
            }
        }
        _ => println!(
            "Unknown command: {}. Type :help for available commands.",
            command
        ),
    }

    out
}

fn print_help() {
    println!("PQL REPL Commands:");
    println!("  :explain <PQL>   Show query execution plan");
    println!("  :use <namespace> Switch namespace");
    println!("  :db <database>   Switch database");
    println!("  :format <fmt>    Set output format (json, table, csv)");
    println!("  :param $n = val  Set a query parameter");
    println!("  :params          List current parameters");
    println!("  :clear           Clear all parameters");
    println!("  :history         Show session history");
    println!("  :help            Show this help");
    println!("  :quit            Exit the REPL");
    println!();
    println!("PQL Examples:");
    println!("  Person {{ name, age }}");
    println!("  Person @filter(age >= 30) {{ name, age }}");
    println!(
        "  query {{ start(func: uid(Person:1_42)) {{ name, -[KNOWS]-> {{ name }} }} }}"
    );
}

/// Check if input needs continuation (unbalanced braces)
fn needs_continuation(input: &str) -> bool {
    let mut depth = 0i32;
    for ch in input.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

/// Collect multiline input until braces are balanced
fn collect_multiline(
    rl: &mut DefaultEditor,
    first_line: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut input = first_line.to_string();
    loop {
        if !needs_continuation(&input) {
            break;
        }
        let continuation = rl.readline("  ... ")?;
        input.push('\n');
        input.push_str(&continuation);
    }
    Ok(input)
}

fn apply_params(input: &str, params: &HashMap<String, String>) -> String {
    if params.is_empty() {
        return input.to_string();
    }

    let mut replaced = input.to_string();
    let mut keys: Vec<&str> = params.keys().map(|k| k.as_str()).collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
    for key in keys {
        if let Some(value) = params.get(key) {
            replaced = replaced.replace(&format!("${}", key), value);
        }
    }
    replaced
}

fn execute_explain_input(
    input: &str,
    context: &ReplContext,
    schemas: &InMemorySchemaProvider,
) {
    let ast = match parse_pql(input) {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            return;
        }
    };

    let resolver = PqlResolver::new();
    let resolved = match resolver.resolve(&ast, schemas) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "Resolution error (db: {}, ns: {}): {}",
                context.database, context.namespace, e
            );
            return;
        }
    };

    let compiler = PqlCompiler::new();
    match compiler.compile(&resolved) {
        Ok(blocks) => println!("{}", explain_query(&blocks)),
        Err(e) => eprintln!("Compile error: {}", e),
    }
}

/// Execute a PQL query against the server
async fn execute_pql_input(
    input: &str,
    conn: &GrpcConnection,
    context: &ReplContext,
    schemas: &InMemorySchemaProvider,
) {
    let start = Instant::now();

    // 1. Parse
    let ast = match parse_pql(input) {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            return;
        }
    };

    // 2. Resolve
    let resolver = PqlResolver::new();
    let resolved = match resolver.resolve(&ast, schemas) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Resolution error: {}", e);
            return;
        }
    };

    // 3. Compile
    let compiler = PqlCompiler::new();
    let blocks = match compiler.compile(&resolved) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Compilation error: {}", e);
            return;
        }
    };

    // Build capture aliases by block name.
    let mut capture_by_block: HashMap<String, String> = HashMap::new();
    for idx in &resolved.execution_order {
        let rb = &resolved.blocks[*idx];
        if let Some(capture_as) = &rb.block.capture_as {
            capture_by_block.insert(rb.name.clone(), capture_as.clone());
        }
    }

    // 4. Execute each block
    let request_context = RequestContext {
        database: context.database.clone(),
        namespace: context.namespace.clone(),
        site_id: String::new(),
        request_id: String::new(),
    };

    let mut total_results = 0usize;
    let mut captured_variables: HashMap<String, Vec<NodeRef>> = HashMap::new();

    for block in &blocks {
        match block {
            CompiledBlock::PointLookup {
                block_name,
                entity_type,
                node_id,
                fields,
            } => {
                let mut client = conn.node_client();
                match client
                    .get_node(GetNodeRequest {
                        context: Some(request_context.clone()),
                        entity_type: entity_type.clone(),
                        node_id: node_id.clone(),
                        consistency: 0,
                        fields: fields.clone(),
                    })
                    .await
                {
                    Ok(resp) => {
                        let resp = resp.into_inner();
                        if let Some(node) = resp.node {
                            total_results += 1;
                            format_result_node(&node, block_name, &context.format);
                            if let Some(var_name) = capture_by_block.get(block_name) {
                                store_capture(
                                    var_name,
                                    std::slice::from_ref(&node),
                                    context,
                                    &mut captured_variables,
                                );
                            }
                        } else {
                            println!("No results for block '{}'", block_name);
                        }
                    }
                    Err(e) => eprintln!("Error in '{}': {}", block_name, e),
                }
            }
            CompiledBlock::FindNodes {
                block_name,
                entity_type,
                cel_expression,
                fields,
                limit,
                ..
            } => {
                let mut client = conn.query_client();
                match client
                    .find_nodes(FindNodesRequest {
                        context: Some(request_context.clone()),
                        entity_type: entity_type.clone(),
                        cel_expression: cel_expression.clone().unwrap_or_default(),
                        consistency: 0,
                        fields: fields.clone(),
                        limit: limit.unwrap_or(100),
                        cursor: Vec::new(),
                    })
                    .await
                {
                    Ok(resp) => {
                        let mut stream = resp.into_inner();
                        let mut nodes = Vec::new();
                        while let Ok(Some(result)) = stream.message().await {
                            if let Some(node) = result.node {
                                nodes.push(node);
                            }
                        }
                        total_results += nodes.len();
                        format_result_nodes(&nodes, block_name, &context.format);
                        if let Some(var_name) = capture_by_block.get(block_name) {
                            store_capture(var_name, &nodes, context, &mut captured_variables);
                        }
                    }
                    Err(e) => eprintln!("Error in '{}': {}", block_name, e),
                }
            }
            CompiledBlock::Traverse {
                block_name,
                start_entity_type,
                start_node_id,
                steps,
                max_depth,
                cascade,
                max_results,
            } => {
                let mut client = conn.query_client();

                let proto_steps: Vec<TraversalStep> = steps
                    .iter()
                    .map(|s| TraversalStep {
                        edge_type: s.edge_type.clone(),
                        direction: match s.direction {
                            PqlEdgeDirection::Outgoing => 1, // EDGE_DIRECTION_OUTGOING
                            PqlEdgeDirection::Incoming => 2, // EDGE_DIRECTION_INCOMING
                            PqlEdgeDirection::Both => 3,     // EDGE_DIRECTION_BOTH
                        },
                        edge_filter: s.edge_filter.clone().unwrap_or_default(),
                        node_filter: s.node_filter.clone().unwrap_or_default(),
                        fields: s.fields.clone(),
                        per_node_limit: s.per_node_limit.unwrap_or(0),
                        edge_fields: s.edge_fields.clone(),
                        sort: s.sort.as_ref().map(|sort| SortSpec {
                            field: sort.field.clone(),
                            descending: sort.descending,
                            on_edge: sort.on_edge,
                        }),
                    })
                    .collect();

                match client
                    .traverse(TraverseRequest {
                        context: Some(request_context.clone()),
                        start: Some(NodeRef {
                            entity_type: start_entity_type.clone(),
                            node_id: start_node_id.clone(),
                            database: String::new(),
                            namespace: String::new(),
                        }),
                        steps: proto_steps,
                        max_depth: *max_depth,
                        timeout_ms: 5000,
                        max_results: *max_results,
                        consistency: 0,
                        cascade: *cascade,
                        cursor: Vec::new(),
                    })
                    .await
                {
                    Ok(resp) => {
                        let mut stream = resp.into_inner();
                        let mut results = Vec::new();
                        while let Ok(Some(result)) = stream.message().await {
                            results.push(result);
                        }
                        let block_nodes: Vec<Node> = results
                            .iter()
                            .filter_map(|r| r.node.clone())
                            .collect();
                        total_results += block_nodes.len();
                        format_traverse_results(&results, block_name, &context.format);
                        if let Some(var_name) = capture_by_block.get(block_name) {
                            store_capture(
                                var_name,
                                &block_nodes,
                                context,
                                &mut captured_variables,
                            );
                        }
                    }
                    Err(e) => eprintln!("Error in '{}': {}", block_name, e),
                }
            }
            CompiledBlock::VariableRef {
                block_name,
                variable,
                filter,
                fields,
            } => {
                let refs = match captured_variables.get(variable) {
                    Some(v) => v.clone(),
                    None => {
                        eprintln!(
                            "Error in '{}': variable ${} has no captured results",
                            block_name, variable
                        );
                        continue;
                    }
                };

                let mut nodes = Vec::new();
                for node_ref in refs {
                    let mut client = conn.node_client();
                    match client
                        .get_node(GetNodeRequest {
                            context: Some(request_context.clone()),
                            entity_type: node_ref.entity_type.clone(),
                            node_id: node_ref.node_id.clone(),
                            consistency: 0,
                            fields: fields.clone(),
                        })
                        .await
                    {
                        Ok(resp) => {
                            if let Some(node) = resp.into_inner().node {
                                let filter_ok = filter
                                    .as_ref()
                                    .map(|expr| node_matches_filter(&node, expr))
                                    .unwrap_or(true);
                                if filter_ok {
                                    nodes.push(node);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error in '{}': {}", block_name, e);
                        }
                    }
                }

                total_results += nodes.len();
                format_result_nodes(&nodes, block_name, &context.format);
                if let Some(var_name) = capture_by_block.get(block_name) {
                    store_capture(var_name, &nodes, context, &mut captured_variables);
                }
            }
        }
    }

    let elapsed = start.elapsed();
    println!(
        "\n{} result(s) ({:.1}ms)",
        total_results,
        elapsed.as_secs_f64() * 1000.0
    );
}

fn store_capture(
    variable: &str,
    nodes: &[Node],
    context: &ReplContext,
    captured_variables: &mut HashMap<String, Vec<NodeRef>>,
) {
    let refs: Vec<NodeRef> = nodes
        .iter()
        .map(|n| NodeRef {
            entity_type: n.entity_type.clone(),
            node_id: n.id.clone(),
            database: context.database.clone(),
            namespace: context.namespace.clone(),
        })
        .collect();
    captured_variables.insert(variable.to_string(), refs);
}

fn node_matches_filter(node: &Node, filter: &str) -> bool {
    use cel_interpreter::{Context, Program, Value as CelValue};

    let program = match Program::compile(filter) {
        Ok(p) => p,
        Err(_) => return false,
    };

    let mut context = Context::default();
    for (k, v) in &node.properties {
        let _ = context.add_variable(k, proto_value_to_cel(v));
    }

    match program.execute(&context) {
        Ok(CelValue::Bool(v)) => v,
        _ => false,
    }
}

fn proto_value_to_cel(value: &Value) -> cel_interpreter::Value {
    use cel_interpreter::Value as CelValue;
    use pelago_proto::value::Kind;
    use std::sync::Arc;

    match &value.kind {
        Some(Kind::StringValue(v)) => CelValue::String(Arc::new(v.clone())),
        Some(Kind::IntValue(v)) => CelValue::Int(*v),
        Some(Kind::FloatValue(v)) => CelValue::Float(*v),
        Some(Kind::BoolValue(v)) => CelValue::Bool(*v),
        Some(Kind::BytesValue(v)) => CelValue::Bytes(Arc::new(v.clone())),
        Some(Kind::NullValue(_)) | None => CelValue::Null,
        _ => CelValue::Null,
    }
}

/// Fetch schemas from server for REPL schema resolution
async fn fetch_schemas(
    conn: &GrpcConnection,
    context: &ReplContext,
) -> Result<InMemorySchemaProvider, Box<dyn std::error::Error>> {
    let mut client = conn.schema_client();
    let resp = client
        .list_schemas(ListSchemasRequest {
            context: Some(RequestContext {
                database: context.database.clone(),
                namespace: context.namespace.clone(),
                site_id: String::new(),
                request_id: String::new(),
            }),
        })
        .await?
        .into_inner();

    let mut provider = InMemorySchemaProvider::new();
    for schema in &resp.schemas {
        let fields: Vec<String> = schema.properties.keys().cloned().collect();
        let edges: Vec<String> = schema.edges.keys().cloned().collect();
        provider.add_schema(SchemaInfo {
            entity_type: schema.name.clone(),
            fields,
            edges,
            allow_undeclared_edges: schema
                .meta
                .as_ref()
                .map(|m| m.allow_undeclared_edges)
                .unwrap_or(false),
        });
    }

    Ok(provider)
}

fn format_result_node(node: &Node, block_name: &str, format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json = node_to_json(node);
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
        _ => {
            println!("Block: {}", block_name);
            println!("  {} {} {{", node.entity_type, node.id);
            for (k, v) in &node.properties {
                println!("    {}: {}", k, format_value(v));
            }
            println!("  }}");
        }
    }
}

fn format_result_nodes(nodes: &[Node], block_name: &str, format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = nodes.iter().map(|n| node_to_json(n)).collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
        OutputFormat::Table => {
            if nodes.is_empty() {
                println!("No results for block '{}'", block_name);
                return;
            }
            // Collect all property keys
            let mut all_keys: Vec<String> = Vec::new();
            for node in nodes {
                for k in node.properties.keys() {
                    if !all_keys.contains(k) {
                        all_keys.push(k.clone());
                    }
                }
            }
            all_keys.sort();

            // Build table
            let mut headers: Vec<&str> = vec!["id", "type"];
            let key_refs: Vec<&str> = all_keys.iter().map(|s| s.as_str()).collect();
            headers.extend(key_refs.iter());

            let rows: Vec<Vec<String>> = nodes
                .iter()
                .map(|n| {
                    let mut row = vec![n.id.clone(), n.entity_type.clone()];
                    for key in &all_keys {
                        let val = n
                            .properties
                            .get(key)
                            .map(|v| format_value(v))
                            .unwrap_or_default();
                        row.push(val);
                    }
                    row
                })
                .collect();

            crate::output::print_table(&headers, &rows);
        }
        OutputFormat::Csv => {
            // Simple CSV
            if let Some(first) = nodes.first() {
                let keys: Vec<&String> = first.properties.keys().collect();
                println!(
                    "id,type,{}",
                    keys.iter()
                        .map(|k| k.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                for n in nodes {
                    let vals: Vec<String> = keys
                        .iter()
                        .map(|k| {
                            n.properties
                                .get(*k)
                                .map(|v| format_value(v))
                                .unwrap_or_default()
                        })
                        .collect();
                    println!("{},{},{}", n.id, n.entity_type, vals.join(","));
                }
            }
        }
    }
}

fn format_traverse_results(
    results: &[TraverseResult],
    block_name: &str,
    format: &OutputFormat,
) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    let mut obj = serde_json::Map::new();
                    obj.insert("depth".to_string(), serde_json::json!(r.depth));
                    if let Some(ref node) = r.node {
                        obj.insert("node".to_string(), node_to_json(node));
                    }
                    serde_json::Value::Object(obj)
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
        _ => {
            println!("Block: {} ({} results)", block_name, results.len());
            for r in results {
                let path_str: Vec<String> = r
                    .path
                    .iter()
                    .map(|nr| format!("{}:{}", nr.entity_type, nr.node_id))
                    .collect();
                if let Some(ref node) = r.node {
                    print!("  [depth={}] ", r.depth);
                    if !path_str.is_empty() {
                        print!("{} -> ", path_str.join(" -> "));
                    }
                    print!("{}:{}", node.entity_type, node.id);
                    if !node.properties.is_empty() {
                        let props: Vec<String> = node
                            .properties
                            .iter()
                            .map(|(k, v)| format!("{}={}", k, format_value(v)))
                            .collect();
                        print!(" {{ {} }}", props.join(", "));
                    }
                    println!();
                }
            }
        }
    }
}

fn node_to_json(node: &Node) -> serde_json::Value {
    let mut props = serde_json::Map::new();
    for (k, v) in &node.properties {
        props.insert(k.clone(), value_to_json(v));
    }
    serde_json::json!({
        "id": node.id,
        "type": node.entity_type,
        "properties": props,
    })
}

fn value_to_json(val: &Value) -> serde_json::Value {
    use pelago_proto::value::Kind;
    match &val.kind {
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::IntValue(n)) => serde_json::json!(n),
        Some(Kind::FloatValue(f)) => serde_json::json!(f),
        Some(Kind::BoolValue(b)) => serde_json::json!(b),
        Some(Kind::NullValue(_)) | None => serde_json::Value::Null,
        _ => serde_json::Value::Null,
    }
}

fn format_value(val: &Value) -> String {
    use pelago_proto::value::Kind;
    match &val.kind {
        Some(Kind::StringValue(s)) => s.clone(),
        Some(Kind::IntValue(n)) => n.to_string(),
        Some(Kind::FloatValue(f)) => f.to_string(),
        Some(Kind::BoolValue(b)) => b.to_string(),
        Some(Kind::TimestampValue(t)) => t.to_string(),
        Some(Kind::NullValue(_)) | None => "null".to_string(),
        _ => "?".to_string(),
    }
}

fn get_history_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        let dir = h.join(".pelago");
        let _ = std::fs::create_dir_all(&dir);
        dir.join("history")
    })
}

/// Check if a string is a meta-command
pub fn is_meta_command(input: &str) -> bool {
    input.starts_with(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_meta_command() {
        assert!(is_meta_command(":explain Person { name }"));
        assert!(is_meta_command(":use core"));
        assert!(is_meta_command(":format json"));
        assert!(is_meta_command(":param $min_age = 30"));
        assert!(is_meta_command(":quit"));
        assert!(is_meta_command(":help"));
        assert!(!is_meta_command("Person { name }"));
        assert!(!is_meta_command(
            "query { start(func: uid(Person:1_42)) { name } }"
        ));
    }

    #[test]
    fn test_needs_continuation() {
        assert!(needs_continuation("Person {"));
        assert!(needs_continuation(
            "query { start(func: type(Person)) {"
        ));
        assert!(!needs_continuation("Person { name }"));
        assert!(!needs_continuation(
            "query { start(func: type(Person)) { name } }"
        ));
        assert!(!needs_continuation(":help"));
    }
}
