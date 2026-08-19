use std::path::PathBuf;

use bgci_core::benchmark::{
    BenchmarkSpec, BenchmarkStore, MatchupHandle, default_benchmark_db_path,
};
use bgci_core::common::parse_variant;
use bgci_core::config::{
    MatchupConfig, engine_identity_from_spec_with_options, load_toml, resolve_engine_shortcuts,
    resolve_engine_spec,
};
use bgci_core::duel_runner::run_matchup;
use bgci_core::engine::filter_supported_engine_options;
use clap::Args;
use tracing::info;

use crate::logging;

#[derive(Debug, Args)]
pub struct DuelArgs {
    /// Load matchup and engine settings from TOML.
    #[arg(short = 'c', long)]
    config: Option<String>,

    /// First engine alias or option-qualified specification.
    #[arg(short = 'a', long = "engine-a")]
    engine_a: Option<String>,

    /// Second engine alias or option-qualified specification.
    #[arg(short = 'b', long = "engine-b")]
    engine_b: Option<String>,

    /// Number of mirrored pairs; each pair contains two games.
    #[arg(long)]
    pairs: Option<usize>,

    /// Number of pair workers to run concurrently.
    #[arg(short = 'p', long)]
    parallel: Option<usize>,

    /// Base seed used to derive deterministic pair dice streams.
    #[arg(short = 's', long)]
    seed: Option<u64>,

    /// Maximum plies before recording a game as incomplete.
    #[arg(short = 'm', long = "max-plies")]
    max_plies: Option<usize>,

    /// Backgammon variant used by both games in every pair.
    #[arg(long)]
    variant: Option<String>,

    /// Tracing level: off, error, warn, info, debug, or trace.
    #[arg(long = "log-level")]
    log_level: Option<String>,

    /// Write tracing output to this file instead of stderr.
    #[arg(long, requires = "log_level")]
    log_file: Option<PathBuf>,

    /// Save this duel to the benchmark database.
    #[arg(long)]
    save: bool,

    /// Human-readable name for a saved duel.
    #[arg(long, requires = "save")]
    name: Option<String>,

    /// Benchmark database path; defaults to the XDG data directory.
    #[arg(long = "db", requires = "save")]
    db_path: Option<PathBuf>,

    #[arg(long = "ply")]
    ply: Option<usize>,

    #[arg(long = "ply-a")]
    ply_a: Option<usize>,

    #[arg(long = "ply-b")]
    ply_b: Option<usize>,
}

pub async fn run(args: DuelArgs) -> Result<(), String> {
    let mut cfg = build_matchup_config(&args)?;
    resolve_engine_shortcuts(&mut cfg)?;
    cfg.engine_a = filter_supported_engine_options(&cfg.engine_a);
    cfg.engine_b = filter_supported_engine_options(&cfg.engine_b);
    if let Some(spec) = &args.engine_a {
        cfg.engine_a.name = engine_identity_from_spec_with_options(spec, &cfg.engine_a.options)?;
    }
    if let Some(spec) = &args.engine_b {
        cfg.engine_b.name = engine_identity_from_spec_with_options(spec, &cfg.engine_b.options)?;
    }

    let _log_guard = logging::init_tracing(&cfg.log_level, args.log_file.as_deref())?;
    let variant = parse_variant(&cfg.variant)?;

    info!(
        log_level = %cfg.log_level,
        pairs = cfg.pairs,
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

    let mut saved = if args.save {
        Some(SavedDuel::start(
            args.db_path
                .clone()
                .unwrap_or_else(default_benchmark_db_path),
            args.name.as_deref(),
            &cfg,
        )?)
    } else {
        None
    };
    let run = match run_matchup(&cfg, variant).await {
        Ok(run) => run,
        Err(error) => {
            if let Some(saved) = &saved {
                return Err(mark_failed(&saved.store, saved.id, error));
            }
            return Err(error);
        }
    };
    if let Some(saved) = &mut saved {
        if let Err(error) = saved.complete(&run.games) {
            return Err(mark_failed(&saved.store, saved.id, error));
        }
        println!(
            "saved benchmark {} -> {}",
            saved.id,
            saved.db_path.display()
        );
    }
    let summary = run.summary;
    println!("{}", summary.line_engines);
    println!("{}", summary.line_result);
    println!("{}", summary.line_rate);
    println!("{}", summary.line_decide);
    println!("{}", summary.line_class);
    println!("{}", summary.line_sides);
    if let Some(log_file) = &args.log_file {
        println!("log   -> {}", log_file.display());
    }

    info!(games = run.games.len(), "duel run complete");
    Ok(())
}

fn build_matchup_config(args: &DuelArgs) -> Result<MatchupConfig, String> {
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

        let (engine_a_key, mut engine_a_cfg) = resolve_engine_spec(&engine_a)?;
        engine_a_cfg.name = engine_a_key;
        let (engine_b_key, mut engine_b_cfg) = resolve_engine_spec(&engine_b)?;
        engine_b_cfg.name = engine_b_key;

        MatchupConfig {
            engine_a: engine_a_cfg,
            engine_b: engine_b_cfg,
            ..MatchupConfig::default()
        }
    };

    if let Some(engine_a) = &args.engine_a {
        let (engine_a_key, mut engine_a_cfg) = resolve_engine_spec(engine_a)?;
        engine_a_cfg.name = engine_a_key;
        cfg.engine_a = engine_a_cfg;
    }
    if let Some(engine_b) = &args.engine_b {
        let (engine_b_key, mut engine_b_cfg) = resolve_engine_spec(engine_b)?;
        engine_b_cfg.name = engine_b_key;
        cfg.engine_b = engine_b_cfg;
    }

    cfg.pairs = match args.pairs {
        Some(0) => return Err("--pairs must be greater than zero".to_string()),
        Some(pairs) => pairs,
        None => cfg.pairs,
    };
    cfg.pairs
        .checked_mul(2)
        .ok_or_else(|| "pair count is too large".to_string())?;
    if let Some(parallel) = args.parallel {
        cfg.parallel = parallel.max(1);
    }
    if let Some(seed) = args.seed {
        cfg.seed = seed;
    }
    if let Some(max_plies) = args.max_plies {
        cfg.max_plies = max_plies.max(1);
    }
    if let Some(variant) = &args.variant {
        cfg.variant = variant.clone();
    }
    if let Some(log_level) = &args.log_level {
        cfg.log_level = log_level.clone();
    }
    let ply_a = args.ply_a.or(args.ply);
    let ply_b = args.ply_b.or(args.ply);
    if let Some(ply) = ply_a {
        if ply < 1 {
            return Err("--ply-a/--ply must be >= 1".to_string());
        }
        cfg.engine_a
            .options
            .insert("engine.ply".to_string(), ply.to_string());
    }
    if let Some(ply) = ply_b {
        if ply < 1 {
            return Err("--ply-b/--ply must be >= 1".to_string());
        }
        cfg.engine_b
            .options
            .insert("engine.ply".to_string(), ply.to_string());
    }

    if let Some(spec) = &args.engine_a {
        cfg.engine_a.name = engine_identity_from_spec_with_options(spec, &cfg.engine_a.options)?;
    }
    if let Some(spec) = &args.engine_b {
        cfg.engine_b.name = engine_identity_from_spec_with_options(spec, &cfg.engine_b.options)?;
    }

    Ok(cfg)
}

struct SavedDuel {
    store: BenchmarkStore,
    db_path: PathBuf,
    id: i64,
    matchup: MatchupHandle,
}

impl SavedDuel {
    fn start(db_path: PathBuf, name: Option<&str>, cfg: &MatchupConfig) -> Result<Self, String> {
        let mut store = BenchmarkStore::open(&db_path)?;
        let default_name = format!("{} vs {}", cfg.engine_a.name, cfg.engine_b.name);
        let started = store.start_duel(
            BenchmarkSpec {
                name: name.unwrap_or(&default_name),
                variant: &cfg.variant,
                seed: cfg.seed,
                max_plies: cfg.max_plies,
                pairs: cfg.pairs,
            },
            &cfg.engine_a,
            &cfg.engine_b,
        )?;
        Ok(Self {
            store,
            db_path,
            id: started.id,
            matchup: started.matchups[0],
        })
    }

    fn complete(&mut self, games: &[bgci_core::duel_runner::GameRecord]) -> Result<(), String> {
        self.store.record_games(self.matchup, games)?;
        self.store.finish_benchmark(self.id)
    }
}

fn mark_failed(store: &BenchmarkStore, benchmark_id: i64, error: String) -> String {
    match store.fail_benchmark(benchmark_id) {
        Ok(()) => error,
        Err(status_error) => {
            format!("{error}; additionally failed to mark benchmark failed: {status_error}")
        }
    }
}
