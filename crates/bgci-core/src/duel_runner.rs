use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bkgm::Variant;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::sync::mpsc;
use tracing::debug;

use crate::config::MatchupConfig;
use crate::duel_messages::{CompletedGame, WorkerMessage};
use crate::duel_workers::{LocalWorkerSpec, spawn_local_workers};
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

#[derive(Clone, Debug)]
pub enum GameOutcome {
    Normal,
    Gammon,
    Backgammon,
    Unknown,
}

impl GameOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Gammon => "gammon",
            Self::Backgammon => "backgammon",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GameRecord {
    pub game_idx: usize,
    pub a_is_x: bool,
    pub winner_a: Option<bool>,
    pub outcome: Option<GameOutcome>,
    pub points_x: f64,
    pub points_o: f64,
    pub points_a: f64,
    pub points_b: f64,
    pub plies: usize,
}

pub struct MatchupRun {
    pub summary: RunSummary,
    pub games: Vec<GameRecord>,
}

pub async fn run_matchup(cfg: &MatchupConfig, variant: Variant) -> Result<MatchupRun, String> {
    let engine_a_label = cfg.engine_a.name.clone();
    let engine_b_label = cfg.engine_b.name.clone();
    let game_count = cfg
        .pairs
        .checked_mul(2)
        .ok_or_else(|| "pair count is too large".to_string())?;

    let ui = ProgressUi::new(game_count)?;

    let mut stats = DuelStats::new();
    let mut games = Vec::with_capacity(game_count);
    let workers = cfg.parallel.max(1).min(cfg.pairs.max(1));

    let run_start = Instant::now();

    let (tx, mut rx) = mpsc::unbounded_channel::<WorkerMessage>();
    let cancel = Arc::new(AtomicBool::new(false));

    spawn_local_workers(
        LocalWorkerSpec {
            workers,
            pairs: cfg.pairs,
            variant,
            max_plies: cfg.max_plies,
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
        let msg = rx.recv().await;
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
                    games.push(process_completed_game(
                        &done, cfg, &mut stats, run_start, &ui, done_games,
                    )?);
                }
            }
        }
    }

    if let Some(err) = run_error {
        return Err(err);
    }
    if done_workers != workers || done_games != game_count {
        return Err(format!(
            "duel ended early: completed {done_games}/{game_count} games across {done_workers}/{workers} workers"
        ));
    }

    ui.finish();

    let elapsed = run_start.elapsed();
    let (line_engines, line_result, line_rate, line_decide, line_class, line_sides) =
        render_status_lines(stats.status_view(
            &engine_a_label,
            &engine_b_label,
            game_count,
            elapsed,
        ));

    games.sort_by_key(|game| game.game_idx);
    for (expected_idx, game) in games.iter().enumerate() {
        if game.game_idx != expected_idx || game.a_is_x != expected_idx.is_multiple_of(2) {
            return Err(format!(
                "invalid mirrored game sequence at game {}",
                expected_idx + 1
            ));
        }
    }
    Ok(MatchupRun {
        summary: RunSummary {
            line_engines,
            line_result,
            line_rate,
            line_decide,
            line_class,
            line_sides,
        },
        games,
    })
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
    cfg: &MatchupConfig,
    stats: &mut DuelStats,
    run_start: Instant,
    ui: &ProgressUi,
    done_games: usize,
) -> Result<GameRecord, String> {
    let game_idx = done.game_idx;
    let a_is_x = done.a_is_x;
    let result = &done.result;

    debug!(
        game = game_idx + 1,
        winner_x = ?result.winner_x,
        points_x = result.points_x,
        points_o = result.points_o,
        plies = result.plies,
        "game complete"
    );

    let winner_a = result.winner_x.map(|winner_x| winner_x == a_is_x);

    let (a_game_points, b_game_points) = stats.record_game(&GameUpdate {
        game_idx,
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

    let outcome = if result.winner_x.is_none() {
        None
    } else {
        Some(match result.points_x.abs().round() as i32 {
            3 => GameOutcome::Backgammon,
            2 => GameOutcome::Gammon,
            1 => GameOutcome::Normal,
            _ => GameOutcome::Unknown,
        })
    };

    let elapsed = run_start.elapsed();
    let (line_engines, line_result, line_rate, line_decide, line_class, line_sides) =
        render_status_lines(stats.status_view(
            &cfg.engine_a.name,
            &cfg.engine_b.name,
            done_games,
            elapsed,
        ));
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

    Ok(GameRecord {
        game_idx,
        a_is_x,
        winner_a,
        outcome,
        points_x: f64::from(result.points_x),
        points_o: f64::from(result.points_o),
        points_a: f64::from(a_game_points),
        points_b: f64::from(b_game_points),
        plies: result.plies,
    })
}
