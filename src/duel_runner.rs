use std::fs;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bkgm::Variant;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant as TokioInstant, sleep_until};
use tracing::debug;

use crate::config::DuelConfig;
use crate::duel_messages::{CompletedGame, WorkerMessage};
use crate::duel_workers::{spawn_local_workers, LocalWorkerSpec};
use crate::output_paths::RunPaths;
use crate::report::render_status_lines;
use crate::stats::{DuelStats, GameUpdate};

pub struct RunSummary {
    pub line_engines: String,
    pub line_result: String,
    pub line_rate: String,
    pub line_decide: String,
    pub line_class: String,
    pub line_sides: String,
}

pub async fn run_duel(
    cfg: &DuelConfig,
    variant: Variant,
    paths: &RunPaths,
    save_results: bool,
) -> Result<RunSummary, String> {
    let mut artifacts = RunArtifacts::new(paths, save_results)?;
    let ui = ProgressUi::new(cfg.games)?;

    let mut stats = DuelStats::new();
    let workers = cfg.parallel.max(1).min(cfg.games.max(1));

    let run_start = Instant::now();

    let (tx, mut rx) = mpsc::unbounded_channel::<WorkerMessage>();
    let timeout_secs = cfg.timeout_secs;
    let cancel = Arc::new(AtomicBool::new(false));
    let deadline = timeout_secs.map(|secs| TokioInstant::now() + Duration::from_secs(secs));

    spawn_local_workers(
        LocalWorkerSpec {
            workers,
            games: cfg.games,
            variant,
            max_plies: cfg.max_plies,
            swap_sides: cfg.swap_sides,
            base_seed: cfg.seed,
            engine_a: cfg.engine_a.clone(),
            engine_b: cfg.engine_b.clone(),
            cancel: cancel.clone(),
        },
        tx.clone(),
    );
    drop(tx);

    let mut done_workers = 0usize;
    let mut done_games = 0usize;
    let mut run_error: Option<String> = None;

    while done_workers < workers {
        let msg = if let Some(deadline) = deadline {
            tokio::select! {
                maybe = rx.recv() => maybe,
                _ = sleep_until(deadline), if run_error.is_none() => {
                    run_error = Some(format!(
                        "duel timed out after {}s (completed {}/{} games)",
                        timeout_secs.unwrap_or(0), done_games, cfg.games
                    ));
                    cancel.store(true, Ordering::Relaxed);
                    continue;
                }
            }
        } else {
            rx.recv().await
        };
        let msg = match msg {
            Some(msg) => msg,
            None => break,
        };
        match msg {
            WorkerMessage::Done => {
                done_workers += 1;
            }
            WorkerMessage::Error(err) => {
                if run_error.is_none() {
                    run_error = Some(err);
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            WorkerMessage::Game(done) => {
                done_games += 1;
                if run_error.is_none() {
                    process_completed_game(
                        &done,
                        cfg,
                        &mut artifacts,
                        &mut stats,
                        run_start,
                        &ui,
                        done_games,
                    )?;
                }
            }
        }
    }

    if let Some(err) = run_error {
        return Err(err);
    }

    artifacts.flush()?;
    ui.finish();

    let elapsed = run_start.elapsed();
    let (line_engines, line_result, line_rate, line_decide, line_class, line_sides) =
        render_status_lines(stats.status_view(&cfg.engine_a.name, &cfg.engine_b.name, cfg.games, elapsed));

    Ok(RunSummary {
        line_engines,
        line_result,
        line_rate,
        line_decide,
        line_class,
        line_sides,
    })
}

struct RunArtifacts {
    trace_dir: std::path::PathBuf,
    csv: Option<BufWriter<fs::File>>,
}

impl RunArtifacts {
    fn new(paths: &RunPaths, save_results: bool) -> Result<Self, String> {
        let csv = if save_results {
            if let Some(parent) = paths.output_csv.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let file = fs::File::create(&paths.output_csv).map_err(|e| e.to_string())?;
            let mut writer = BufWriter::new(file);
            writeln!(
                writer,
                "game,engine_x,engine_o,winner,outcome,points_x,points_o,points_a,points_b,plies"
            )
            .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            Some(writer)
        } else {
            None
        };

        fs::create_dir_all(&paths.trace_games_dir).map_err(|e| e.to_string())?;

        Ok(Self {
            trace_dir: paths.trace_games_dir.clone(),
            csv,
        })
    }

    fn flush(&mut self) -> Result<(), String> {
        if let Some(csv) = self.csv.as_mut() {
            csv.flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

struct ProgressUi {
    progress: ProgressBar,
    stats_engines: ProgressBar,
    stats_result: ProgressBar,
    stats_rate: ProgressBar,
    stats_decide: ProgressBar,
    stats_class: ProgressBar,
    stats_sides: ProgressBar,
}

impl ProgressUi {
    fn new(total_games: usize) -> Result<Self, String> {
        let mp = MultiProgress::new();
        let progress = mp.add(ProgressBar::new(total_games as u64));
        progress.set_style(
            ProgressStyle::with_template(
                "{prefix} {wide_bar:.green/black} {pos}/{len} ({percent}%) eta {eta_precise}",
            )
            .map_err(|e| e.to_string())?
            .progress_chars("█▉░"),
        );
        progress.set_prefix("   DUEL");

        let stats_engines = mp.add(ProgressBar::new_spinner());
        stats_engines.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_result = mp.add(ProgressBar::new_spinner());
        stats_result.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_rate = mp.add(ProgressBar::new_spinner());
        stats_rate.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_decide = mp.add(ProgressBar::new_spinner());
        stats_decide.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_class = mp.add(ProgressBar::new_spinner());
        stats_class.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
        let stats_sides = mp.add(ProgressBar::new_spinner());
        stats_sides.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);

        Ok(Self {
            progress,
            stats_engines,
            stats_result,
            stats_rate,
            stats_decide,
            stats_class,
            stats_sides,
        })
    }

    fn update(&self, done_games: usize, lines: (&str, &str, &str, &str, &str, &str)) {
        self.progress.set_position(done_games as u64);
        self.stats_engines.set_message(lines.0.to_string());
        self.stats_result.set_message(lines.1.to_string());
        self.stats_rate.set_message(lines.2.to_string());
        self.stats_decide.set_message(lines.3.to_string());
        self.stats_class.set_message(lines.4.to_string());
        self.stats_sides.set_message(lines.5.to_string());
    }

    fn finish(&self) {
        self.progress.finish_and_clear();
        self.stats_engines.finish_and_clear();
        self.stats_result.finish_and_clear();
        self.stats_rate.finish_and_clear();
        self.stats_decide.finish_and_clear();
        self.stats_class.finish_and_clear();
        self.stats_sides.finish_and_clear();
    }
}

fn process_completed_game(
    done: &CompletedGame,
    cfg: &DuelConfig,
    artifacts: &mut RunArtifacts,
    stats: &mut DuelStats,
    run_start: Instant,
    ui: &ProgressUi,
    done_games: usize,
) -> Result<(), String> {
    let game_idx = done.game_idx;
    let a_is_x = done.a_is_x;
    let result = &done.result;

    let trace_path = artifacts.trace_dir.join(format!("game_{:05}.log", game_idx + 1));
    let mut trace = String::new();
    trace.push_str(&format!(
        "game={} engine_x={} engine_o={} winner={} plies={}\n",
        game_idx + 1,
        if a_is_x {
            &cfg.engine_a.name
        } else {
            &cfg.engine_b.name
        },
        if a_is_x {
            &cfg.engine_b.name
        } else {
            &cfg.engine_a.name
        },
        match result.winner_x {
            Some(true) => "x",
            Some(false) => "o",
            None => "incomplete",
        },
        result.plies,
    ));
    for line in &result.trace_lines {
        trace.push_str(line);
        trace.push('\n');
    }
    fs::write(trace_path, trace).map_err(|e| e.to_string())?;

    debug!(
        game = game_idx + 1,
        winner_x = ?result.winner_x,
        points_x = result.points_x,
        points_o = result.points_o,
        plies = result.plies,
        "game complete"
    );

    let winner_name = match result.winner_x {
        Some(true) => {
            if a_is_x {
                &cfg.engine_a.name
            } else {
                &cfg.engine_b.name
            }
        }
        Some(false) => {
            if a_is_x {
                &cfg.engine_b.name
            } else {
                &cfg.engine_a.name
            }
        }
        None => "incomplete",
    };

    let (a_game_points, b_game_points) = stats.record_game(&GameUpdate {
        a_is_x,
        winner_x: result.winner_x,
        points_x: result.points_x,
        points_o: result.points_o,
        plies: result.plies,
        a_decisions: result.a_decisions,
        b_decisions: result.b_decisions,
        a_decision_sec: result.a_decision_sec,
        b_decision_sec: result.b_decision_sec,
    });

    let engine_x = if a_is_x {
        &cfg.engine_a.name
    } else {
        &cfg.engine_b.name
    };
    let engine_o = if a_is_x {
        &cfg.engine_b.name
    } else {
        &cfg.engine_a.name
    };
    let outcome = if result.winner_x.is_none() {
        "incomplete"
    } else {
        match result.points_x.abs().round() as i32 {
            3 => "backgammon",
            2 => "gammon",
            1 => "normal",
            _ => "unknown",
        }
    };

    if let Some(csv) = artifacts.csv.as_mut() {
        writeln!(
            csv,
            "{},{},{},{},{},{:.1},{:.1},{:.1},{:.1},{}",
            game_idx + 1,
            engine_x,
            engine_o,
            winner_name,
            outcome,
            result.points_x,
            result.points_o,
            a_game_points,
            b_game_points,
            result.plies
        )
        .map_err(|e| e.to_string())?;
        if done_games.is_multiple_of(256) {
            csv.flush().map_err(|e| e.to_string())?;
        }
    }

    let elapsed = run_start.elapsed();
    let (line_engines, line_result, line_rate, line_decide, line_class, line_sides) =
        render_status_lines(stats.status_view(&cfg.engine_a.name, &cfg.engine_b.name, done_games, elapsed));
    ui.update(
        done_games,
        (
            &line_engines,
            &line_result,
            &line_rate,
            &line_decide,
            &line_class,
            &line_sides,
        ),
    );

    Ok(())
}
