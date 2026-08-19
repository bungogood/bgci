use clap::{Parser, Subcommand};

use commands::{CheckArgs, DuelArgs, EngineArgs, HistoryArgs, LeagueArgs};

mod commands;
mod logging;

#[derive(Debug, Parser)]
#[command(
    name = "bgci",
    version,
    about = "Reproducible backgammon engine testing over UBGI"
)]
struct CliArgs {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a mirrored two-engine benchmark, optionally saving it.
    Duel(DuelArgs),
    /// Run and save a round-robin multi-engine benchmark.
    League(LeagueArgs),
    /// Inspect saved duels and leagues.
    History(HistoryArgs),
    /// Validate an engine's UBGI behavior.
    Check(CheckArgs),
    /// List configured engines or run a built-in engine adapter.
    Engine(EngineArgs),
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args = CliArgs::parse();
    match args.command {
        Commands::Duel(duel) => commands::duel::run(duel).await,
        Commands::League(league) => commands::league::run(league).await,
        Commands::History(history) => commands::history::run(history),
        Commands::Check(check) => commands::check::run(check),
        Commands::Engine(engine) => commands::engine::run(engine),
    }
}
