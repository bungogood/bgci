use std::path::PathBuf;

use bgci_core::benchmark::{BenchmarkSpec, Database, default_db_path};
use bgci_core::common::parse_variant;
use bgci_core::config::{ResolvedEngine, ResolvedMatchup};
use bgci_core::duel_runner::run_matchup;
use bgci_core::engine::resolve_engine;
use clap::Args;

#[derive(Debug, Args)]
pub struct LeagueArgs {
    /// Human-readable league name.
    #[arg(long)]
    name: String,

    /// Engine aliases or option-qualified specifications.
    #[arg(long, num_args = 2..)]
    engines: Vec<String>,

    /// Mirrored pairs to run for every engine matchup.
    #[arg(long, default_value_t = 100)]
    pairs_per_matchup: usize,

    /// Number of pair workers to run concurrently.
    #[arg(long, default_value_t = 1)]
    parallel: usize,

    /// Base seed used to derive deterministic matchup and pair seeds.
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Maximum plies before recording a game as incomplete.
    #[arg(long, default_value_t = 512)]
    max_plies: usize,

    /// Backgammon variant used by every matchup.
    #[arg(long, default_value = "backgammon")]
    variant: String,

    /// Application database path; defaults to the XDG data directory.
    #[arg(long = "db")]
    db_path: Option<PathBuf>,
}

pub async fn run(args: LeagueArgs) -> Result<(), String> {
    validate_pairs(args.pairs_per_matchup)?;
    let max_plies = args.max_plies.max(1);
    let variant = parse_variant(&args.variant)?;
    let mut engines = Vec::new();
    for spec in &args.engines {
        let engine = resolve_engine(spec)?;
        if engines
            .iter()
            .any(|existing: &ResolvedEngine| existing.name == engine.name)
        {
            return Err(format!("duplicate resolved engine: {}", engine.name));
        }
        engines.push(engine);
    }

    let db_path = args.db_path.unwrap_or_else(default_db_path);
    let mut store = Database::open(&db_path)?;
    let started = store.start_league(
        BenchmarkSpec {
            name: &args.name,
            variant: &args.variant,
            seed: args.seed,
            max_plies,
            pairs: args.pairs_per_matchup,
        },
        &engines,
    )?;
    let matchup_count = started.matchups.len();

    let mut matchup_number = 0;
    for a in 0..engines.len() {
        for b in (a + 1)..engines.len() {
            matchup_number += 1;
            println!(
                "matchup {matchup_number}/{matchup_count}: {} vs {}",
                engines[a].name, engines[b].name
            );
            let matchup = started.matchups[matchup_number - 1];
            let cfg = ResolvedMatchup {
                pairs: args.pairs_per_matchup,
                parallel: args.parallel.max(1),
                seed: matchup.seed(),
                max_plies,
                variant: args.variant.clone(),
                log_level: "off".to_string(),
                engine_a: engines[a].clone(),
                engine_b: engines[b].clone(),
            };
            let result = match run_matchup(&cfg, variant).await {
                Ok(result) => result,
                Err(error) => {
                    return Err(mark_failed(&store, started.id, error));
                }
            };
            if let Err(error) = store.record_games(matchup, &result.games) {
                return Err(mark_failed(&store, started.id, error));
            }
        }
    }
    store.finish_benchmark(started.id)?;
    println!("league {} completed: {matchup_count} matchups", started.id);
    println!("db -> {}", db_path.display());
    Ok(())
}

fn mark_failed(store: &Database, benchmark_id: i64, error: String) -> String {
    match store.fail_benchmark(benchmark_id) {
        Ok(()) => error,
        Err(status_error) => {
            format!("{error}; additionally failed to mark benchmark failed: {status_error}")
        }
    }
}

fn validate_pairs(pairs: usize) -> Result<(), String> {
    if pairs == 0 {
        Err("--pairs-per-matchup must be greater than zero".to_string())
    } else if pairs.checked_mul(2).is_none() {
        Err("pair count is too large".to_string())
    } else {
        Ok(())
    }
}
