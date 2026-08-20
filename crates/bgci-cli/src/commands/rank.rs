use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bgci_core::benchmark::{BenchmarkStore, RankingPool, RankingSpec, default_benchmark_db_path};
use bgci_core::common::parse_variant;
use bgci_core::config::{EngineConfig, MatchupConfig};
use bgci_core::duel_runner::run_matchup;
use bgci_core::engine::resolve_engine;
use bgci_core::ranking::{fit_ratings, is_provisional, select_pair_for_pool};
use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct RankArgs {
    /// Benchmark database path; defaults to the XDG data directory.
    #[arg(long = "db", global = true)]
    db_path: Option<PathBuf>,

    #[command(subcommand)]
    command: RankCommand,
}

#[derive(Debug, Subcommand)]
enum RankCommand {
    /// Create a named persistent ranking pool.
    Create(CreateArgs),
    /// Run or continue a named ranking pool.
    Run(RunArgs),
    /// Add provisional engines to a paused ranking pool.
    Add(AddArgs),
    /// Recompute and display a named ranking pool.
    Show(PoolArgs),
    /// List named ranking pools.
    List,
}

#[derive(Debug, Args)]
struct CreateArgs {
    /// Ranking pool name.
    #[arg(default_value = "main")]
    name: String,

    /// Initial engine aliases or option-qualified specifications.
    #[arg(long, num_args = 2..)]
    engines: Vec<String>,

    /// Base seed used to derive deterministic batch and pair seeds.
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Maximum plies before recording a game as incomplete.
    #[arg(long, default_value_t = 512)]
    max_plies: usize,

    /// Backgammon variant used by every matchup.
    #[arg(long, default_value = "backgammon")]
    variant: String,

    /// Distinct sufficiently-covered opponents needed for established status.
    #[arg(long, default_value_t = 3)]
    placement_opponents: usize,

    /// Pairs required against each placement opponent.
    #[arg(long, default_value_t = 20)]
    placement_pairs: usize,

    /// Maximum approximate RD for established status.
    #[arg(long, default_value_t = 80.0)]
    established_rd: f64,
}

#[derive(Debug, Args)]
struct RunArgs {
    /// Ranking pool name.
    #[arg(default_value = "main")]
    name: String,

    #[command(flatten)]
    session: SessionArgs,
}

#[derive(Debug, Args)]
struct AddArgs {
    /// Ranking pool name.
    #[arg(default_value = "main")]
    name: String,

    /// Engines to add as provisional members.
    #[arg(long, num_args = 1..)]
    engines: Vec<String>,
}

#[derive(Debug, Args)]
struct PoolArgs {
    /// Ranking pool name.
    #[arg(default_value = "main")]
    name: String,
}

#[derive(Debug, Clone, Args)]
struct SessionArgs {
    /// Additional pairs to run in this session; omit to run until Ctrl-C.
    #[arg(long)]
    budget_pairs: Option<usize>,

    /// Mirrored pairs in each adaptively selected batch.
    #[arg(long, default_value_t = 10)]
    batch_pairs: usize,

    /// Number of pair workers to run concurrently.
    #[arg(long, default_value_t = 1)]
    parallel: usize,
}

pub async fn run(args: RankArgs) -> Result<(), String> {
    let db_path = args.db_path.unwrap_or_else(default_benchmark_db_path);
    let mut store = BenchmarkStore::open(&db_path)?;
    match args.command {
        RankCommand::Create(args) => {
            parse_variant(&args.variant)?;
            if args.placement_opponents == 0 || args.placement_pairs == 0 {
                return Err("placement opponents and pairs must be greater than zero".to_string());
            }
            if !args.established_rd.is_finite() || args.established_rd <= 0.0 {
                return Err("--established-rd must be a positive number".to_string());
            }
            let engines = resolve_engines(&args.engines)?;
            let pool = store.start_ranking(
                RankingSpec {
                    name: &args.name,
                    variant: &args.variant,
                    seed: args.seed,
                    max_plies: args.max_plies.max(1),
                    placement_opponents: args.placement_opponents,
                    placement_pairs: args.placement_pairs,
                    established_rd: args.established_rd,
                },
                &engines,
            )?;
            println!("ranking '{}' created -> {}", pool.name, db_path.display());
            show_ranking(&store, &pool)
        }
        RankCommand::Run(args) => {
            validate_session(&args.session)?;
            let pool = store.load_ranking_by_name(&args.name)?;
            store.resume_ranking(pool.id)?;
            let pool = store.load_ranking_by_name(&args.name)?;
            println!("running ranking '{}'", pool.name);
            run_pool(&mut store, pool, args.session).await
        }
        RankCommand::Add(args) => {
            let engines = resolve_engines(&args.engines)?;
            let pool = store.add_ranking_engines(&args.name, &engines)?;
            println!(
                "added {} engine(s) to ranking '{}'",
                engines.len(),
                pool.name
            );
            show_ranking(&store, &pool)
        }
        RankCommand::Show(args) => {
            let pool = store.load_ranking_by_name(&args.name)?;
            show_ranking(&store, &pool)
        }
        RankCommand::List => {
            let rankings = store
                .list()?
                .into_iter()
                .filter(|row| row.kind == "ranking")
                .collect::<Vec<_>>();
            if rankings.is_empty() {
                println!("no ranking pools");
            } else {
                println!("name                           status       pairs   games");
                for ranking in rankings {
                    println!(
                        "{:<30} {:<10} {:>7} {:>7}",
                        ranking.name, ranking.status, ranking.completed_pairs, ranking.games
                    );
                }
            }
            Ok(())
        }
    }
}

async fn run_pool(
    store: &mut BenchmarkStore,
    mut pool: RankingPool,
    session: SessionArgs,
) -> Result<(), String> {
    let variant = parse_variant(&pool.variant)?;
    let stop = Arc::new(AtomicBool::new(false));
    let signal_stop = stop.clone();
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_stop.store(true, Ordering::Relaxed);
            eprintln!("Ctrl-C received; pausing after the current batch");
        }
    });
    let mut session_pairs = 0usize;

    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break pause(store, &pool.name, pool.id, None);
        }
        let batch_pairs = match session.budget_pairs {
            Some(budget) => {
                let remaining = budget.saturating_sub(session_pairs);
                if remaining == 0 {
                    break pause(store, &pool.name, pool.id, None);
                }
                session.batch_pairs.min(remaining)
            }
            None => session.batch_pairs,
        };

        let data = match store.ranking_data(&pool) {
            Ok(data) => data,
            Err(error) => break pause(store, &pool.name, pool.id, Some(error)),
        };
        let ratings = fit_ratings(pool.engines.len(), &data.games);
        let Some((engine_a, engine_b)) = select_pair_for_pool(
            &ratings,
            &data.pair_counts,
            &data.average_decision_time,
            &data.last_played_batch,
            pool.next_batch,
            pool.placement_opponents,
            pool.placement_pairs,
        ) else {
            break pause(
                store,
                &pool.name,
                pool.id,
                Some("unable to select the next matchup".to_string()),
            );
        };
        println!(
            "batch {}: {} vs {} ({} pairs)",
            pool.next_batch + 1,
            pool.engines[engine_a].name,
            pool.engines[engine_b].name,
            batch_pairs
        );
        let matchup = match store.start_ranking_batch(&pool, engine_a, engine_b, batch_pairs) {
            Ok(matchup) => matchup,
            Err(error) => break pause(store, &pool.name, pool.id, Some(error)),
        };
        let config = MatchupConfig {
            pairs: batch_pairs,
            parallel: session.parallel.max(1),
            seed: matchup.seed(),
            max_plies: pool.max_plies,
            variant: pool.variant.clone(),
            log_level: "off".to_string(),
            engine_a: pool.engines[engine_a].config.clone(),
            engine_b: pool.engines[engine_b].config.clone(),
        };
        let run = match run_matchup(&config, variant).await {
            Ok(run) => run,
            Err(error) => {
                let error = cleanup_failed_batch(store, matchup, error);
                break pause(store, &pool.name, pool.id, Some(error));
            }
        };
        if let Err(error) = store.record_games(matchup, &run.games) {
            let error = cleanup_failed_batch(store, matchup, error);
            break pause(store, &pool.name, pool.id, Some(error));
        }
        pool.next_batch += 1;
        session_pairs += batch_pairs;
        if let Err(error) = show_ranking(store, &pool) {
            break pause(store, &pool.name, pool.id, Some(error));
        }
    };
    signal_task.abort();
    result
}

fn show_ranking(store: &BenchmarkStore, pool: &RankingPool) -> Result<(), String> {
    let data = store.ranking_data(pool)?;
    let mut ratings = fit_ratings(pool.engines.len(), &data.games);
    ratings.sort_by(|a, b| b.elo.total_cmp(&a.elo));
    println!();
    println!("ranking '{}' [{}]", pool.name, pool.status);
    println!(
        " rank  status       family             engine                         rating     rd  move ms   games  pairs"
    );
    for (rank, rating) in ratings.iter().enumerate() {
        let engine = &pool.engines[rating.index];
        let pairs = data.pair_counts[rating.index].iter().sum::<usize>();
        let status = if is_provisional(
            rating,
            &data.pair_counts,
            pool.placement_opponents,
            pool.placement_pairs,
            pool.established_rd,
        ) {
            "provisional"
        } else {
            "established"
        };
        let move_ms = data.average_decision_time[rating.index]
            .map(|duration| format!("{:.2}", duration.as_secs_f64() * 1_000.0))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:>5}  {:<11}  {:<18}  {:<29}  {:>7.1}  {:>5.1}  {:>7}  {:>6}  {:>5}",
            rank + 1,
            status,
            engine.family.as_deref().unwrap_or("-"),
            engine.name,
            rating.elo,
            rating.rd,
            move_ms,
            rating.games,
            pairs
        );
    }
    println!();
    Ok(())
}

fn resolve_engines(specs: &[String]) -> Result<Vec<EngineConfig>, String> {
    let mut engines = Vec::with_capacity(specs.len());
    for spec in specs {
        let engine = resolve_engine(spec)?;
        if engines
            .iter()
            .any(|existing: &EngineConfig| existing.name == engine.name)
        {
            return Err(format!("duplicate resolved engine: {}", engine.name));
        }
        engines.push(engine);
    }
    Ok(engines)
}

fn validate_session(args: &SessionArgs) -> Result<(), String> {
    if args.batch_pairs == 0 {
        return Err("--batch-pairs must be greater than zero".to_string());
    }
    if args.budget_pairs == Some(0) {
        return Err("--budget-pairs must be greater than zero when supplied".to_string());
    }
    Ok(())
}

fn pause(store: &BenchmarkStore, name: &str, id: i64, error: Option<String>) -> Result<(), String> {
    let pause_error = store.pause_ranking(id).err();
    match (error, pause_error) {
        (None, None) => {
            println!("ranking '{name}' paused");
            Ok(())
        }
        (Some(error), None) => Err(error),
        (None, Some(pause_error)) => Err(pause_error),
        (Some(error), Some(pause_error)) => Err(format!(
            "{error}; additionally failed to pause ranking: {pause_error}"
        )),
    }
}

fn cleanup_failed_batch(
    store: &BenchmarkStore,
    matchup: bgci_core::benchmark::MatchupHandle,
    error: String,
) -> String {
    match store.discard_empty_matchup(matchup) {
        Ok(()) => error,
        Err(cleanup_error) => {
            format!("{error}; additionally failed to discard batch: {cleanup_error}")
        }
    }
}
