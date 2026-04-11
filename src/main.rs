use bgci::common::parse_variant;
use clap::Parser;
use config::{load_toml, DuelConfig};
use duel_runner::run_duel;
use output_paths::build_run_paths;
use tracing::info;

mod config;
mod duel_runner;
mod engine;
mod logging;
mod output_paths;
mod report;
mod stats;

#[derive(Debug, Parser)]
#[command(name = "bgci", about = "UBGI dueller")]
struct CliArgs {
    #[arg(long)]
    config: String,

    #[arg(long)]
    games: Option<usize>,
}

fn main() -> Result<(), String> {
    let args = CliArgs::parse();
    let mut cfg: DuelConfig = load_toml(&args.config)?;
    if let Some(games) = args.games {
        cfg.games = games;
    }

    let run_paths = build_run_paths(&cfg.engine_a.name, &cfg.engine_b.name);

    let _log_guard = logging::init_tracing(&cfg.log, &run_paths.log_file)?;
    let variant = parse_variant(&cfg.variant)?;

    info!(
        run = %run_paths.timestamp,
        config = %args.config,
        log = %cfg.log,
        log_path = %run_paths.log_file.display(),
        output_csv = %run_paths.output_csv.display(),
        games = cfg.games,
        seed = cfg.seed,
        max_plies = cfg.max_plies,
        variant = %cfg.variant,
        engine_a = %cfg.engine_a.name,
        engine_a_cmd = %cfg.engine_a.command.join(" "),
        engine_b = %cfg.engine_b.name,
        engine_b_cmd = %cfg.engine_b.command.join(" "),
        "duel run header"
    );

    let summary = run_duel(&cfg, variant, &run_paths)?;
    println!("{}", summary.line_engines);
    println!("{}", summary.line_result);
    println!("{}", summary.line_rate);
    println!("{}", summary.line_decide);
    println!("{}", summary.line_class);
    println!("{}", summary.line_sides);
    println!("saved -> {}", run_paths.output_csv.display());
    if logging::normalize_level(&cfg.log).is_some() {
        println!("log   -> {}", run_paths.log_file.display());
    }

    info!(output_csv = %run_paths.output_csv.display(), "duel run complete");
    Ok(())
}
