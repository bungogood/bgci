use clap::{Args, Subcommand};

use crate::managed::{default_db_path, ManagedRunStore};
use crate::status_display::{StatusDisplay, StatusLines};

#[derive(Debug, Args)]
pub struct RunsArgs {
    #[command(subcommand)]
    command: RunsCommand,
}

#[derive(Debug, Subcommand)]
pub enum RunsCommand {
    List(RunsListArgs),
    Show(RunsShowArgs),
    Watch(RunsWatchArgs),
    Cancel(RunsCancelArgs),
}

#[derive(Debug, Args)]
pub struct RunsListArgs {
    #[arg(long)]
    db: Option<String>,

    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Debug, Args)]
pub struct RunsShowArgs {
    run_id: String,

    #[arg(long)]
    db: Option<String>,
}

#[derive(Debug, Args)]
pub struct RunsCancelArgs {
    run_id: Option<String>,

    #[arg(long)]
    db: Option<String>,
}

#[derive(Debug, Args)]
pub struct RunsWatchArgs {
    run_id: Option<String>,

    #[arg(long)]
    db: Option<String>,

    #[arg(long, default_value_t = 1000)]
    interval_ms: u64,
}

pub fn run(args: RunsArgs) -> Result<(), String> {
    match args.command {
        RunsCommand::List(list) => run_list(list),
        RunsCommand::Show(show) => run_show(show),
        RunsCommand::Watch(watch) => run_watch(watch),
        RunsCommand::Cancel(cancel) => run_cancel(cancel),
    }
}

fn run_list(args: RunsListArgs) -> Result<(), String> {
    let db_path = args
        .db
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_db_path);
    if !db_path.exists() {
        println!("no runs found in {}", db_path.display());
        return Ok(());
    }
    let store = ManagedRunStore::open(&db_path)?;
    let runs = store.list_runs(args.limit.max(1))?;

    if runs.is_empty() {
        println!("no runs found in {}", db_path.display());
        return Ok(());
    }

    for run in runs {
        let name = if run.name.is_empty() {
            "(unnamed)"
        } else {
            run.name.as_str()
        };
        println!(
            "{}  {}  {}/{}  name={}  created={}",
            run.id, run.status, run.completed_games, run.total_games, name, run.created_at
        );
    }
    Ok(())
}

fn run_show(args: RunsShowArgs) -> Result<(), String> {
    let db_path = args
        .db
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_db_path);
    if !db_path.exists() {
        return Err(format!(
            "no managed run database found at {}",
            db_path.display()
        ));
    }
    let store = ManagedRunStore::open(&db_path)?;
    let Some(run) = store.get_run(&args.run_id)? else {
        return Err(format!("run not found: {}", args.run_id));
    };

    println!("run_id: {}", run.id);
    println!(
        "name: {}",
        if run.name.is_empty() {
            "(unnamed)"
        } else {
            &run.name
        }
    );
    println!("status: {}", run.status);
    println!("created_at: {}", run.created_at);
    if let Some(started_at) = run.started_at {
        println!("started_at: {started_at}");
    }
    if let Some(finished_at) = run.finished_at {
        println!("finished_at: {finished_at}");
    }
    println!(
        "progress: {}/{} (failed={})",
        run.completed_games, run.total_games, run.failed_games
    );
    if let Some(output_csv) = run.output_csv {
        if !output_csv.is_empty() {
            println!("output_csv: {output_csv}");
        }
    }
    if let Some(log_file) = run.log_file {
        if !log_file.is_empty() {
            println!("log_file: {log_file}");
        }
    }

    let counts = store.get_job_status_counts(&run.id)?;
    if !counts.is_empty() {
        println!("jobs:");
        for (status, count) in counts {
            println!("  {}: {}", status, count);
        }
    }

    Ok(())
}

fn run_cancel(args: RunsCancelArgs) -> Result<(), String> {
    let db_path = args
        .db
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_db_path);
    if !db_path.exists() {
        return Err(format!(
            "no managed run database found at {}",
            db_path.display()
        ));
    }
    let store = ManagedRunStore::open(&db_path)?;
    let run_id = match args.run_id {
        Some(id) => id,
        None => {
            let running = store.list_running_run_ids()?;
            if running.len() == 1 {
                running[0].clone()
            } else if running.is_empty() {
                return Err("no running runs found; pass a run_id explicitly".to_string());
            } else {
                return Err(format!(
                    "multiple running runs found ({}); pass a run_id explicitly",
                    running.len()
                ));
            }
        }
    };

    let cancelled = store.cancel_run(&run_id)?;
    if cancelled {
        println!("cancelled run {}", run_id);
    } else {
        println!("run {} is not queued/running (no change)", run_id);
    }
    Ok(())
}

fn run_watch(args: RunsWatchArgs) -> Result<(), String> {
    let db_path = args
        .db
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_db_path);
    if !db_path.exists() {
        return Err(format!(
            "no managed run database found at {}",
            db_path.display()
        ));
    }

    let store = ManagedRunStore::open(&db_path)?;
    let run_id = match args.run_id {
        Some(id) => id,
        None => {
            let running = store.list_running_run_ids()?;
            if running.len() == 1 {
                let selected = running[0].clone();
                println!("watching run {}", selected);
                selected
            } else if running.is_empty() {
                return Err("no running runs found; pass a run_id explicitly".to_string());
            } else {
                return Err(format!(
                    "multiple running runs found ({}); pass a run_id explicitly",
                    running.len()
                ));
            }
        }
    };

    let first_store = ManagedRunStore::open(&db_path)?;
    let first_run = first_store
        .get_run(&run_id)?
        .ok_or_else(|| format!("run not found: {}", run_id))?;
    let display = StatusDisplay::new(first_run.total_games as usize, "  WATCH")?;
    let sleep_ms = args.interval_ms.max(100);
    let mut last_line = String::new();
    let mut used_display = false;
    loop {
        let store = ManagedRunStore::open(&db_path)?;
        let Some(run) = store.get_run(&run_id)? else {
            return Err(format!("run not found: {}", run_id));
        };
        if let Some(snapshot) = store.get_run_snapshot(&run.id)? {
            let lines = StatusLines {
                line_engines: snapshot.line_engines,
                line_result: snapshot.line_result,
                line_rate: snapshot.line_rate,
                line_decide: snapshot.line_decide,
                line_class: snapshot.line_class,
                line_sides: snapshot.line_sides,
            };
            let block = format!(
                "{}\n{}\n{}\n{}\n{}\n{}",
                lines.line_engines,
                lines.line_result,
                lines.line_rate,
                lines.line_decide,
                lines.line_class,
                lines.line_sides
            );
            if block != last_line {
                display.update(run.completed_games as usize, &lines);
                last_line = block;
                used_display = true;
            }

            if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
            continue;
        }

        let counts = store.get_job_status_counts(&run.id)?;
        let mut counts_text = String::new();
        for (idx, (status, count)) in counts.iter().enumerate() {
            if idx > 0 {
                counts_text.push_str(", ");
            }
            counts_text.push_str(&format!("{}={}", status, count));
        }
        let pct = if run.total_games <= 0 {
            0.0
        } else {
            (run.completed_games as f64 / run.total_games as f64) * 100.0
        };
        let elapsed = run
            .started_at
            .as_deref()
            .or(Some(run.created_at.as_str()))
            .and_then(parse_rfc3339)
            .map(|start| {
                (time::OffsetDateTime::now_utc() - start)
                    .whole_seconds()
                    .max(0)
            })
            .unwrap_or(0);
        let rate = if elapsed == 0 {
            0.0
        } else {
            run.completed_games as f64 / elapsed as f64
        };
        let line = format!(
            "DUEL run={} status={} {}/{} ({:.2}%) rate={:.2} g/s failed={} jobs=[{}]",
            run.id,
            run.status,
            run.completed_games,
            run.total_games,
            pct,
            rate,
            run.failed_games,
            counts_text
        );
        if line != last_line {
            println!("{line}");
            last_line = line;
        }

        if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
    }
    if used_display {
        display.finish();
    }
    Ok(())
}

fn parse_rfc3339(s: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}
