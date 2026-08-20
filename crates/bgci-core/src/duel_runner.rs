use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::sync::mpsc;
use tracing::debug;

use crate::config::ResolvedMatchup;
use crate::duel_game::DuelGameResult;
use crate::duel_workers::{LocalWorkerSpec, WorkerMessage, spawn_local_workers};
use crate::stats::{DuelStats, GameUpdate};

pub struct RunSummary {
    pub lines: [String; 6],
}

#[derive(Clone, Debug)]
pub struct GameRecord {
    pub game_idx: usize,
    pub points_a: f64,
    pub plies: usize,
    pub a_decisions: usize,
    pub b_decisions: usize,
    pub a_decision_time: Duration,
    pub b_decision_time: Duration,
}

pub struct MatchupRun {
    pub summary: RunSummary,
    pub games: Vec<GameRecord>,
}

pub async fn run_matchup(cfg: &ResolvedMatchup) -> Result<MatchupRun, String> {
    let game_count = validate_execution(cfg.pairs, cfg.parallel, cfg.max_plies)?;

    let ui = ProgressUi::new(game_count)?;

    let mut stats = DuelStats::new();
    let mut games = Vec::with_capacity(game_count);
    let workers = cfg.parallel.min(cfg.pairs);

    let run_start = Instant::now();

    let (tx, mut rx) = mpsc::unbounded_channel::<WorkerMessage>();
    let cancel = Arc::new(AtomicBool::new(false));

    let handles = spawn_local_workers(
        LocalWorkerSpec {
            workers,
            pairs: cfg.pairs,
            variant: cfg.variant,
            max_plies: cfg.max_plies,
            base_seed: cfg.seed,
            engine_a: cfg.engine_a.clone(),
            engine_b: cfg.engine_b.clone(),
            cancel: cancel.clone(),
        },
        tx.clone(),
    );
    drop(tx);

    let mut done_games = 0usize;
    let mut run_error: Option<String> = None;

    while let Some(msg) = rx.recv().await {
        match msg {
            WorkerMessage::Error(err) => {
                if run_error.is_none() {
                    run_error = Some(err);
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            WorkerMessage::Game { game_idx, result } => {
                done_games += 1;
                if run_error.is_none() {
                    games.push(process_completed_game(
                        game_idx, &result, cfg, &mut stats, run_start, &ui, done_games,
                    ));
                }
            }
        }
    }

    for (worker_id, handle) in handles.into_iter().enumerate() {
        if let Err(err) = handle.await {
            let join_error = format!("worker {} task failed: {err}", worker_id + 1);
            if let Some(run_error) = &mut run_error {
                run_error.push_str("; ");
                run_error.push_str(&join_error);
            } else {
                run_error = Some(join_error);
            }
        }
    }

    if let Some(err) = run_error {
        return Err(err);
    }
    if done_games != game_count {
        return Err(format!(
            "duel ended early: completed {done_games}/{game_count} games across {workers} workers"
        ));
    }

    ui.finish();

    let elapsed = run_start.elapsed();
    let lines = stats.status_lines(&cfg.engine_a.name, &cfg.engine_b.name, game_count, elapsed);

    games.sort_by_key(|game| game.game_idx);
    for (expected_idx, game) in games.iter().enumerate() {
        if game.game_idx != expected_idx {
            return Err(format!(
                "invalid mirrored game sequence at game {}",
                expected_idx + 1
            ));
        }
    }
    Ok(MatchupRun {
        summary: RunSummary { lines },
        games,
    })
}

fn validate_execution(pairs: usize, parallel: usize, max_plies: usize) -> Result<usize, String> {
    if pairs == 0 {
        return Err("pairs must be greater than zero".to_string());
    }
    if parallel == 0 {
        return Err("parallel must be greater than zero".to_string());
    }
    if max_plies == 0 {
        return Err("max plies must be greater than zero".to_string());
    }
    pairs
        .checked_mul(2)
        .ok_or_else(|| "pair count is too large".to_string())
}

struct ProgressUi {
    progress: ProgressBar,
    stats: [ProgressBar; 6],
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

        let stats_style = ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?;
        let stats = std::array::from_fn(|_| {
            let bar = mp.add(ProgressBar::new_spinner());
            bar.set_style(stats_style.clone());
            bar
        });

        Ok(Self { progress, stats })
    }

    fn update(&self, done_games: usize, lines: &[String; 6]) {
        self.progress.set_position(done_games as u64);
        for (bar, line) in self.stats.iter().zip(lines) {
            bar.set_message(line.clone());
        }
    }

    fn finish(&self) {
        self.progress.finish_and_clear();
        for bar in &self.stats {
            bar.finish_and_clear();
        }
    }
}

fn process_completed_game(
    game_idx: usize,
    result: &DuelGameResult,
    cfg: &ResolvedMatchup,
    stats: &mut DuelStats,
    run_start: Instant,
    ui: &ProgressUi,
    done_games: usize,
) -> GameRecord {
    let a_is_x = game_idx % 2 == 0;

    debug!(
        game = game_idx + 1,
        winner_x = ?result.winner_x,
        points_x = result.points_x,
        points_o = result.points_o,
        plies = result.plies,
        "game complete"
    );

    let a_game_points = stats.record_game(&GameUpdate {
        game_idx,
        a_is_x,
        winner_x: result.winner_x,
        points_x: result.points_x,
        points_o: result.points_o,
        plies: result.plies,
        a_decisions: result.a_decisions,
        b_decisions: result.b_decisions,
        a_decision_time: result.a_decision_time,
        b_decision_time: result.b_decision_time,
    });

    let elapsed = run_start.elapsed();
    let lines = stats.status_lines(&cfg.engine_a.name, &cfg.engine_b.name, done_games, elapsed);
    ui.update(done_games, &lines);

    GameRecord {
        game_idx,
        points_a: f64::from(a_game_points),
        plies: result.plies,
        a_decisions: result.a_decisions,
        b_decisions: result.b_decisions,
        a_decision_time: result.a_decision_time,
        b_decision_time: result.b_decision_time,
    }
}

#[cfg(test)]
mod tests {
    use super::validate_execution;

    #[test]
    fn validates_execution_limits_without_starting_engines() {
        assert_eq!(validate_execution(1, 1, 1), Ok(2));
        assert_eq!(
            validate_execution(0, 1, 1),
            Err("pairs must be greater than zero".to_string())
        );
        assert_eq!(
            validate_execution(1, 0, 1),
            Err("parallel must be greater than zero".to_string())
        );
        assert_eq!(
            validate_execution(1, 1, 0),
            Err("max plies must be greater than zero".to_string())
        );
        assert_eq!(
            validate_execution(usize::MAX, 1, 1),
            Err("pair count is too large".to_string())
        );
    }
}
