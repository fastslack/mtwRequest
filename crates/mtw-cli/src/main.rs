//! `mtw` — command-line tool for mtwRequest.

use clap::{Parser, Subcommand};
use std::process::ExitCode;

mod commands;

#[derive(Parser, Debug)]
#[command(
    name = "mtw",
    version,
    about = "mtwRequest — scaffold, run, and share real-time modules",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a new mtwRequest project in a new directory
    New {
        /// Name of the project (also the directory created)
        name: String,
        /// Skip creating the mtw.toml file
        #[arg(long)]
        no_config: bool,
    },
    /// Add an `mtw.toml` to the current directory
    Init {
        /// Overwrite existing mtw.toml
        #[arg(long)]
        force: bool,
    },
    /// Run an mtwRequest server from the current directory's `mtw.toml`
    Run {
        /// Path to a config file (defaults to ./mtw.toml)
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Search the module marketplace
    Search {
        query: String,
        /// Filter by module type (middleware, agent, transport, etc.)
        #[arg(long)]
        module_type: Option<String>,
        /// Filter by author
        #[arg(long)]
        author: Option<String>,
    },
    /// Install a module from the marketplace
    Install {
        /// Module name, optionally with @version (e.g. my-mod@1.2.0)
        module: String,
    },
    /// Publish the current module to the marketplace
    Publish {
        /// Path to the module directory (defaults to .)
        #[arg(long, default_value = ".")]
        path: String,
        /// Do everything except the final upload
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,mtw=info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let result = match cli.command {
        Command::New { name, no_config } => commands::new::run(&name, no_config),
        Command::Init { force } => commands::init::run(force),
        Command::Run { config } => commands::run::run(config.as_deref()),
        Command::Search {
            query,
            module_type,
            author,
        } => commands::search::run(&query, module_type.as_deref(), author.as_deref()).await,
        Command::Install { module } => commands::install::run(&module).await,
        Command::Publish { path, dry_run } => commands::publish::run(&path, dry_run).await,
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
