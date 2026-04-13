use clap::{Parser, Subcommand};

use commands::{CheckArgs, DuelArgs, EngineArgs, RunsArgs, SubmitArgs};

mod checker;
mod commands;
mod config;
mod domain;
mod duel_runner;
mod engine;
mod executor;
mod logging;
mod managed;
mod output_paths;
mod report;
mod status_display;
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
    Submit(SubmitArgs),
    Check(CheckArgs),
    Engine(EngineArgs),
    Runs(RunsArgs),
}

fn main() -> Result<(), String> {
    let args = CliArgs::parse();
    match args.command {
        Commands::Duel(duel) => commands::duel::run(duel),
        Commands::Submit(submit) => commands::submit::run(submit),
        Commands::Check(check) => commands::check::run(check),
        Commands::Engine(engine) => commands::engine::run(engine),
        Commands::Runs(runs) => commands::runs::run(runs),
    }
}
