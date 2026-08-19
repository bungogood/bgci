use std::path::PathBuf;

use bgci_core::benchmark::{BenchmarkStore, default_benchmark_db_path};
use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct HistoryArgs {
    /// Benchmark database path; defaults to the XDG data directory.
    #[arg(long = "db", global = true)]
    db_path: Option<PathBuf>,

    #[command(subcommand)]
    command: HistoryCommand,
}

#[derive(Debug, Subcommand)]
enum HistoryCommand {
    /// List saved duels and leagues.
    List,
    /// Show one saved duel or league.
    Show { id: i64 },
}

pub fn run(args: HistoryArgs) -> Result<(), String> {
    let db_path = args.db_path.unwrap_or_else(default_benchmark_db_path);
    if !db_path.exists() {
        println!("no saved duels or leagues");
        return Ok(());
    }
    let store = BenchmarkStore::open(db_path)?;
    match args.command {
        HistoryCommand::List => list(&store),
        HistoryCommand::Show { id } => show(&store, id),
    }
}

fn list(store: &BenchmarkStore) -> Result<(), String> {
    let rows = store.list()?;
    if rows.is_empty() {
        println!("no saved duels or leagues");
        return Ok(());
    }
    println!(" id  kind    status     pairs       games  name");
    for row in rows {
        println!(
            "{:>3}  {:<6}  {:<9}  {:>5}/{:<5}  {:>5}  {}",
            row.id,
            row.kind,
            row.status,
            row.completed_pairs,
            row.requested_pairs,
            row.games,
            row.name
        );
    }
    Ok(())
}

fn show(store: &BenchmarkStore, id: i64) -> Result<(), String> {
    let row = store
        .get(id)?
        .ok_or_else(|| format!("benchmark {id} not found"))?;
    println!("{} {}: {}", row.kind, row.id, row.name);
    println!("status:    {}", row.status);
    println!("variant:   {}", row.variant);
    println!("pairs:     {}/{}", row.completed_pairs, row.requested_pairs);
    println!("games:     {}", row.games);
    let summaries = store.engine_summaries(id)?;
    if !summaries.is_empty() {
        println!();
        println!(
            " rank  engine                         role       games   wins   win%     points     ppg"
        );
        for (index, summary) in summaries.iter().enumerate() {
            let win_rate = if summary.games == 0 {
                0.0
            } else {
                summary.wins as f64 * 100.0 / summary.games as f64
            };
            let ppg = if summary.games == 0 {
                0.0
            } else {
                summary.points / summary.games as f64
            };
            println!(
                "{:>5}  {:<29}  {:<9}  {:>5}  {:>5}  {:>5.1}  {:>9.1}  {:>6.3}",
                index + 1,
                summary.name,
                summary.role,
                summary.games,
                summary.wins,
                win_rate,
                summary.points,
                ppg
            );
        }
    }
    Ok(())
}
