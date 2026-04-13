use clap::{Parser, Subcommand};

use commands::{CheckArgs, DuelArgs, EngineArgs};

mod checker;
mod commands;
mod config;
mod duel_game;
mod duel_messages;
mod duel_runner;
mod duel_workers;
mod engine;
mod logging;
mod output_paths;
mod report;
mod stats;

#[derive(Debug, Parser)]
#[command(name = "bgci", about = "UBGI dueller")]
struct CliArgs {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Duel(DuelArgs),
    Check(CheckArgs),
    Engine(EngineArgs),
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args = CliArgs::parse();
    match args.command {
        Commands::Duel(duel) => commands::duel::run(duel).await,
        Commands::Check(check) => commands::check::run(check),
        Commands::Engine(engine) => commands::engine::run(engine),
    }
}
