use std::path::PathBuf;

use bgci_core::benchmark::{BenchmarkSpec, Database, MatchupHandle, default_db_path};
use bgci_core::common::{parse_variant, variant_name};
use bgci_core::config::{
    MatchupConfig, ResolvedMatchup, load_toml, resolve_engine_input, resolve_engine_spec,
};
use bgci_core::duel_runner::{run_matchup, run_matchup_with_transcripts};
use bgci_core::engine::finalize_resolved_engine;
use clap::Args;
use tracing::info;

use crate::logging;

#[derive(Debug, Args)]
pub struct DuelArgs {
    /// Load matchup and engine settings from TOML.
    #[arg(short = 'c', long)]
    config: Option<String>,

    /// First profile alias or option-qualified engine specification.
    #[arg(short = 'a', long = "engine-a")]
    engine_a: Option<String>,

    /// Second profile alias or option-qualified engine specification.
    #[arg(short = 'b', long = "engine-b")]
    engine_b: Option<String>,

    /// Number of games to run.
    #[arg(long)]
    games: Option<usize>,

    /// Number of mirrored game groups to run concurrently.
    #[arg(short = 'p', long)]
    parallel: Option<usize>,

    /// Base seed used to derive deterministic mirrored dice streams.
    #[arg(short = 's', long)]
    seed: Option<u64>,

    /// Maximum plies before recording a game as incomplete.
    #[arg(short = 'm', long = "max-plies")]
    max_plies: Option<usize>,

    /// Backgammon variant used by every game.
    #[arg(long)]
    variant: Option<String>,

    /// Tracing level: off, error, warn, info, debug, or trace.
    #[arg(long = "log-level")]
    log_level: Option<String>,

    /// Write tracing output to this file instead of stderr.
    #[arg(long, requires = "log_level")]
    log_file: Option<PathBuf>,

    /// Save this duel to the local application database.
    #[arg(long)]
    save: bool,

    /// Human-readable name for a saved duel.
    #[arg(long, requires = "save")]
    name: Option<String>,

    /// Application database path; defaults to the XDG data directory.
    #[arg(long = "db", requires = "save")]
    db_path: Option<PathBuf>,

    /// Write this duel's games as a Jellyfish MAT money session.
    #[arg(long, value_name = "PATH")]
    mat: Option<PathBuf>,
}

pub async fn run(args: DuelArgs) -> Result<(), String> {
    let built = build_matchup_config(&args)?;
    let _log_guard = logging::init_tracing(&built.log_level, args.log_file.as_deref())?;
    let mut cfg = built.matchup;
    if args.mat.is_some() {
        bgci_core::mat::ensure_supported(cfg.variant)?;
    }
    cfg.engine_a = finalize_resolved_engine(cfg.engine_a);
    cfg.engine_b = finalize_resolved_engine(cfg.engine_b);

    info!(
        log_level = %built.log_level,
        games = cfg.games,
        parallel = cfg.parallel,
        seed = cfg.seed,
        max_plies = cfg.max_plies,
        variant = %variant_name(cfg.variant),
        engine_a = %cfg.engine_a.name,
        engine_a_cmd = %cfg.engine_a.launch.command().join(" "),
        engine_b = %cfg.engine_b.name,
        engine_b_cmd = %cfg.engine_b.launch.command().join(" "),
        "duel run header"
    );

    let mut saved = if args.save {
        Some(SavedDuel::start(
            args.db_path.clone().unwrap_or_else(default_db_path),
            args.name.as_deref(),
            &cfg,
        )?)
    } else {
        None
    };
    let run_result = if args.mat.is_some() {
        run_matchup_with_transcripts(&cfg).await
    } else {
        run_matchup(&cfg).await
    };
    let run = match run_result {
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
    if let Some(path) = &args.mat {
        bgci_core::mat::write_session(
            path,
            &cfg.engine_a.name,
            &cfg.engine_b.name,
            cfg.variant,
            &run.games,
        )?;
        println!("mat -> {}", path.display());
    }
    let summary = run.summary;
    for line in summary.lines {
        println!("{line}");
    }
    if let Some(log_file) = &args.log_file {
        println!("log   -> {}", log_file.display());
    }

    info!(games = run.games.len(), "duel run complete");
    Ok(())
}

struct DuelConfig {
    matchup: ResolvedMatchup,
    log_level: String,
}

fn build_matchup_config(args: &DuelArgs) -> Result<DuelConfig, String> {
    let mut cfg = if let Some(config_path) = &args.config {
        load_toml(config_path)?
    } else {
        if args.engine_a.is_none() && args.engine_b.is_none() {
            return Err(
                "duel requires either --config or both --engine-a and --engine-b".to_string(),
            );
        }
        if args.engine_a.is_none() {
            return Err("missing --engine-a (or use --config)".to_string());
        }
        if args.engine_b.is_none() {
            return Err("missing --engine-b (or use --config)".to_string());
        }

        MatchupConfig::default()
    };

    cfg.games = match args.games {
        Some(0) => return Err("--games must be greater than zero".to_string()),
        Some(games) => games,
        None => cfg.games,
    };
    if let Some(parallel) = args.parallel {
        cfg.parallel = parallel;
    }
    if let Some(seed) = args.seed {
        cfg.seed = seed;
    }
    if let Some(max_plies) = args.max_plies {
        cfg.max_plies = max_plies;
    }
    if let Some(variant) = &args.variant {
        cfg.variant = variant.clone();
    }
    if let Some(log_level) = &args.log_level {
        cfg.log_level = log_level.clone();
    }
    let engine_a = match &args.engine_a {
        Some(engine_a) => resolve_engine_spec(engine_a)?,
        None => resolve_engine_input(cfg.engine_a)?,
    };
    let engine_b = match &args.engine_b {
        Some(engine_b) => resolve_engine_spec(engine_b)?,
        None => resolve_engine_input(cfg.engine_b)?,
    };
    let log_level = cfg.log_level;
    let matchup = ResolvedMatchup {
        games: cfg.games,
        parallel: cfg.parallel,
        seed: cfg.seed,
        max_plies: cfg.max_plies,
        variant: parse_variant(&cfg.variant)?,
        engine_a,
        engine_b,
    };

    Ok(DuelConfig { matchup, log_level })
}

struct SavedDuel {
    store: Database,
    db_path: PathBuf,
    id: i64,
    matchup: MatchupHandle,
}

impl SavedDuel {
    fn start(db_path: PathBuf, name: Option<&str>, cfg: &ResolvedMatchup) -> Result<Self, String> {
        let mut store = Database::open(&db_path)?;
        let default_name = format!("{} vs {}", cfg.engine_a.name, cfg.engine_b.name);
        let started = store.start_duel(
            BenchmarkSpec {
                name: name.unwrap_or(&default_name),
                variant: variant_name(cfg.variant),
                seed: cfg.seed,
                max_plies: cfg.max_plies,
                games: cfg.games,
            },
            &cfg.engine_a,
            &cfg.engine_b,
        )?;
        let matchup = started
            .matchups
            .first()
            .ok_or_else(|| "saved duel did not create a matchup".to_string())?
            .handle;
        Ok(Self {
            store,
            db_path,
            id: started.id,
            matchup,
        })
    }

    fn complete(&mut self, games: &[bgci_core::duel_runner::GameRecord]) -> Result<(), String> {
        self.store.record_games(self.matchup, games)?;
        self.store.finish_benchmark(self.id)
    }
}

fn mark_failed(store: &Database, benchmark_id: i64, error: String) -> String {
    match store.fail_benchmark(benchmark_id) {
        Ok(()) => error,
        Err(status_error) => {
            format!("{error}; additionally failed to mark benchmark failed: {status_error}")
        }
    }
}
