use std::path::PathBuf;

use bgci_core::benchmark::{BenchmarkSpec, Database, default_db_path};
use bgci_core::common::{parse_variant, variant_name};
use bgci_core::config::ResolvedMatchup;
use bgci_core::duel_runner::run_matchup;
use bgci_core::engine::resolve_and_finalize_engines;
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
    let variant = parse_variant(&args.variant)?;
    let engines = resolve_and_finalize_engines(&args.engines)?;

    let db_path = args.db_path.unwrap_or_else(default_db_path);
    let mut store = Database::open(&db_path)?;
    let started = store.start_league(
        BenchmarkSpec {
            name: &args.name,
            variant: variant_name(variant),
            seed: args.seed,
            max_plies: args.max_plies,
            pairs: args.pairs_per_matchup,
        },
        &engines,
    )?;
    let matchup_count = started.matchups.len();

    for (matchup_index, scheduled) in started.matchups.iter().enumerate() {
        let matchup_number = matchup_index + 1;
        println!(
            "matchup {matchup_number}/{matchup_count}: {} vs {}",
            engines[scheduled.engine_a].name, engines[scheduled.engine_b].name
        );
        let cfg = ResolvedMatchup {
            pairs: args.pairs_per_matchup,
            parallel: args.parallel,
            seed: scheduled.handle.seed(),
            max_plies: args.max_plies,
            variant,
            engine_a: engines[scheduled.engine_a].clone(),
            engine_b: engines[scheduled.engine_b].clone(),
        };
        let result = match run_matchup(&cfg).await {
            Ok(result) => result,
            Err(error) => {
                return Err(mark_failed(&store, started.id, error));
            }
        };
        if let Err(error) = store.record_games(scheduled.handle, &result.games) {
            return Err(mark_failed(&store, started.id, error));
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
