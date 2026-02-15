use clap::Parser;

mod commands;
mod config;
mod connection;
mod output;
mod repl;

#[derive(Parser)]
#[command(name = "pelago", about = "PelagoDB CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Server address
    #[arg(
        short = 's',
        long,
        env = "PELAGO_SERVER",
        default_value = "http://localhost:27615",
        global = true
    )]
    server: String,

    /// Database name
    #[arg(
        short = 'd',
        long,
        env = "PELAGO_DATABASE",
        default_value = "default",
        global = true
    )]
    database: String,

    /// Namespace
    #[arg(
        short = 'n',
        long,
        env = "PELAGO_NAMESPACE",
        default_value = "default",
        global = true
    )]
    namespace: String,

    /// Output format
    #[arg(short = 'f', long, default_value = "table", global = true)]
    format: output::OutputFormat,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Schema management
    Schema(commands::schema::SchemaArgs),
    /// Node operations
    Node(commands::node::NodeArgs),
    /// Edge operations
    Edge(commands::edge::EdgeArgs),
    /// Admin operations
    Admin(commands::admin::AdminArgs),
    /// Interactive PQL REPL
    Repl(repl::ReplArgs),
    /// Show version information
    Version,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    // Load config file, merge with CLI args
    let _config = config::load_config();

    match cli.command {
        Commands::Schema(args) => {
            commands::schema::run(args, &cli.server, &cli.database, &cli.namespace, &cli.format)
                .await?
        }
        Commands::Node(args) => {
            commands::node::run(args, &cli.server, &cli.database, &cli.namespace, &cli.format)
                .await?
        }
        Commands::Edge(args) => {
            commands::edge::run(args, &cli.server, &cli.database, &cli.namespace, &cli.format)
                .await?
        }
        Commands::Admin(args) => {
            commands::admin::run(args, &cli.server, &cli.database, &cli.namespace, &cli.format)
                .await?
        }
        Commands::Repl(args) => {
            repl::run(args, &cli.server, &cli.database, &cli.namespace).await?
        }
        Commands::Version => {
            println!("pelago {}", env!("CARGO_PKG_VERSION"));
            println!("PelagoDB CLI - graph database command-line interface");
        }
    }

    Ok(())
}
