use bgci::common::parse_variant;
use clap::Args;

use crate::config::{load_toml, resolve_engine_shortcuts, DuelConfig, EngineConfig};
use crate::domain::MatchPlan;
use crate::managed::{default_db_path, ManagedRunStore};

#[derive(Debug, Args)]
pub struct SubmitArgs {
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

    #[arg(long)]
    ubgi_log: Option<String>,

    #[arg(long)]
    db: Option<String>,

    #[arg(long = "run-name")]
    run_name: Option<String>,
}

pub fn run(args: SubmitArgs) -> Result<(), String> {
    let mut cfg = build_duel_config(&args)?;
    resolve_engine_shortcuts(&mut cfg)?;
    let variant = parse_variant(&cfg.variant)?;

    let plan = MatchPlan {
        config: cfg,
        variant,
    };

    let db_path = args
        .db
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_db_path);
    let mut store = ManagedRunStore::open(&db_path)?;
    let run_id = store.create_run(&plan, args.run_name.as_deref())?;
    println!("run_id -> {run_id}");
    println!("queued  -> {}", db_path.display());
    println!("note    -> run `bgci serve` to execute queued runs (server mode WIP)");
    Ok(())
}

fn build_duel_config(args: &SubmitArgs) -> Result<DuelConfig, String> {
    if let Some(config_path) = &args.config {
        let mut cfg: DuelConfig = load_toml(config_path)?;
        if let Some(games) = args.games {
            cfg.games = games;
        }
        if let Some(parallel) = args.parallel {
            cfg.parallel = parallel.max(1);
        }
        if let Some(ubgi_log) = &args.ubgi_log {
            cfg.ubgi_log = ubgi_log.clone();
        }
        return Ok(cfg);
    }

    if args.engine_a.is_none() && args.engine_b.is_none() {
        return Err(
            "submit requires either --config or both --engine-a and --engine-b".to_string(),
        );
    }

    let engine_a = args
        .engine_a
        .as_ref()
        .ok_or_else(|| "missing --engine-a (or use --config)".to_string())?;
    let engine_b = args
        .engine_b
        .as_ref()
        .ok_or_else(|| "missing --engine-b (or use --config)".to_string())?;

    let mut cfg = DuelConfig::default();
    cfg.engine_a = EngineConfig {
        name: engine_a.clone(),
        engine: Some(engine_a.clone()),
        command: Vec::new(),
        env: Default::default(),
    };
    cfg.engine_b = EngineConfig {
        name: engine_b.clone(),
        engine: Some(engine_b.clone()),
        command: Vec::new(),
        env: Default::default(),
    };
    if let Some(games) = args.games {
        cfg.games = games;
    }
    if let Some(parallel) = args.parallel {
        cfg.parallel = parallel.max(1);
    }
    if let Some(ubgi_log) = &args.ubgi_log {
        cfg.ubgi_log = ubgi_log.clone();
    }
    Ok(cfg)
}
