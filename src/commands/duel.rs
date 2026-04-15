use bgci::common::parse_variant;
use clap::Args;
use std::path::PathBuf;
use tracing::info;

use crate::config::{DuelConfig, EngineConfig, load_toml, resolve_engine_shortcuts};
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

    #[arg(short = 'S', long, help = "Enable default CSV and trace outputs")]
    save: bool,

    #[arg(short = 'o', long = "output", visible_alias = "output-csv")]
    output_csv: Option<String>,

    #[arg(long = "output-mat")]
    output_mat: Option<String>,

    #[arg(long = "output-log")]
    output_log: Option<String>,

    #[arg(long = "output-traces")]
    output_traces_dir: Option<String>,
}

pub async fn run(args: DuelArgs) -> Result<(), String> {
    let mut cfg = build_duel_config(&args)?;
    resolve_engine_shortcuts(&mut cfg)?;

    let mut run_paths = build_run_paths(&cfg.engine_a.name, &cfg.engine_b.name);
    if let Some(path) = &cfg.output_csv {
        run_paths.output_csv = PathBuf::from(path);
    }
    if let Some(path) = &cfg.output_mat {
        run_paths.output_mat = PathBuf::from(path);
    }
    if let Some(path) = &cfg.output_log {
        run_paths.log_file = PathBuf::from(path);
    }
    if let Some(path) = &cfg.output_traces_dir {
        run_paths.trace_games_dir = PathBuf::from(path);
    }

    let save_csv = args.save || cfg.output_csv.is_some();
    let save_traces = args.save || cfg.output_traces_dir.is_some();
    let save_mat = cfg.output_mat.is_some();
    let save_log = logging::normalize_level(&cfg.log).is_some() && cfg.output_log.is_some();

    let _log_guard = if save_log {
        logging::init_tracing(&cfg.log, &run_paths.log_file)?
    } else {
        None
    };
    let variant = parse_variant(&cfg.variant)?;

    info!(
        run = %run_paths.timestamp,
        log = %cfg.log,
        log_path = %run_paths.log_file.display(),
        save_csv,
        save_traces,
        save_mat,
        save_log,
        output_csv = %run_paths.output_csv.display(),
        output_mat = %run_paths.output_mat.display(),
        output_log = %run_paths.log_file.display(),
        output_traces = %run_paths.trace_games_dir.display(),
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

    let summary = run_duel(&cfg, variant, &run_paths, save_csv, save_traces, save_mat).await?;
    println!("{}", summary.line_engines);
    println!("{}", summary.line_result);
    println!("{}", summary.line_rate);
    println!("{}", summary.line_decide);
    println!("{}", summary.line_class);
    println!("{}", summary.line_sides);
    if save_csv {
        println!("saved -> {}", run_paths.output_csv.display());
    }
    if save_mat {
        println!("mat   -> {}", run_paths.output_mat.display());
    }
    if save_traces {
        println!("trace -> {}", run_paths.trace_games_dir.display());
    }
    if save_log {
        println!("log   -> {}", run_paths.log_file.display());
    }

    info!(
        save_csv,
        save_traces,
        save_mat,
        save_log,
        output_csv = %run_paths.output_csv.display(),
        output_mat = %run_paths.output_mat.display(),
        output_log = %run_paths.log_file.display(),
        output_traces = %run_paths.trace_games_dir.display(),
        "duel run complete"
    );
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
    if let Some(output_csv) = &args.output_csv {
        cfg.output_csv = Some(output_csv.clone());
    }
    if let Some(output_mat) = &args.output_mat {
        cfg.output_mat = Some(output_mat.clone());
    }
    if let Some(output_log) = &args.output_log {
        cfg.output_log = Some(output_log.clone());
    }
    if let Some(output_traces_dir) = &args.output_traces_dir {
        cfg.output_traces_dir = Some(output_traces_dir.clone());
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
