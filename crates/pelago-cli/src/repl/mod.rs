use crate::connection::GrpcConnection;
use crate::output::OutputFormat;
use crate::schema_input::{parse_schema_input, SchemaInputFormat};
use pelago_proto::{
    value, CreateEdgeRequest, CreateNodeRequest, DeleteEdgeRequest, DeleteNodeRequest,
    DropEntityTypeRequest, Edge, EdgeDirection, EntitySchema, FindNodesRequest,
    GetJobStatusRequest, GetNamespaceSettingsRequest, GetNodeRequest, GetReplicationStatusRequest,
    GetSchemaRequest, IndexDefaultMode, Job, JobStatus, ListEdgesRequest, ListJobsRequest,
    ListSchemasRequest, ListSitesRequest, Node, NodeRef, RegisterSchemaRequest, RequestContext,
    SetNamespaceSchemaOwnerRequest, SortSpec, TraversalStep, TraverseRequest, TraverseResult,
    UpdateNodeRequest, Value,
};
use pelago_query::pql::{
    explain_query, parse_pql, CompiledBlock, InMemorySchemaProvider, PqlCompiler, PqlEdgeDirection,
    PqlResolver, SchemaInfo, SetOp,
};
use rustyline::history::DefaultHistory;
use rustyline::{CompletionType, Config, Editor};
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::time::Instant;

mod ux;

type ReplEditor = Editor<ux::ReplHelper, DefaultHistory>;

#[derive(clap::Args)]
pub struct ReplArgs {
    /// Initial namespace
    #[arg(long)]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplCompletionMode {
    #[default]
    Fuzzy,
    Prefix,
}

impl std::fmt::Display for ReplCompletionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplCompletionMode::Fuzzy => write!(f, "fuzzy"),
            ReplCompletionMode::Prefix => write!(f, "prefix"),
        }
    }
}

impl ReplCompletionMode {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "fuzzy" => Some(Self::Fuzzy),
            "prefix" => Some(Self::Prefix),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplColorMode {
    #[default]
    On,
    Off,
}

impl std::fmt::Display for ReplColorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplColorMode::On => write!(f, "on"),
            ReplColorMode::Off => write!(f, "off"),
        }
    }
}

impl ReplColorMode {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplUxSettings {
    pub completion_mode: ReplCompletionMode,
    pub color_mode: ReplColorMode,
}

#[derive(Debug, Clone, Default)]
struct ReplSchemaCatalog {
    entity_types: Vec<String>,
    fields_by_entity: HashMap<String, Vec<String>>,
    edges_by_entity: HashMap<String, Vec<String>>,
}

pub struct ReplContext {
    pub database: String,
    pub namespace: String,
    pub format: OutputFormat,
    pub params: HashMap<String, String>,
    pub ux: ReplUxSettings,
}

struct MetaCommandOutcome {
    exit: bool,
    refresh_schemas: bool,
    explain_query: Option<String>,
}

#[derive(Debug)]
enum ReplSetCommand {
    Show,
    Completion(ReplCompletionMode),
    Color(ReplColorMode),
}

enum SqlCommand {
    UseNamespace(String),
    UseDatabase(String),
    ShowSchemas,
    DescribeSchema(String),
    CreateSchema(EntitySchema),
    DropType(String),
    InsertNode {
        entity_type: String,
        properties: HashMap<String, Value>,
    },
    UpdateNode {
        entity_type: String,
        node_id: String,
        properties: HashMap<String, Value>,
    },
    DeleteNode {
        entity_type: String,
        node_id: String,
    },
    CreateEdge {
        source: NodeRef,
        label: String,
        target: NodeRef,
        properties: HashMap<String, Value>,
    },
    DeleteEdge {
        source: NodeRef,
        label: String,
        target: NodeRef,
    },
    ShowEdges {
        entity_type: String,
        node_id: String,
        direction: i32,
        label: Option<String>,
        limit: u32,
    },
    SelectNodes {
        entity_type: String,
        filter: String,
        limit: u32,
    },
    Traverse {
        start: NodeRef,
        edge_type: String,
        max_depth: u32,
        max_results: u32,
    },
    ShowJobs,
    ShowJob(String),
    ShowSites,
    ShowReplicationStatus,
    ShowNamespaceSettings,
    SetNamespaceOwner(String),
    ClearNamespaceOwner,
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
        ux: ReplUxSettings::default(),
    };

    // Connect to server (optional; REPL supports offline parse/explain mode)
    let conn = match GrpcConnection::connect(server).await {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("Warning: Could not connect to server at {}: {}", server, e);
            eprintln!("REPL will start in offline mode (parse/explain only).\n");
            None
        }
    };

    // Non-interactive mode for scripted execution (used by tests/automation).
    if !std::io::stdin().is_terminal() {
        return run_scripted_repl(&mut context, conn.as_ref(), server).await;
    }

    if let Some(conn) = conn {
        // Print banner
        println!("PelagoDB REPL v{}", env!("CARGO_PKG_VERSION"));
        println!("Connected to {} (db: {}, ns: {})", server, database, ns);
        println!("Type :help for commands, :quit to exit.");
        println!("Tab: completion, Ctrl-R: history search, Enter: smart multiline.\n");

        let mut session_history: Vec<String> = Vec::new();
        let history_path = get_history_path();

        // Fetch schemas for resolution and completion.
        let (mut schemas, mut schema_catalog) = fetch_schemas(&conn, &context)
            .await
            .unwrap_or_else(|_| (InMemorySchemaProvider::new(), ReplSchemaCatalog::default()));

        let mut rl = create_editor(&context, &schema_catalog)?;
        if let Some(ref path) = history_path {
            let _ = rl.load_history(path);
        }

        loop {
            sync_editor_context(&mut rl, &context, &schema_catalog);

            let prompt = format_prompt(&context);
            let line = match rl.readline(&prompt) {
                Ok(line) => line,
                Err(rustyline::error::ReadlineError::Interrupted) => continue,
                Err(rustyline::error::ReadlineError::Eof) => break,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    break;
                }
            };

            let input = line.trim();
            if input.is_empty() {
                continue;
            }

            // Handle meta-commands
            if input.starts_with(':') {
                let _ = rl.add_history_entry(input);
                session_history.push(input.to_string());

                let outcome = handle_meta_command(input, &mut context, &session_history);
                if let Some(explain_query) = outcome.explain_query {
                    execute_explain_input(&explain_query, &context, &schemas);
                }
                if outcome.refresh_schemas {
                    if let Ok((next_schemas, next_catalog)) = fetch_schemas(&conn, &context).await {
                        schemas = next_schemas;
                        schema_catalog = next_catalog;
                    }
                }
                if outcome.exit {
                    break; // :quit
                }
                continue;
            }

            let _ = rl.add_history_entry(input);
            session_history.push(input.to_string());

            let statement = apply_params(input, &context.params);

            if let Some(refresh_schemas) =
                try_execute_sql_command(&statement, Some(&conn), &mut context).await
            {
                if refresh_schemas {
                    if let Ok((next_schemas, next_catalog)) = fetch_schemas(&conn, &context).await {
                        schemas = next_schemas;
                        schema_catalog = next_catalog;
                    }
                }
                continue;
            }

            // Execute PQL
            execute_pql_input(&statement, &conn, &context, &schemas).await;
        }

        // Save history
        if let Some(ref path) = history_path {
            let _ = rl.save_history(path);
        }

        println!("Goodbye!");
        Ok(())
    } else {
        run_offline_repl(&mut context).await
    }
}

async fn run_scripted_repl(
    context: &mut ReplContext,
    conn: Option<&GrpcConnection>,
    server: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if conn.is_some() {
        println!("PelagoDB REPL v{}", env!("CARGO_PKG_VERSION"));
        println!(
            "Connected to {} (db: {}, ns: {})",
            server, context.database, context.namespace
        );
        println!("Type :help for commands, :quit to exit.\n");
    } else {
        println!(
            "PelagoDB REPL v{} (offline mode)",
            env!("CARGO_PKG_VERSION")
        );
        println!("Type :help for commands, :quit to exit.\n");
    }

    let mut session_history: Vec<String> = Vec::new();
    let mut pending_query = String::new();
    let mut schemas = match conn {
        Some(c) => fetch_schemas(c, context)
            .await
            .map(|(provider, _)| provider)
            .unwrap_or_default(),
        None => InMemorySchemaProvider::new(),
    };

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if pending_query.is_empty() && trimmed.starts_with(':') {
            session_history.push(trimmed.to_string());
            let outcome = handle_meta_command(trimmed, context, &session_history);
            if let Some(explain_query) = outcome.explain_query {
                execute_explain_input(&explain_query, context, &schemas);
            }
            if outcome.refresh_schemas {
                if let Some(c) = conn {
                    schemas = fetch_schemas(c, context)
                        .await
                        .map(|(provider, _)| provider)
                        .unwrap_or_default();
                }
            }
            if outcome.exit {
                println!("Goodbye!");
                return Ok(());
            }
            continue;
        }

        if !pending_query.is_empty() {
            pending_query.push('\n');
            pending_query.push_str(trimmed);
        } else {
            pending_query = trimmed.to_string();
        }

        if needs_continuation(&pending_query) {
            continue;
        }

        let raw_query = pending_query.trim().to_string();
        session_history.push(raw_query.clone());
        let query = apply_params(&raw_query, &context.params);
        if let Some(refresh_schemas) = try_execute_sql_command(&query, conn, context).await {
            if refresh_schemas {
                if let Some(c) = conn {
                    schemas = fetch_schemas(c, context)
                        .await
                        .map(|(provider, _)| provider)
                        .unwrap_or_default();
                }
            }
            pending_query.clear();
            continue;
        }

        if let Some(c) = conn {
            execute_pql_input(&query, c, context, &schemas).await;
        } else {
            execute_offline_pql_input(&query, &schemas);
        }
        pending_query.clear();
    }

    if !pending_query.trim().is_empty() {
        let raw_query = pending_query.trim().to_string();
        session_history.push(raw_query.clone());
        let query = apply_params(&raw_query, &context.params);
        if try_execute_sql_command(&query, conn, context)
            .await
            .is_some()
        {
            println!("Goodbye!");
            return Ok(());
        }
        if let Some(c) = conn {
            execute_pql_input(&query, c, context, &schemas).await;
        } else {
            execute_offline_pql_input(&query, &schemas);
        }
    }

    println!("Goodbye!");
    Ok(())
}

fn execute_offline_pql_input(input: &str, schemas: &InMemorySchemaProvider) {
    let ast = match parse_pql(input) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            return;
        }
    };

    println!("Parsed successfully ({} block(s))", ast.blocks.len());
    let resolver = PqlResolver::new();
    match resolver.resolve(&ast, schemas) {
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

async fn run_offline_repl(context: &mut ReplContext) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "PelagoDB REPL v{} (offline mode)",
        env!("CARGO_PKG_VERSION")
    );
    println!("Type :help for commands, :quit to exit.");
    println!("Tab: completion, Ctrl-R: history search, Enter: smart multiline.\n");

    let history_path = get_history_path();
    let schema_catalog = ReplSchemaCatalog::default();
    let mut rl = create_editor(context, &schema_catalog)?;
    let mut session_history: Vec<String> = Vec::new();
    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }
    let schemas = InMemorySchemaProvider::new();

    loop {
        sync_editor_context(&mut rl, context, &schema_catalog);
        let prompt = format_prompt(context);
        let line = match rl.readline(&prompt) {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        };

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        if input.starts_with(':') {
            let _ = rl.add_history_entry(input);
            session_history.push(input.to_string());

            let outcome = handle_meta_command(input, context, &session_history);
            if let Some(explain_query) = outcome.explain_query {
                execute_explain_input(&explain_query, context, &schemas);
            }
            if outcome.exit {
                break;
            }
            continue;
        }

        let _ = rl.add_history_entry(input);
        session_history.push(input.to_string());

        let statement = apply_params(input, &context.params);
        if try_execute_sql_command(&statement, None, context)
            .await
            .is_some()
        {
            continue;
        }

        match parse_pql(&statement) {
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

fn create_editor(
    context: &ReplContext,
    schema_catalog: &ReplSchemaCatalog,
) -> Result<ReplEditor, Box<dyn std::error::Error>> {
    let config = Config::builder()
        .max_history_size(1_000)?
        .history_ignore_dups(true)?
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .completion_prompt_limit(50)
        .build();

    let mut rl = ReplEditor::with_config(config)?;
    let mut helper = ux::ReplHelper::new();
    helper.set_schema_catalog(schema_catalog);
    helper.set_params(context.params.keys());
    helper.set_completion_mode(context.ux.completion_mode);
    helper.set_color_mode(context.ux.color_mode);
    rl.set_helper(Some(helper));
    Ok(rl)
}

fn sync_editor_context(
    rl: &mut ReplEditor,
    context: &ReplContext,
    schema_catalog: &ReplSchemaCatalog,
) {
    if let Some(helper) = rl.helper_mut() {
        helper.set_schema_catalog(schema_catalog);
        helper.set_params(context.params.keys());
        helper.set_completion_mode(context.ux.completion_mode);
        helper.set_color_mode(context.ux.color_mode);
    }
}

fn format_prompt(context: &ReplContext) -> String {
    format!(
        "{}:{} [{}]> ",
        context.database, context.namespace, context.format
    )
}

fn parse_set_command(arg: Option<&str>) -> Result<ReplSetCommand, String> {
    let Some(arg) = arg else {
        return Ok(ReplSetCommand::Show);
    };
    let mut parts = arg.split_whitespace();
    let key = parts
        .next()
        .ok_or_else(|| "Usage: :set [completion <fuzzy|prefix> | color <on|off>]".to_string())?;
    let value = parts.next();
    let extra = parts.next();
    if extra.is_some() {
        return Err("Usage: :set [completion <fuzzy|prefix> | color <on|off>]".to_string());
    }

    match key.to_ascii_lowercase().as_str() {
        "completion" => {
            let mode = value.ok_or_else(|| "Usage: :set completion <fuzzy|prefix>".to_string())?;
            let parsed = ReplCompletionMode::parse(mode).ok_or_else(|| {
                format!("Unknown completion mode '{}'. Use fuzzy or prefix.", mode)
            })?;
            Ok(ReplSetCommand::Completion(parsed))
        }
        "color" => {
            let mode = value.ok_or_else(|| "Usage: :set color <on|off>".to_string())?;
            let parsed = ReplColorMode::parse(mode)
                .ok_or_else(|| format!("Unknown color mode '{}'. Use on or off.", mode))?;
            Ok(ReplSetCommand::Color(parsed))
        }
        _ => Err(format!(
            "Unknown :set key '{}'. Supported keys: completion, color.",
            key
        )),
    }
}

fn print_ux_settings(context: &ReplContext) {
    println!("UX settings:");
    println!("  completion = {}", context.ux.completion_mode);
    println!("  color      = {}", context.ux.color_mode);
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
        ":set" => match parse_set_command(arg) {
            Ok(ReplSetCommand::Show) => print_ux_settings(context),
            Ok(ReplSetCommand::Completion(mode)) => {
                context.ux.completion_mode = mode;
                println!("Completion mode: {}", mode);
            }
            Ok(ReplSetCommand::Color(mode)) => {
                context.ux.color_mode = mode;
                println!("Color mode: {}", mode);
            }
            Err(msg) => println!("{}", msg),
        },
        ":param" => {
            if let Some(assignment) = arg {
                if let Some((name, value)) = assignment.split_once('=') {
                    let name = name.trim().trim_start_matches('$');
                    let value = value.trim();
                    context.params.insert(name.to_string(), value.to_string());
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

async fn try_execute_sql_command(
    input: &str,
    conn: Option<&GrpcConnection>,
    context: &mut ReplContext,
) -> Option<bool> {
    let parsed = match parse_sql_command(input) {
        None => return None,
        Some(Ok(cmd)) => cmd,
        Some(Err(err)) => {
            eprintln!("SQL parse error: {}", err);
            return Some(false);
        }
    };

    if conn.is_none() {
        match parsed {
            SqlCommand::UseNamespace(ns) => {
                context.namespace = ns;
                println!("Switched to namespace: {}", context.namespace);
                return Some(false);
            }
            SqlCommand::UseDatabase(db) => {
                context.database = db;
                println!("Switched to database: {}", context.database);
                return Some(false);
            }
            _ => {
                eprintln!("This SQL command requires a live server connection.");
                return Some(false);
            }
        }
    }

    match execute_sql_command(parsed, conn.expect("checked Some"), context).await {
        Ok(refresh) => Some(refresh),
        Err(err) => {
            eprintln!("SQL execution error: {}", err);
            Some(false)
        }
    }
}

async fn execute_sql_command(
    command: SqlCommand,
    conn: &GrpcConnection,
    context: &mut ReplContext,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut refresh_schemas = false;

    match command {
        SqlCommand::UseNamespace(ns) => {
            context.namespace = ns;
            println!("Switched to namespace: {}", context.namespace);
            refresh_schemas = true;
        }
        SqlCommand::UseDatabase(db) => {
            context.database = db;
            println!("Switched to database: {}", context.database);
            refresh_schemas = true;
        }
        SqlCommand::ShowSchemas => {
            let mut client = conn.schema_client();
            let resp = client
                .list_schemas(ListSchemasRequest {
                    context: Some(request_context(context)),
                })
                .await?
                .into_inner();
            format_sql_schema_list(&resp.schemas, &context.format);
        }
        SqlCommand::DescribeSchema(entity_type) => {
            let mut client = conn.schema_client();
            let resp = client
                .get_schema(GetSchemaRequest {
                    context: Some(request_context(context)),
                    entity_type,
                    version: 0,
                })
                .await?
                .into_inner();

            if let Some(schema) = resp.schema {
                format_sql_schema(&schema, &context.format);
            } else {
                println!("Schema not found");
            }
        }
        SqlCommand::CreateSchema(schema) => {
            let mut client = conn.schema_client();
            let resp = client
                .register_schema(RegisterSchemaRequest {
                    context: Some(request_context(context)),
                    schema: Some(schema),
                    index_default_mode: IndexDefaultMode::AutoByTypeV1 as i32,
                })
                .await?
                .into_inner();
            match context.format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "version": resp.version,
                        "created": resp.created,
                    }));
                }
                _ => {
                    println!(
                        "Schema registered (version: {}, new: {})",
                        resp.version, resp.created
                    );
                }
            }
            refresh_schemas = true;
        }
        SqlCommand::DropType(entity_type) => {
            let mut client = conn.admin_client();
            let resp = client
                .drop_entity_type(DropEntityTypeRequest {
                    context: Some(request_context(context)),
                    entity_type: entity_type.clone(),
                    mutation_mode: pelago_proto::MutationExecutionMode::AsyncAllowed as i32,
                })
                .await?
                .into_inner();
            match context.format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "entity_type": entity_type,
                        "cleanup_job_id": resp.cleanup_job_id,
                    }));
                }
                _ => {
                    println!("Dropped entity type: {}", entity_type);
                    if !resp.cleanup_job_id.is_empty() {
                        println!("Cleanup job: {}", resp.cleanup_job_id);
                    }
                }
            }
            refresh_schemas = true;
        }
        SqlCommand::InsertNode {
            entity_type,
            properties,
        } => {
            let mut client = conn.node_client();
            let resp = client
                .create_node(CreateNodeRequest {
                    context: Some(request_context(context)),
                    entity_type,
                    properties,
                })
                .await?
                .into_inner();
            if let Some(node) = resp.node {
                format_result_node(&node, "insert", &context.format);
            } else {
                println!("Insert completed.");
            }
        }
        SqlCommand::UpdateNode {
            entity_type,
            node_id,
            properties,
        } => {
            let mut client = conn.node_client();
            let resp = client
                .update_node(UpdateNodeRequest {
                    context: Some(request_context(context)),
                    entity_type,
                    node_id,
                    properties,
                })
                .await?
                .into_inner();
            if let Some(node) = resp.node {
                format_result_node(&node, "update", &context.format);
            } else {
                println!("Update completed.");
            }
        }
        SqlCommand::DeleteNode {
            entity_type,
            node_id,
        } => {
            let mut client = conn.node_client();
            let resp = client
                .delete_node(DeleteNodeRequest {
                    context: Some(request_context(context)),
                    entity_type: entity_type.clone(),
                    node_id: node_id.clone(),
                    mutation_mode: pelago_proto::MutationExecutionMode::AsyncAllowed as i32,
                })
                .await?
                .into_inner();
            match context.format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "entity_type": entity_type,
                        "node_id": node_id,
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
        SqlCommand::CreateEdge {
            source,
            label,
            target,
            properties,
        } => {
            let mut client = conn.edge_client();
            let resp = client
                .create_edge(CreateEdgeRequest {
                    context: Some(request_context(context)),
                    source: Some(source),
                    target: Some(target),
                    label,
                    properties,
                })
                .await?
                .into_inner();
            if let Some(edge) = resp.edge {
                format_sql_edge(&edge, &context.format);
            } else {
                println!("Edge created.");
            }
        }
        SqlCommand::DeleteEdge {
            source,
            label,
            target,
        } => {
            let mut client = conn.edge_client();
            let resp = client
                .delete_edge(DeleteEdgeRequest {
                    context: Some(request_context(context)),
                    source: Some(source),
                    target: Some(target),
                    label,
                })
                .await?
                .into_inner();
            match context.format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "deleted": resp.deleted,
                    }));
                }
                _ => {
                    println!("Deleted: {}", resp.deleted);
                }
            }
        }
        SqlCommand::ShowEdges {
            entity_type,
            node_id,
            direction,
            label,
            limit,
        } => {
            let mut client = conn.edge_client();
            let mut stream = client
                .list_edges(ListEdgesRequest {
                    context: Some(request_context(context)),
                    entity_type,
                    node_id,
                    label: label.unwrap_or_default(),
                    direction,
                    consistency: pelago_proto::ReadConsistency::Strong as i32,
                    limit,
                    cursor: vec![],
                })
                .await?
                .into_inner();
            let mut edges = Vec::new();
            while let Some(item) = stream.message().await? {
                if let Some(edge) = item.edge {
                    edges.push(edge);
                }
            }
            format_sql_edges(&edges, &context.format);
        }
        SqlCommand::SelectNodes {
            entity_type,
            filter,
            limit,
        } => {
            let mut client = conn.query_client();
            let mut stream = client
                .find_nodes(FindNodesRequest {
                    context: Some(request_context(context)),
                    entity_type,
                    cel_expression: filter,
                    consistency: pelago_proto::ReadConsistency::Strong as i32,
                    fields: vec![],
                    limit,
                    cursor: vec![],
                    snapshot_mode: pelago_proto::SnapshotMode::Strict as i32,
                    allow_degrade_to_best_effort: false,
                })
                .await?
                .into_inner();

            let mut nodes = Vec::new();
            while let Some(item) = stream.message().await? {
                if let Some(node) = item.node {
                    nodes.push(node);
                }
            }
            format_result_nodes(&nodes, "select", &context.format);
        }
        SqlCommand::Traverse {
            start,
            edge_type,
            max_depth,
            max_results,
        } => {
            let mut client = conn.query_client();
            let mut stream = client
                .traverse(TraverseRequest {
                    context: Some(request_context(context)),
                    start: Some(start),
                    steps: vec![TraversalStep {
                        edge_type,
                        direction: EdgeDirection::Outgoing as i32,
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
                    consistency: pelago_proto::ReadConsistency::Strong as i32,
                    cascade: false,
                    cursor: vec![],
                    snapshot_mode: pelago_proto::SnapshotMode::BestEffort as i32,
                    allow_degrade_to_best_effort: true,
                })
                .await?
                .into_inner();
            let mut results = Vec::new();
            while let Some(item) = stream.message().await? {
                results.push(item);
            }
            format_traverse_results(&results, "traverse", &context.format);
        }
        SqlCommand::ShowJobs => {
            let mut client = conn.admin_client();
            let resp = client
                .list_jobs(ListJobsRequest {
                    context: Some(request_context(context)),
                })
                .await?
                .into_inner();
            format_sql_jobs(&resp.jobs, &context.format);
        }
        SqlCommand::ShowJob(job_id) => {
            let mut client = conn.admin_client();
            let resp = client
                .get_job_status(GetJobStatusRequest {
                    context: Some(request_context(context)),
                    job_id,
                })
                .await?
                .into_inner();
            if let Some(job) = resp.job {
                format_sql_job(&job, &context.format);
            } else {
                println!("Job not found");
            }
        }
        SqlCommand::ShowSites => {
            let mut client = conn.admin_client();
            let resp = client
                .list_sites(ListSitesRequest {
                    context: Some(request_context(context)),
                })
                .await?
                .into_inner();
            format_sql_sites(&resp.sites, &context.format);
        }
        SqlCommand::ShowReplicationStatus => {
            let mut client = conn.admin_client();
            let resp = client
                .get_replication_status(GetReplicationStatusRequest {
                    context: Some(request_context(context)),
                })
                .await?
                .into_inner();
            format_sql_replication_status(&resp.peers, &context.format);
        }
        SqlCommand::ShowNamespaceSettings => {
            let mut client = conn.admin_client();
            let resp = client
                .get_namespace_settings(GetNamespaceSettingsRequest {
                    context: Some(request_context(context)),
                })
                .await?
                .into_inner();
            format_sql_namespace_settings(&resp, context);
        }
        SqlCommand::SetNamespaceOwner(site_id) => {
            let mut client = conn.admin_client();
            let resp = client
                .set_namespace_schema_owner(SetNamespaceSchemaOwnerRequest {
                    context: Some(request_context(context)),
                    site_id: site_id.clone(),
                })
                .await?
                .into_inner();
            if let Some(settings) = resp.settings {
                match context.format {
                    OutputFormat::Json => {
                        crate::output::print_json(&serde_json::json!({
                            "database": settings.database,
                            "namespace": settings.namespace,
                            "schema_owner_site_id": settings.schema_owner_site_id,
                            "epoch": settings.epoch,
                            "updated_at": settings.updated_at,
                        }));
                    }
                    _ => {
                        println!(
                            "Namespace owner set: {}/{} -> {} (epoch {})",
                            settings.database,
                            settings.namespace,
                            settings.schema_owner_site_id,
                            settings.epoch
                        );
                    }
                }
            } else {
                println!("Namespace owner updated.");
            }
        }
        SqlCommand::ClearNamespaceOwner => {
            let mut client = conn.admin_client();
            let resp = client
                .set_namespace_schema_owner(SetNamespaceSchemaOwnerRequest {
                    context: Some(request_context(context)),
                    site_id: String::new(),
                })
                .await?
                .into_inner();
            if let Some(settings) = resp.settings {
                match context.format {
                    OutputFormat::Json => {
                        crate::output::print_json(&serde_json::json!({
                            "database": settings.database,
                            "namespace": settings.namespace,
                            "schema_owner_site_id": serde_json::Value::Null,
                            "epoch": settings.epoch,
                            "updated_at": settings.updated_at,
                        }));
                    }
                    _ => {
                        println!(
                            "Namespace owner cleared: {}/{} (epoch {})",
                            settings.database, settings.namespace, settings.epoch
                        );
                    }
                }
            } else {
                println!("Namespace owner cleared.");
            }
        }
    }

    Ok(refresh_schemas)
}

fn parse_sql_command(input: &str) -> Option<Result<SqlCommand, String>> {
    let statement = trim_sql_statement(input);
    if statement.is_empty() {
        return None;
    }

    if starts_with_ci(statement, "create type ") || starts_with_ci(statement, "create schema ") {
        return Some(
            parse_schema_input(statement, SchemaInputFormat::Sql)
                .map(SqlCommand::CreateSchema)
                .map_err(|e| format!("Invalid CREATE TYPE statement: {}", e)),
        );
    }

    if let Some(rest) = strip_prefix_ci(statement, "create edge ") {
        return Some(parse_create_edge_sql(rest));
    }

    if let Some(rest) = strip_prefix_ci(statement, "use database ") {
        let db = rest.trim();
        if db.is_empty() {
            return Some(Err("USE DATABASE requires a database name".to_string()));
        }
        return Some(Ok(SqlCommand::UseDatabase(db.to_string())));
    }

    if let Some(rest) = strip_prefix_ci(statement, "use db ") {
        let db = rest.trim();
        if db.is_empty() {
            return Some(Err("USE DB requires a database name".to_string()));
        }
        return Some(Ok(SqlCommand::UseDatabase(db.to_string())));
    }

    if let Some(rest) = strip_prefix_ci(statement, "use namespace ") {
        let ns = rest.trim();
        if ns.is_empty() {
            return Some(Err("USE NAMESPACE requires a namespace".to_string()));
        }
        return Some(Ok(SqlCommand::UseNamespace(ns.to_string())));
    }

    if let Some(rest) = strip_prefix_ci(statement, "use ") {
        let ns = rest.trim();
        if ns.is_empty() {
            return Some(Err("USE requires a namespace".to_string()));
        }
        return Some(Ok(SqlCommand::UseNamespace(ns.to_string())));
    }

    if statement.eq_ignore_ascii_case("show schemas") {
        return Some(Ok(SqlCommand::ShowSchemas));
    }

    if let Some(rest) = strip_prefix_ci(statement, "describe ") {
        let entity = rest.trim();
        if entity.is_empty() {
            return Some(Err("DESCRIBE requires an entity type".to_string()));
        }
        return Some(Ok(SqlCommand::DescribeSchema(entity.to_string())));
    }

    if let Some(rest) = strip_prefix_ci(statement, "show schema ") {
        let entity = rest.trim();
        if entity.is_empty() {
            return Some(Err("SHOW SCHEMA requires an entity type".to_string()));
        }
        return Some(Ok(SqlCommand::DescribeSchema(entity.to_string())));
    }

    if let Some(rest) = strip_prefix_ci(statement, "drop type ") {
        let entity = rest.trim();
        if entity.is_empty() {
            return Some(Err("DROP TYPE requires an entity type".to_string()));
        }
        return Some(Ok(SqlCommand::DropType(entity.to_string())));
    }

    if let Some(rest) = strip_prefix_ci(statement, "insert into ") {
        return Some(parse_insert_sql(rest));
    }

    if let Some(rest) = strip_prefix_ci(statement, "update ") {
        return Some(parse_update_sql(rest));
    }

    if let Some(rest) = strip_prefix_ci(statement, "delete edge ") {
        return Some(parse_delete_edge_sql(rest));
    }

    if let Some(rest) = strip_prefix_ci(statement, "delete from ") {
        return Some(parse_delete_sql(rest));
    }

    if let Some(rest) = strip_prefix_ci(statement, "select ") {
        return Some(parse_select_sql(rest));
    }

    if let Some(rest) = strip_prefix_ci(statement, "traverse ") {
        return Some(parse_traverse_sql(rest));
    }

    if statement.eq_ignore_ascii_case("show jobs") {
        return Some(Ok(SqlCommand::ShowJobs));
    }

    if let Some(rest) = strip_prefix_ci(statement, "show job ") {
        let job_id = rest.trim();
        if job_id.is_empty() {
            return Some(Err("SHOW JOB requires a job ID".to_string()));
        }
        return Some(Ok(SqlCommand::ShowJob(job_id.to_string())));
    }

    if statement.eq_ignore_ascii_case("show sites") {
        return Some(Ok(SqlCommand::ShowSites));
    }

    if let Some(rest) = strip_prefix_ci(statement, "show edges ") {
        return Some(parse_show_edges_sql(rest));
    }

    if statement.eq_ignore_ascii_case("show replication status") {
        return Some(Ok(SqlCommand::ShowReplicationStatus));
    }

    if statement.eq_ignore_ascii_case("show namespace settings") {
        return Some(Ok(SqlCommand::ShowNamespaceSettings));
    }

    if let Some(rest) = strip_prefix_ci(statement, "set namespace owner ") {
        let site_id = rest.trim();
        if site_id.is_empty() {
            return Some(Err("SET NAMESPACE OWNER requires a site id".to_string()));
        }
        return Some(Ok(SqlCommand::SetNamespaceOwner(site_id.to_string())));
    }

    if statement.eq_ignore_ascii_case("clear namespace owner") {
        return Some(Ok(SqlCommand::ClearNamespaceOwner));
    }

    None
}

fn parse_insert_sql(rest: &str) -> Result<SqlCommand, String> {
    let (entity_type, mut payload) = split_first_word(rest)
        .ok_or_else(|| "INSERT INTO requires entity type and JSON payload".to_string())?;
    payload = payload.trim();

    if let Some(after_values) = strip_prefix_ci(payload, "values ") {
        payload = after_values.trim();
    }

    let properties = parse_json_props(payload)?;
    Ok(SqlCommand::InsertNode {
        entity_type: entity_type.to_string(),
        properties,
    })
}

fn parse_update_sql(rest: &str) -> Result<SqlCommand, String> {
    let upper = rest.to_ascii_uppercase();
    let set_idx = upper
        .find(" SET ")
        .ok_or_else(|| "UPDATE requires SET <json_payload>".to_string())?;

    let target = rest[..set_idx].trim();
    let payload = rest[set_idx + 5..].trim();
    let (entity_type, node_id) = parse_entity_node_target(target)?;
    let properties = parse_json_props(payload)?;

    Ok(SqlCommand::UpdateNode {
        entity_type,
        node_id,
        properties,
    })
}

fn parse_delete_sql(rest: &str) -> Result<SqlCommand, String> {
    let (entity_type, node_id) = parse_entity_node_target(rest.trim())?;
    Ok(SqlCommand::DeleteNode {
        entity_type,
        node_id,
    })
}

fn parse_create_edge_sql(rest: &str) -> Result<SqlCommand, String> {
    let (source, label, target, tail) = parse_edge_triplet(rest)?;
    let properties = if tail.trim().is_empty() {
        HashMap::new()
    } else if let Some(payload) = strip_prefix_ci(tail.trim(), "values ") {
        parse_json_props(payload.trim())?
    } else {
        return Err("CREATE EDGE supports optional VALUES <json_payload>".to_string());
    };

    Ok(SqlCommand::CreateEdge {
        source: parse_node_ref_sql(source)?,
        label: label.to_string(),
        target: parse_node_ref_sql(target)?,
        properties,
    })
}

fn parse_delete_edge_sql(rest: &str) -> Result<SqlCommand, String> {
    let (source, label, target, tail) = parse_edge_triplet(rest)?;
    if !tail.trim().is_empty() {
        return Err("DELETE EDGE does not accept trailing clauses".to_string());
    }
    Ok(SqlCommand::DeleteEdge {
        source: parse_node_ref_sql(source)?,
        label: label.to_string(),
        target: parse_node_ref_sql(target)?,
    })
}

fn parse_show_edges_sql(rest: &str) -> Result<SqlCommand, String> {
    let mut parts = rest.split_whitespace();
    let target = parts
        .next()
        .ok_or_else(|| "SHOW EDGES requires a target node (Type:id)".to_string())?;
    let (entity_type, node_id) = parse_entity_node_target(target)?;

    let mut direction = EdgeDirection::Outgoing as i32;
    let mut label = None;
    let mut limit = 100u32;

    while let Some(token) = parts.next() {
        if token.eq_ignore_ascii_case("direction") {
            let dir = parts
                .next()
                .ok_or_else(|| "SHOW EDGES DIRECTION requires out|in|both".to_string())?;
            direction = parse_edge_direction(dir)?;
            continue;
        }
        if token.eq_ignore_ascii_case("label") {
            let edge_label = parts
                .next()
                .ok_or_else(|| "SHOW EDGES LABEL requires a label value".to_string())?;
            label = Some(edge_label.to_string());
            continue;
        }
        if token.eq_ignore_ascii_case("limit") {
            let raw = parts
                .next()
                .ok_or_else(|| "SHOW EDGES LIMIT requires an integer".to_string())?;
            limit = raw
                .parse::<u32>()
                .map_err(|_| format!("Invalid SHOW EDGES LIMIT '{}'", raw))?;
            continue;
        }
        return Err(format!("Unsupported SHOW EDGES clause '{}'", token));
    }

    Ok(SqlCommand::ShowEdges {
        entity_type,
        node_id,
        direction,
        label,
        limit,
    })
}

fn parse_traverse_sql(rest: &str) -> Result<SqlCommand, String> {
    let mut parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.is_empty() {
        return Err("TRAVERSE requires a start node and edge label".to_string());
    }

    if parts[0].eq_ignore_ascii_case("from") {
        parts.remove(0);
    }
    if parts.is_empty() {
        return Err("TRAVERSE requires a start node and edge label".to_string());
    }

    let start_ref = parts[0];
    if parts.len() < 2 {
        return Err("TRAVERSE requires an edge label".to_string());
    }

    let mut idx = 1usize;
    if parts[idx].eq_ignore_ascii_case("via") {
        idx += 1;
    }
    if idx >= parts.len() {
        return Err("TRAVERSE requires an edge label".to_string());
    }
    let edge_type = parts[idx].to_string();
    idx += 1;

    let mut max_depth = 1u32;
    let mut max_results = 100u32;

    while idx < parts.len() {
        let key = parts[idx];
        idx += 1;
        if idx >= parts.len() {
            return Err(format!("TRAVERSE missing value for '{}'", key));
        }
        let value = parts[idx];
        idx += 1;

        if key.eq_ignore_ascii_case("depth") || key.eq_ignore_ascii_case("max_depth") {
            max_depth = value
                .parse::<u32>()
                .map_err(|_| format!("Invalid TRAVERSE depth '{}'", value))?;
            continue;
        }
        if key.eq_ignore_ascii_case("max_results") || key.eq_ignore_ascii_case("limit") {
            max_results = value
                .parse::<u32>()
                .map_err(|_| format!("Invalid TRAVERSE max_results '{}'", value))?;
            continue;
        }

        return Err(format!("Unsupported TRAVERSE clause '{}'", key));
    }

    Ok(SqlCommand::Traverse {
        start: parse_node_ref_sql(start_ref)?,
        edge_type,
        max_depth,
        max_results,
    })
}

fn parse_edge_triplet<'a>(input: &'a str) -> Result<(&'a str, &'a str, &'a str, &'a str), String> {
    let (source, rest) = split_word_and_tail(input)
        .ok_or_else(|| "EDGE command requires source node".to_string())?;
    if rest.trim().is_empty() {
        return Err("EDGE command requires source, label, and target".to_string());
    }
    let rest = rest.trim();

    if let Some(after_arrow) = rest.strip_prefix("-[") {
        let close_idx = after_arrow
            .find("]->")
            .ok_or_else(|| "Expected edge arrow syntax '-[LABEL]->'".to_string())?;
        let label = after_arrow[..close_idx].trim();
        if label.is_empty() {
            return Err("Edge label cannot be empty".to_string());
        }
        let remainder = after_arrow[close_idx + 3..].trim();
        let (target, tail) = split_word_and_tail(remainder)
            .ok_or_else(|| "EDGE command requires target node".to_string())?;
        return Ok((source, label, target, tail));
    }

    let (label, rest) =
        split_word_and_tail(rest).ok_or_else(|| "EDGE command requires edge label".to_string())?;
    let (target, tail) =
        split_word_and_tail(rest).ok_or_else(|| "EDGE command requires target node".to_string())?;
    Ok((source, label, target, tail))
}

fn parse_select_sql(rest: &str) -> Result<SqlCommand, String> {
    let rest = rest.trim();
    let after_from = strip_prefix_ci(rest, "* from ")
        .ok_or_else(|| "Only SELECT * FROM <type> [WHERE ...] [LIMIT n] is supported".to_string())?
        .trim();

    let mut base = after_from;
    let mut limit = 100u32;
    let mut filter = String::new();

    let upper = base.to_ascii_uppercase();
    if let Some(limit_idx) = upper.rfind(" LIMIT ") {
        let limit_raw = base[limit_idx + 7..].trim();
        limit = limit_raw
            .parse::<u32>()
            .map_err(|_| format!("Invalid LIMIT value '{}'", limit_raw))?;
        base = base[..limit_idx].trim();
    }

    let upper = base.to_ascii_uppercase();
    let entity_type = if let Some(where_idx) = upper.find(" WHERE ") {
        filter = base[where_idx + 7..].trim().to_string();
        base[..where_idx].trim()
    } else {
        base.trim()
    };

    if entity_type.is_empty() {
        return Err("SELECT is missing entity type after FROM".to_string());
    }

    Ok(SqlCommand::SelectNodes {
        entity_type: entity_type.to_string(),
        filter,
        limit,
    })
}

fn parse_entity_node_target(target: &str) -> Result<(String, String), String> {
    if let Some((entity_type, node_id)) = target.split_once(':') {
        let entity_type = entity_type.trim();
        let node_id = node_id.trim();
        if entity_type.is_empty() || node_id.is_empty() {
            return Err(format!(
                "Expected target as Type:node_id (received '{}')",
                target
            ));
        }
        return Ok((entity_type.to_string(), node_id.to_string()));
    }

    let mut parts = target.split_whitespace();
    let entity_type = parts.next();
    let node_id = parts.next();
    let extra = parts.next();
    match (entity_type, node_id, extra) {
        (Some(e), Some(n), None) => Ok((e.to_string(), n.to_string())),
        _ => Err(format!(
            "Expected target as Type:node_id or '<Type> <node_id>' (received '{}')",
            target
        )),
    }
}

fn parse_node_ref_sql(input: &str) -> Result<NodeRef, String> {
    let (entity_type, node_id) = parse_entity_node_target(input)?;
    Ok(NodeRef {
        entity_type,
        node_id,
        database: String::new(),
        namespace: String::new(),
    })
}

fn parse_edge_direction(input: &str) -> Result<i32, String> {
    match input.to_ascii_lowercase().as_str() {
        "out" | "outgoing" => Ok(EdgeDirection::Outgoing as i32),
        "in" | "incoming" => Ok(EdgeDirection::Incoming as i32),
        "both" => Ok(EdgeDirection::Both as i32),
        _ => Err(format!(
            "Unsupported edge direction '{}'. Expected out|in|both",
            input
        )),
    }
}

fn parse_json_props(input: &str) -> Result<HashMap<String, Value>, String> {
    let json: serde_json::Value =
        serde_json::from_str(input).map_err(|e| format!("Expected JSON object payload: {}", e))?;
    let obj = json
        .as_object()
        .ok_or_else(|| "JSON payload must be an object".to_string())?;

    let mut out = HashMap::new();
    for (k, v) in obj {
        out.insert(k.clone(), json_to_proto_value(v));
    }
    Ok(out)
}

fn json_to_proto_value(val: &serde_json::Value) -> Value {
    Value {
        kind: Some(match val {
            serde_json::Value::String(s) => value::Kind::StringValue(s.clone()),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    value::Kind::IntValue(i)
                } else {
                    value::Kind::FloatValue(n.as_f64().unwrap_or_default())
                }
            }
            serde_json::Value::Bool(b) => value::Kind::BoolValue(*b),
            serde_json::Value::Null => value::Kind::NullValue(true),
            _ => value::Kind::StringValue(val.to_string()),
        }),
    }
}

fn request_context(context: &ReplContext) -> RequestContext {
    RequestContext {
        database: context.database.clone(),
        namespace: context.namespace.clone(),
        site_id: String::new(),
        request_id: String::new(),
    }
}

fn trim_sql_statement(input: &str) -> &str {
    let mut statement = input.trim();
    while let Some(stripped) = statement.strip_suffix(';') {
        statement = stripped.trim_end();
    }
    statement
}

fn starts_with_ci(haystack: &str, prefix: &str) -> bool {
    haystack
        .get(..prefix.len())
        .map(|p| p.eq_ignore_ascii_case(prefix))
        .unwrap_or(false)
}

fn strip_prefix_ci<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    if starts_with_ci(input, prefix) {
        input.get(prefix.len()..)
    } else {
        None
    }
}

fn split_first_word(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    let idx = trimmed.find(char::is_whitespace)?;
    Some((&trimmed[..idx], trimmed[idx + 1..].trim_start()))
}

fn split_word_and_tail(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(idx) = trimmed.find(char::is_whitespace) {
        Some((&trimmed[..idx], trimmed[idx + 1..].trim_start()))
    } else {
        Some((trimmed, ""))
    }
}

fn schema_property_type_name(value: i32) -> &'static str {
    match pelago_proto::PropertyType::try_from(value) {
        Ok(pelago_proto::PropertyType::String) => "string",
        Ok(pelago_proto::PropertyType::Int) => "int",
        Ok(pelago_proto::PropertyType::Float) => "float",
        Ok(pelago_proto::PropertyType::Bool) => "bool",
        Ok(pelago_proto::PropertyType::Timestamp) => "timestamp",
        Ok(pelago_proto::PropertyType::Bytes) => "bytes",
        _ => "unknown",
    }
}

fn schema_index_type_name(value: Option<i32>) -> &'static str {
    let raw = value.unwrap_or(pelago_proto::IndexType::None as i32);
    match pelago_proto::IndexType::try_from(raw) {
        Ok(pelago_proto::IndexType::None) => "none",
        Ok(pelago_proto::IndexType::Unique) => "unique",
        Ok(pelago_proto::IndexType::Equality) => "equality",
        Ok(pelago_proto::IndexType::Range) => "range",
        _ => "none",
    }
}

fn schema_edge_target_name(target: &Option<pelago_proto::EdgeTarget>) -> String {
    match target
        .as_ref()
        .and_then(|t| t.kind.as_ref())
        .cloned()
        .unwrap_or(pelago_proto::edge_target::Kind::Polymorphic(true))
    {
        pelago_proto::edge_target::Kind::SpecificType(name) => name,
        pelago_proto::edge_target::Kind::Polymorphic(_) => "*".to_string(),
    }
}

fn schema_direction_name(value: i32) -> &'static str {
    match pelago_proto::EdgeDirectionDef::try_from(value) {
        Ok(pelago_proto::EdgeDirectionDef::Outgoing) => "outgoing",
        Ok(pelago_proto::EdgeDirectionDef::Bidirectional) => "bidirectional",
        _ => "outgoing",
    }
}

fn schema_ownership_name(value: i32) -> &'static str {
    match pelago_proto::OwnershipMode::try_from(value) {
        Ok(pelago_proto::OwnershipMode::SourceSite) => "source_site",
        Ok(pelago_proto::OwnershipMode::Independent) => "independent",
        _ => "source_site",
    }
}

fn schema_to_json(schema: &EntitySchema) -> serde_json::Value {
    let props: serde_json::Map<String, serde_json::Value> = schema
        .properties
        .iter()
        .map(|(name, def)| {
            (
                name.clone(),
                serde_json::json!({
                    "type": schema_property_type_name(def.r#type),
                    "required": def.required,
                    "index": schema_index_type_name(def.index),
                    "default_value": def.default_value.as_ref().map(value_to_json).unwrap_or(serde_json::Value::Null),
                }),
            )
        })
        .collect();

    let edges: serde_json::Map<String, serde_json::Value> = schema
        .edges
        .iter()
        .map(|(label, edge)| {
            let edge_props: serde_json::Map<String, serde_json::Value> = edge
                .properties
                .iter()
                .map(|(name, def)| {
                    (
                        name.clone(),
                        serde_json::json!({
                            "type": schema_property_type_name(def.r#type),
                            "required": def.required,
                            "index": schema_index_type_name(def.index),
                        }),
                    )
                })
                .collect();
            (
                label.clone(),
                serde_json::json!({
                    "target": schema_edge_target_name(&edge.target),
                    "direction": schema_direction_name(edge.direction),
                    "ownership": schema_ownership_name(edge.ownership),
                    "sort_key": if edge.sort_key.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(edge.sort_key.clone()) },
                    "properties": edge_props,
                }),
            )
        })
        .collect();

    serde_json::json!({
        "name": schema.name,
        "version": schema.version,
        "properties": props,
        "edges": edges,
        "meta": schema.meta.as_ref().map(|m| serde_json::json!({
            "allow_undeclared_edges": m.allow_undeclared_edges,
            "extras_policy": match pelago_proto::ExtrasPolicy::try_from(m.extras_policy) {
                Ok(pelago_proto::ExtrasPolicy::Reject) => "reject",
                Ok(pelago_proto::ExtrasPolicy::Allow) => "allow",
                Ok(pelago_proto::ExtrasPolicy::Warn) => "warn",
                _ => "unspecified",
            }
        })),
        "created_at": schema.created_at,
        "created_by": schema.created_by,
    })
}

fn format_sql_schema(schema: &EntitySchema, format: &OutputFormat) {
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
                let rows: Vec<Vec<String>> = schema
                    .properties
                    .iter()
                    .map(|(name, def)| {
                        vec![
                            name.clone(),
                            schema_property_type_name(def.r#type).to_string(),
                            def.required.to_string(),
                            schema_index_type_name(def.index).to_string(),
                        ]
                    })
                    .collect();
                crate::output::print_table(&headers, &rows);
            }
            if !schema.edges.is_empty() {
                let headers = vec!["Edge", "Target", "Direction", "Ownership", "Sort Key"];
                let rows: Vec<Vec<String>> = schema
                    .edges
                    .iter()
                    .map(|(label, edge)| {
                        vec![
                            label.clone(),
                            schema_edge_target_name(&edge.target),
                            schema_direction_name(edge.direction).to_string(),
                            schema_ownership_name(edge.ownership).to_string(),
                            if edge.sort_key.is_empty() {
                                "-".to_string()
                            } else {
                                edge.sort_key.clone()
                            },
                        ]
                    })
                    .collect();
                crate::output::print_table(&headers, &rows);
            }
        }
        OutputFormat::Csv => {
            let headers = vec!["Property", "Type", "Required", "Index"];
            let rows: Vec<Vec<String>> = schema
                .properties
                .iter()
                .map(|(name, def)| {
                    vec![
                        name.clone(),
                        schema_property_type_name(def.r#type).to_string(),
                        def.required.to_string(),
                        schema_index_type_name(def.index).to_string(),
                    ]
                })
                .collect();
            crate::output::print_csv(&headers, &rows);
        }
    }
}

fn format_sql_schema_list(schemas: &[EntitySchema], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = schemas.iter().map(schema_to_json).collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Name", "Version", "Properties", "Edges", "Created"];
            let rows: Vec<Vec<String>> = schemas
                .iter()
                .map(|s| {
                    vec![
                        s.name.clone(),
                        s.version.to_string(),
                        s.properties.len().to_string(),
                        s.edges.len().to_string(),
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

fn edge_node_ref_str(node_ref: &Option<NodeRef>) -> String {
    match node_ref {
        Some(r) => format!("{}:{}", r.entity_type, r.node_id),
        None => "<unknown>".to_string(),
    }
}

fn edge_to_json(edge: &Edge) -> serde_json::Value {
    let props: serde_json::Map<String, serde_json::Value> = edge
        .properties
        .iter()
        .map(|(k, v)| (k.clone(), value_to_json(v)))
        .collect();
    serde_json::json!({
        "edge_id": edge.edge_id,
        "source": edge_node_ref_str(&edge.source),
        "label": edge.label,
        "target": edge_node_ref_str(&edge.target),
        "properties": props,
        "created_at": edge.created_at,
    })
}

fn format_sql_edge(edge: &Edge, format: &OutputFormat) {
    match format {
        OutputFormat::Json => crate::output::print_json(&edge_to_json(edge)),
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Edge ID", "Source", "Label", "Target"];
            let rows = vec![vec![
                edge.edge_id.clone(),
                edge_node_ref_str(&edge.source),
                edge.label.clone(),
                edge_node_ref_str(&edge.target),
            ]];
            match format {
                OutputFormat::Table => crate::output::print_table(&headers, &rows),
                OutputFormat::Csv => crate::output::print_csv(&headers, &rows),
                _ => unreachable!(),
            }
        }
    }
}

fn format_sql_edges(edges: &[Edge], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = edges.iter().map(edge_to_json).collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Edge ID", "Source", "Label", "Target"];
            let rows: Vec<Vec<String>> = edges
                .iter()
                .map(|edge| {
                    vec![
                        edge.edge_id.clone(),
                        edge_node_ref_str(&edge.source),
                        edge.label.clone(),
                        edge_node_ref_str(&edge.target),
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

fn job_status_name(status: i32) -> &'static str {
    match JobStatus::try_from(status) {
        Ok(JobStatus::Pending) => "pending",
        Ok(JobStatus::Running) => "running",
        Ok(JobStatus::Completed) => "completed",
        Ok(JobStatus::Failed) => "failed",
        _ => "unknown",
    }
}

fn job_to_json(job: &Job) -> serde_json::Value {
    serde_json::json!({
        "job_id": job.job_id,
        "job_type": job.job_type,
        "status": job_status_name(job.status),
        "progress": job.progress,
        "created_at": job.created_at,
        "updated_at": job.updated_at,
        "error": if job.error.is_empty() { None } else { Some(&job.error) },
    })
}

fn format_sql_job(job: &Job, format: &OutputFormat) {
    match format {
        OutputFormat::Json => crate::output::print_json(&job_to_json(job)),
        OutputFormat::Table => {
            let headers = vec!["Job ID", "Type", "Status", "Progress", "Error"];
            let rows = vec![vec![
                job.job_id.clone(),
                job.job_type.clone(),
                job_status_name(job.status).to_string(),
                format!("{:.0}%", job.progress * 100.0),
                if job.error.is_empty() {
                    "-".to_string()
                } else {
                    job.error.clone()
                },
            ]];
            crate::output::print_table(&headers, &rows);
        }
        OutputFormat::Csv => {
            let headers = vec!["Job ID", "Type", "Status", "Progress", "Error"];
            let rows = vec![vec![
                job.job_id.clone(),
                job.job_type.clone(),
                job_status_name(job.status).to_string(),
                format!("{:.0}%", job.progress * 100.0),
                job.error.clone(),
            ]];
            crate::output::print_csv(&headers, &rows);
        }
    }
}

fn format_sql_jobs(jobs: &[Job], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = jobs.iter().map(job_to_json).collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Job ID", "Type", "Status", "Progress", "Error"];
            let rows: Vec<Vec<String>> = jobs
                .iter()
                .map(|job| {
                    vec![
                        job.job_id.clone(),
                        job.job_type.clone(),
                        job_status_name(job.status).to_string(),
                        format!("{:.0}%", job.progress * 100.0),
                        if job.error.is_empty() {
                            "-".to_string()
                        } else {
                            job.error.clone()
                        },
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

fn format_sql_sites(sites: &[pelago_proto::SiteInfo], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = sites
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "site_id": s.site_id,
                        "site_name": s.site_name,
                        "status": s.status,
                        "claimed_at": s.claimed_at,
                    })
                })
                .collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Site ID", "Name", "Status", "Claimed At"];
            let rows: Vec<Vec<String>> = sites
                .iter()
                .map(|s| {
                    vec![
                        s.site_id.clone(),
                        s.site_name.clone(),
                        s.status.clone(),
                        s.claimed_at.to_string(),
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

fn format_sql_replication_status(
    peers: &[pelago_proto::ReplicationPeerStatus],
    format: &OutputFormat,
) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = peers
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "remote_site_id": p.remote_site_id,
                        "last_applied_versionstamp": format!("{:x?}", p.last_applied_versionstamp),
                        "lag_events": p.lag_events,
                        "updated_at": p.updated_at,
                    })
                })
                .collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Remote Site", "Lag Events", "Updated At"];
            let rows: Vec<Vec<String>> = peers
                .iter()
                .map(|p| {
                    vec![
                        p.remote_site_id.clone(),
                        p.lag_events.to_string(),
                        p.updated_at.to_string(),
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

fn format_sql_namespace_settings(
    resp: &pelago_proto::GetNamespaceSettingsResponse,
    context: &ReplContext,
) {
    let settings = resp.settings.as_ref();
    let owner = settings
        .map(|s| {
            if s.schema_owner_site_id.is_empty() {
                "<unrestricted>".to_string()
            } else {
                s.schema_owner_site_id.clone()
            }
        })
        .unwrap_or_else(|| "<unrestricted>".to_string());
    let epoch = settings.map(|s| s.epoch).unwrap_or_default();
    let updated_at = settings.map(|s| s.updated_at).unwrap_or_default();

    match context.format {
        OutputFormat::Json => {
            crate::output::print_json(&serde_json::json!({
                "database": context.database,
                "namespace": context.namespace,
                "configured": resp.found,
                "schema_owner_site_id": if owner == "<unrestricted>" { serde_json::Value::Null } else { serde_json::Value::String(owner.clone()) },
                "epoch": epoch,
                "updated_at": updated_at,
            }));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec![
                "Database",
                "Namespace",
                "Configured",
                "Schema Owner",
                "Epoch",
                "Updated At",
            ];
            let rows = vec![vec![
                context.database.clone(),
                context.namespace.clone(),
                resp.found.to_string(),
                owner,
                epoch.to_string(),
                updated_at.to_string(),
            ]];
            match context.format {
                OutputFormat::Table => crate::output::print_table(&headers, &rows),
                OutputFormat::Csv => crate::output::print_csv(&headers, &rows),
                _ => unreachable!(),
            }
        }
    }
}

fn print_help() {
    println!("REPL Meta Commands:");
    println!("  :explain <PQL>   Show query execution plan");
    println!("  :use <namespace> Switch namespace");
    println!("  :db <database>   Switch database");
    println!("  :format <fmt>    Set output format (json, table, csv)");
    println!("  :set             Show UX settings");
    println!("  :set completion <fuzzy|prefix>");
    println!("  :set color <on|off>");
    println!("  :param $n = val  Set a query parameter");
    println!("  :params          List current parameters");
    println!("  :clear           Clear all parameters");
    println!("  :history         Show session history");
    println!("  :help            Show this help");
    println!("  :quit            Exit the REPL");
    println!();
    println!("SQL-like Commands:");
    println!("  USE <namespace>");
    println!("  USE DATABASE <database>");
    println!("  SHOW SCHEMAS");
    println!("  DESCRIBE <entity_type>");
    println!("  CREATE TYPE ... ;");
    println!("  DROP TYPE <entity_type>");
    println!("  INSERT INTO <Type> VALUES {{...json...}}");
    println!("  UPDATE <Type:id> SET {{...json...}}");
    println!("  DELETE FROM <Type:id>");
    println!("  CREATE EDGE <SrcType:id> <LABEL> <DstType:id> [VALUES {{...json...}}]");
    println!("  CREATE EDGE <SrcType:id> -[<LABEL>]-> <DstType:id> [VALUES {{...json...}}]");
    println!("  DELETE EDGE <SrcType:id> <LABEL> <DstType:id>");
    println!("  SHOW EDGES <Type:id> [DIRECTION out|in|both] [LABEL <label>] [LIMIT n]");
    println!("  SELECT * FROM <Type> [WHERE <CEL>] [LIMIT n]");
    println!("  TRAVERSE [FROM] <Type:id> [VIA] <label> [DEPTH n] [MAX_RESULTS n]");
    println!("  SHOW JOBS | SHOW JOB <id> | SHOW SITES");
    println!("  SHOW REPLICATION STATUS | SHOW NAMESPACE SETTINGS");
    println!("  SET NAMESPACE OWNER <site_id> | CLEAR NAMESPACE OWNER");
    println!();
    println!("PQL Examples:");
    println!("  Person {{ name, age }}");
    println!("  Person @filter(age >= 30) {{ name, age }}");
    println!("  query {{ start(func: uid(Person:1_42)) {{ name, -[KNOWS]-> {{ name }} }} }}");
    println!();
    println!("Editor Tips:");
    println!("  Tab              Context-aware completion");
    println!("  Ctrl-R           Reverse-search history");
    println!("  Enter            Auto-continues open braces/quotes");
    println!("  Default completion mode is fuzzy");
    println!("  Use ':set completion prefix' for strict prefix matching");
}

/// Check if input needs continuation.
fn needs_continuation(input: &str) -> bool {
    ux::needs_continuation(input)
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

fn execute_explain_input(input: &str, context: &ReplContext, schemas: &InMemorySchemaProvider) {
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
                        snapshot_mode: pelago_proto::SnapshotMode::Strict as i32,
                        allow_degrade_to_best_effort: false,
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
                        snapshot_mode: pelago_proto::SnapshotMode::BestEffort as i32,
                        allow_degrade_to_best_effort: true,
                    })
                    .await
                {
                    Ok(resp) => {
                        let mut stream = resp.into_inner();
                        let mut results = Vec::new();
                        while let Ok(Some(result)) = stream.message().await {
                            results.push(result);
                        }
                        let block_nodes: Vec<Node> =
                            results.iter().filter_map(|r| r.node.clone()).collect();
                        total_results += block_nodes.len();
                        format_traverse_results(&results, block_name, &context.format);
                        if let Some(var_name) = capture_by_block.get(block_name) {
                            store_capture(var_name, &block_nodes, context, &mut captured_variables);
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
                limit,
                offset,
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

                nodes = apply_offset_limit(nodes, *offset, *limit);
                total_results += nodes.len();
                format_result_nodes(&nodes, block_name, &context.format);
                if let Some(var_name) = capture_by_block.get(block_name) {
                    store_capture(var_name, &nodes, context, &mut captured_variables);
                }
            }
            CompiledBlock::VariableSet {
                block_name,
                variables,
                set_op,
                filter,
                fields,
                limit,
                offset,
            } => {
                let refs = merge_variable_refs(&captured_variables, variables, set_op);
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

                nodes = apply_offset_limit(nodes, *offset, *limit);
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

fn apply_offset_limit(mut nodes: Vec<Node>, offset: Option<u32>, limit: Option<u32>) -> Vec<Node> {
    let skip = offset.unwrap_or(0) as usize;
    if skip > 0 {
        nodes = nodes.into_iter().skip(skip).collect();
    }
    if let Some(max) = limit {
        nodes = nodes.into_iter().take(max as usize).collect();
    }
    nodes
}

fn merge_variable_refs(
    captured_variables: &HashMap<String, Vec<NodeRef>>,
    variable_names: &[String],
    set_op: &SetOp,
) -> Vec<NodeRef> {
    if variable_names.is_empty() {
        return Vec::new();
    }

    let first = captured_variables
        .get(&variable_names[0])
        .cloned()
        .unwrap_or_default();

    match set_op {
        SetOp::Union => {
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            for name in variable_names {
                if let Some(nodes) = captured_variables.get(name) {
                    for node in nodes {
                        let key = node_ref_identity(node);
                        if seen.insert(key) {
                            out.push(node.clone());
                        }
                    }
                }
            }
            out
        }
        SetOp::Intersect => {
            if variable_names.len() == 1 {
                return first;
            }

            let other_sets: Vec<HashSet<(String, String)>> = variable_names[1..]
                .iter()
                .map(|name| {
                    captured_variables
                        .get(name)
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .map(node_ref_identity)
                        .collect()
                })
                .collect();

            first
                .into_iter()
                .filter(|node| {
                    let key = node_ref_identity(node);
                    other_sets.iter().all(|s| s.contains(&key))
                })
                .collect()
        }
        SetOp::Difference => {
            if variable_names.len() == 1 {
                return first;
            }

            let mut excluded: HashSet<(String, String)> = HashSet::new();
            for name in &variable_names[1..] {
                if let Some(nodes) = captured_variables.get(name) {
                    for node in nodes {
                        excluded.insert(node_ref_identity(node));
                    }
                }
            }

            first
                .into_iter()
                .filter(|node| !excluded.contains(&node_ref_identity(node)))
                .collect()
        }
    }
}

fn node_ref_identity(node_ref: &NodeRef) -> (String, String) {
    (node_ref.entity_type.clone(), node_ref.node_id.clone())
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
) -> Result<(InMemorySchemaProvider, ReplSchemaCatalog), Box<dyn std::error::Error>> {
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
    let mut catalog = ReplSchemaCatalog::default();
    catalog.entity_types = Vec::with_capacity(resp.schemas.len());
    for schema in &resp.schemas {
        catalog.entity_types.push(schema.name.clone());

        let fields = dedupe_sorted_case_insensitive(schema.properties.keys().cloned().collect());
        let edges = dedupe_sorted_case_insensitive(schema.edges.keys().cloned().collect());
        catalog
            .fields_by_entity
            .insert(schema.name.clone(), fields.clone());
        catalog
            .edges_by_entity
            .insert(schema.name.clone(), edges.clone());

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
    catalog.entity_types = dedupe_sorted_case_insensitive(catalog.entity_types);

    Ok((provider, catalog))
}

fn dedupe_sorted_case_insensitive(mut values: Vec<String>) -> Vec<String> {
    values.sort_by_key(|s| s.to_ascii_lowercase());
    values.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    values
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

fn format_traverse_results(results: &[TraverseResult], block_name: &str, format: &OutputFormat) {
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

    fn test_context() -> ReplContext {
        ReplContext {
            database: "default".to_string(),
            namespace: "default".to_string(),
            format: OutputFormat::Table,
            params: HashMap::new(),
            ux: ReplUxSettings::default(),
        }
    }

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
        assert!(needs_continuation("query { start(func: type(Person)) {"));
        assert!(needs_continuation("CREATE TYPE Person (name STRING)"));
        assert!(!needs_continuation("Person { name }"));
        assert!(!needs_continuation(
            "query { start(func: type(Person)) { name } }"
        ));
        assert!(!needs_continuation("CREATE TYPE Person (name STRING);"));
        assert!(!needs_continuation(":help"));
    }

    #[test]
    fn test_parse_set_command() {
        assert!(matches!(
            parse_set_command(None).unwrap(),
            ReplSetCommand::Show
        ));
        assert!(matches!(
            parse_set_command(Some("completion fuzzy")).unwrap(),
            ReplSetCommand::Completion(ReplCompletionMode::Fuzzy)
        ));
        assert!(matches!(
            parse_set_command(Some("completion prefix")).unwrap(),
            ReplSetCommand::Completion(ReplCompletionMode::Prefix)
        ));
        assert!(matches!(
            parse_set_command(Some("color off")).unwrap(),
            ReplSetCommand::Color(ReplColorMode::Off)
        ));
    }

    #[test]
    fn test_parse_set_command_errors() {
        let err = parse_set_command(Some("completion nope")).unwrap_err();
        assert!(err.contains("Unknown completion mode"));

        let err = parse_set_command(Some("color maybe")).unwrap_err();
        assert!(err.contains("Unknown color mode"));

        let err = parse_set_command(Some("unknown foo")).unwrap_err();
        assert!(err.contains("Supported keys"));
    }

    #[test]
    fn test_set_meta_command_updates_context() {
        let mut context = test_context();
        let history: Vec<String> = Vec::new();

        handle_meta_command(":set completion prefix", &mut context, &history);
        assert_eq!(context.ux.completion_mode, ReplCompletionMode::Prefix);

        handle_meta_command(":set completion fuzzy", &mut context, &history);
        assert_eq!(context.ux.completion_mode, ReplCompletionMode::Fuzzy);

        handle_meta_command(":set color off", &mut context, &history);
        assert_eq!(context.ux.color_mode, ReplColorMode::Off);
    }

    #[test]
    fn test_parse_sql_select_command() {
        let parsed = parse_sql_command("SELECT * FROM Person WHERE age >= 30 LIMIT 25");
        match parsed {
            Some(Ok(SqlCommand::SelectNodes {
                entity_type,
                filter,
                limit,
            })) => {
                assert_eq!(entity_type, "Person");
                assert_eq!(filter, "age >= 30");
                assert_eq!(limit, 25);
            }
            _ => panic!("expected select command"),
        }
    }

    #[test]
    fn test_parse_sql_insert_command() {
        let parsed = parse_sql_command("INSERT INTO Person VALUES {\"name\":\"Alice\",\"age\":31}");
        match parsed {
            Some(Ok(SqlCommand::InsertNode {
                entity_type,
                properties,
            })) => {
                assert_eq!(entity_type, "Person");
                assert!(properties.contains_key("name"));
                assert!(properties.contains_key("age"));
            }
            _ => panic!("expected insert command"),
        }
    }

    #[test]
    fn test_parse_sql_create_type_command() {
        let parsed = parse_sql_command("CREATE TYPE Person (name STRING REQUIRED);");
        match parsed {
            Some(Ok(SqlCommand::CreateSchema(schema))) => {
                assert_eq!(schema.name, "Person");
                assert!(schema.properties.contains_key("name"));
            }
            _ => panic!("expected create schema command"),
        }
    }

    #[test]
    fn test_parse_sql_create_edge_command() {
        let parsed = parse_sql_command(
            "CREATE EDGE Person:1_42 -[WORKS_AT]-> Company:1_7 VALUES {\"since\":2020}",
        );
        match parsed {
            Some(Ok(SqlCommand::CreateEdge {
                source,
                label,
                target,
                properties,
            })) => {
                assert_eq!(source.entity_type, "Person");
                assert_eq!(source.node_id, "1_42");
                assert_eq!(label, "WORKS_AT");
                assert_eq!(target.entity_type, "Company");
                assert_eq!(target.node_id, "1_7");
                assert!(properties.contains_key("since"));
            }
            _ => panic!("expected create edge command"),
        }
    }

    #[test]
    fn test_parse_sql_show_edges_command() {
        let parsed =
            parse_sql_command("SHOW EDGES Person:1_42 DIRECTION both LABEL WORKS_AT LIMIT 25");
        match parsed {
            Some(Ok(SqlCommand::ShowEdges {
                entity_type,
                node_id,
                direction,
                label,
                limit,
            })) => {
                assert_eq!(entity_type, "Person");
                assert_eq!(node_id, "1_42");
                assert_eq!(direction, EdgeDirection::Both as i32);
                assert_eq!(label.as_deref(), Some("WORKS_AT"));
                assert_eq!(limit, 25);
            }
            _ => panic!("expected show edges command"),
        }
    }

    #[test]
    fn test_parse_sql_traverse_command() {
        let parsed =
            parse_sql_command("TRAVERSE FROM Person:1_42 VIA WORKS_AT DEPTH 2 MAX_RESULTS 10");
        match parsed {
            Some(Ok(SqlCommand::Traverse {
                start,
                edge_type,
                max_depth,
                max_results,
            })) => {
                assert_eq!(start.entity_type, "Person");
                assert_eq!(start.node_id, "1_42");
                assert_eq!(edge_type, "WORKS_AT");
                assert_eq!(max_depth, 2);
                assert_eq!(max_results, 10);
            }
            _ => panic!("expected traverse command"),
        }
    }
}
