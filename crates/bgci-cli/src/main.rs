use clap::{Parser, Subcommand};

use commands::{CheckArgs, DuelArgs, EngineArgs, EvalArgs, RatingsArgs};

mod commands;
mod logging;

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
    Ratings(RatingsArgs),
    Eval(EvalArgs),
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args = CliArgs::parse();
    match args.command {
        Commands::Duel(duel) => commands::duel::run(duel).await,
        Commands::Check(check) => commands::check::run(check),
        Commands::Engine(engine) => commands::engine::run(engine),
        Commands::Ratings(ratings) => commands::ratings::run(ratings).await,
        Commands::Eval(eval) => commands::eval::run(eval).await,
    }
}
