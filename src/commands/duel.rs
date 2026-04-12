use bgci::common::parse_variant;
use clap::Args;
use tracing::info;

use crate::config::{load_toml, resolve_engine_shortcuts, DuelConfig, EngineConfig};
use crate::duel_runner::run_duel;
use crate::logging;
use crate::output_paths::build_run_paths;

#[derive(Debug, Args)]
pub struct DuelArgs {
    #[arg(long)]
    config: Option<String>,

    #[arg(long = "engine-a")]
    engine_a: Option<String>,

    #[arg(long = "engine-b")]
    engine_b: Option<String>,

    #[arg(long)]
    games: Option<usize>,

    #[arg(long)]
    parallel: Option<usize>,
}

pub fn run(args: DuelArgs) -> Result<(), String> {
    let mut cfg = build_duel_config(args)?;
    resolve_engine_shortcuts(&mut cfg)?;

    let run_paths = build_run_paths(&cfg.engine_a.name, &cfg.engine_b.name);
    let _log_guard = logging::init_tracing(&cfg.log, &run_paths.log_file)?;
    let variant = parse_variant(&cfg.variant)?;

    info!(
        run = %run_paths.timestamp,
        log = %cfg.log,
        log_path = %run_paths.log_file.display(),
        output_csv = %run_paths.output_csv.display(),
        games = cfg.games,
        parallel = cfg.parallel,
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

fn build_duel_config(args: DuelArgs) -> Result<DuelConfig, String> {
    if let Some(config_path) = args.config {
        let mut cfg: DuelConfig = load_toml(config_path)?;
        if let Some(games) = args.games {
            cfg.games = games;
        }
        if let Some(parallel) = args.parallel {
            cfg.parallel = parallel.max(1);
        }
        return Ok(cfg);
    }

    if args.engine_a.is_none() && args.engine_b.is_none() {
        return Err("duel requires either --config or both --engine-a and --engine-b".to_string());
    }

    let engine_a = args
        .engine_a
        .ok_or_else(|| "missing --engine-a (or use --config)".to_string())?;
    let engine_b = args
        .engine_b
        .ok_or_else(|| "missing --engine-b (or use --config)".to_string())?;

    let mut cfg = DuelConfig::default();
    cfg.engine_a = EngineConfig {
        name: engine_a.clone(),
        engine: Some(engine_a),
        command: Vec::new(),
        env: Default::default(),
    };
    cfg.engine_b = EngineConfig {
        name: engine_b.clone(),
        engine: Some(engine_b),
        command: Vec::new(),
        env: Default::default(),
    };
    if let Some(games) = args.games {
        cfg.games = games;
    }
    if let Some(parallel) = args.parallel {
        cfg.parallel = parallel.max(1);
    }
    Ok(cfg)
}
