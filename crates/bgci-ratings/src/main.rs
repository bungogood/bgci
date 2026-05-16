use bgci_core::ratings::{RunRatingsArgs, run_ratings};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "bgci-ratings", about = "Ratings and pairing scheduler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Run(RunRatingsArgs),
}

#[tokio::main]
async fn main() -> Result<(), String> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Commands::Run(args) => run_ratings(args).await,
    }
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|raw| EnvFilter::try_new(raw).ok())
        .unwrap_or_else(|| EnvFilter::new("warn,bgci_core::ratings=info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(true)
        .compact()
        .try_init();
}
