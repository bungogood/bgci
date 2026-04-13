use bgci::common::parse_variant;
use clap::Args;
use std::path::PathBuf;
use tracing::info;

use crate::config::{load_toml, resolve_engine_shortcuts, DuelConfig, EngineConfig};
use crate::duel_runner::run_duel;
use crate::logging;
use crate::output_paths::build_run_paths;

#[derive(Debug, Args)]
pub struct DuelArgs {
    #[arg(short = 'c', long)]
    config: Option<String>,

    #[arg(short = 'a', long = "engine-a")]
    engine_a: Option<String>,

    #[arg(short = 'b', long = "engine-b")]
    engine_b: Option<String>,

    #[arg(short = 'g', long)]
    games: Option<usize>,

    #[arg(short = 'p', long)]
    parallel: Option<usize>,

    #[arg(short = 's', long)]
    seed: Option<u64>,

    #[arg(short = 'm', long = "max-plies")]
    max_plies: Option<usize>,

    #[arg(short = 't', long = "timeout-secs")]
    timeout_secs: Option<u64>,

    #[arg(short = 'v', long)]
    variant: Option<String>,

    #[arg(short = 'l', long)]
    log: Option<String>,

    #[arg(short = 'w', long = "swap-sides")]
    swap_sides: bool,

    #[arg(short = 'n', long = "no-swap-sides")]
    no_swap_sides: bool,

    #[arg(short = 'S', long)]
    save: bool,

    #[arg(short = 'o', long = "output")]
    output_csv: Option<String>,
}

pub async fn run(args: DuelArgs) -> Result<(), String> {
    let mut cfg = build_duel_config(&args)?;
    resolve_engine_shortcuts(&mut cfg)?;

    let mut run_paths = build_run_paths(&cfg.engine_a.name, &cfg.engine_b.name);
    if let Some(path) = &args.output_csv {
        run_paths.output_csv = PathBuf::from(path);
    }
    let save_results = args.save || args.output_csv.is_some();
    let _log_guard = logging::init_tracing(&cfg.log, &run_paths.log_file)?;
    let variant = parse_variant(&cfg.variant)?;

    info!(
        run = %run_paths.timestamp,
        log = %cfg.log,
        log_path = %run_paths.log_file.display(),
        save_results,
        output_csv = %run_paths.output_csv.display(),
        games = cfg.games,
        parallel = cfg.parallel,
        seed = cfg.seed,
        max_plies = cfg.max_plies,
        timeout_secs = cfg.timeout_secs,
        variant = %cfg.variant,
        engine_a = %cfg.engine_a.name,
        engine_a_cmd = %cfg.engine_a.command.join(" "),
        engine_b = %cfg.engine_b.name,
        engine_b_cmd = %cfg.engine_b.command.join(" "),
        "duel run header"
    );

    let summary = run_duel(&cfg, variant, &run_paths, save_results).await?;
    println!("{}", summary.line_engines);
    println!("{}", summary.line_result);
    println!("{}", summary.line_rate);
    println!("{}", summary.line_decide);
    println!("{}", summary.line_class);
    println!("{}", summary.line_sides);
    if save_results {
        println!("saved -> {}", run_paths.output_csv.display());
    }
    if logging::normalize_level(&cfg.log).is_some() {
        println!("log   -> {}", run_paths.log_file.display());
    }

    info!(save_results, output_csv = %run_paths.output_csv.display(), "duel run complete");
    Ok(())
}

fn build_duel_config(args: &DuelArgs) -> Result<DuelConfig, String> {
    let mut cfg = if let Some(config_path) = &args.config {
        load_toml(config_path)?
    } else {
        if args.engine_a.is_none() && args.engine_b.is_none() {
            return Err(
                "duel requires either --config or both --engine-a and --engine-b".to_string(),
            );
        }

        let engine_a = args
            .engine_a
            .clone()
            .ok_or_else(|| "missing --engine-a (or use --config)".to_string())?;
        let engine_b = args
            .engine_b
            .clone()
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
        cfg
    };

    if let Some(engine_a) = &args.engine_a {
        cfg.engine_a = EngineConfig {
            name: engine_a.clone(),
            engine: Some(engine_a.clone()),
            command: Vec::new(),
            env: Default::default(),
        };
    }
    if let Some(engine_b) = &args.engine_b {
        cfg.engine_b = EngineConfig {
            name: engine_b.clone(),
            engine: Some(engine_b.clone()),
            command: Vec::new(),
            env: Default::default(),
        };
    }

    if let Some(games) = args.games {
        cfg.games = games;
    }
    if let Some(parallel) = args.parallel {
        cfg.parallel = parallel.max(1);
    }
    if let Some(seed) = args.seed {
        cfg.seed = seed;
    }
    if let Some(max_plies) = args.max_plies {
        cfg.max_plies = max_plies.max(1);
    }
    if let Some(timeout_secs) = args.timeout_secs {
        cfg.timeout_secs = Some(timeout_secs.max(1));
    }
    if let Some(variant) = &args.variant {
        cfg.variant = variant.clone();
    }
    if let Some(log) = &args.log {
        cfg.log = log.clone();
    }
    if args.swap_sides && args.no_swap_sides {
        return Err("cannot pass both --swap-sides and --no-swap-sides".to_string());
    }
    if args.swap_sides {
        cfg.swap_sides = true;
    }
    if args.no_swap_sides {
        cfg.swap_sides = false;
    }

    Ok(cfg)
}
