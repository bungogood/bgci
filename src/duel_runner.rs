use std::fs;
use std::io::{BufWriter, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::dice_gen::{DiceGen, FastrandDice};
use bkgm::{normalize_move_text, Game, GameState, Variant};
use tracing::debug;

use crate::config::DuelConfig;
use crate::engine::EngineProcess;
use crate::output_paths::RunPaths;
use crate::report::render_status_lines;
use crate::stats::{DuelStats, GameUpdate};
use crate::status_display::{StatusDisplay, StatusLines};

pub struct RunSummary {
    pub line_engines: String,
    pub line_result: String,
    pub line_rate: String,
    pub line_decide: String,
    pub line_class: String,
    pub line_sides: String,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct ProgressSnapshot {
    pub done_games: usize,
    pub lines: StatusLines,
}

enum WorkerMessage {
    Game(CompletedGame),
    Error(String),
    Done,
}

struct CompletedGame {
    game_idx: usize,
    a_is_x: bool,
    result: DuelGameResult,
}

pub fn run_duel(
    cfg: &DuelConfig,
    variant: Variant,
    paths: &RunPaths,
) -> Result<RunSummary, String> {
    run_duel_with_progress(cfg, variant, paths, |_| Ok(()))
}

pub fn run_duel_with_progress<F>(
    cfg: &DuelConfig,
    variant: Variant,
    paths: &RunPaths,
    mut on_game_done: F,
) -> Result<RunSummary, String>
where
    F: FnMut(&ProgressSnapshot) -> Result<(), String>,
{
    if let Some(parent) = paths.output_csv.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = fs::File::create(&paths.output_csv).map_err(|e| e.to_string())?;
    let mut csv = BufWriter::new(file);
    writeln!(
        csv,
        "game,engine_x,engine_o,winner,outcome,points_x,points_o,points_a,points_b,plies"
    )
    .map_err(|e| e.to_string())?;
    csv.flush().map_err(|e| e.to_string())?;

    let trace_dir = paths.trace_games_dir.clone();
    fs::create_dir_all(&trace_dir).map_err(|e| e.to_string())?;
    let ubgi_dir = if ubgi_log_enabled(&cfg.ubgi_log) {
        fs::create_dir_all(&paths.ubgi_games_dir).map_err(|e| e.to_string())?;
        Some(paths.ubgi_games_dir.clone())
    } else {
        None
    };

    let mut stats = DuelStats::new();
    let workers = cfg.parallel.max(1).min(cfg.games.max(1));

    let run_start = Instant::now();
    let display = StatusDisplay::new(cfg.games, "   DUEL")?;

    let (tx, rx) = mpsc::channel::<WorkerMessage>();
    let engine_a_cfg = cfg.engine_a.clone();
    let engine_b_cfg = cfg.engine_b.clone();
    let max_plies = cfg.max_plies;
    let swap_sides = cfg.swap_sides;
    let base_seed = cfg.seed;
    let games = cfg.games;
    let ubgi_dir_for_workers = ubgi_dir.clone();

    thread::scope(|scope| {
        for worker_id in 0..workers {
            let tx = tx.clone();
            let engine_a_cfg = engine_a_cfg.clone();
            let engine_b_cfg = engine_b_cfg.clone();
            let worker_variant = variant;
            let worker_ubgi_dir = ubgi_dir_for_workers.clone();
            scope.spawn(move || {
                let mut engine_a = match EngineProcess::spawn(&engine_a_cfg) {
                    Ok(e) => e,
                    Err(err) => {
                        let _ = tx.send(WorkerMessage::Error(format!(
                            "worker {} failed to spawn engine A: {}",
                            worker_id + 1,
                            err
                        )));
                        let _ = tx.send(WorkerMessage::Done);
                        return;
                    }
                };
                let mut engine_b = match EngineProcess::spawn(&engine_b_cfg) {
                    Ok(e) => e,
                    Err(err) => {
                        let _ = tx.send(WorkerMessage::Error(format!(
                            "worker {} failed to spawn engine B: {}",
                            worker_id + 1,
                            err
                        )));
                        let _ = tx.send(WorkerMessage::Done);
                        return;
                    }
                };

                let init_result = (|| -> Result<(), String> {
                    engine_a.init_ubgi()?;
                    engine_b.init_ubgi()?;
                    engine_a.set_variant(worker_variant)?;
                    engine_b.set_variant(worker_variant)?;
                    Ok(())
                })();

                if let Err(err) = init_result {
                    let _ = tx.send(WorkerMessage::Error(format!(
                        "worker {} engine init failed: {}",
                        worker_id + 1,
                        err
                    )));
                    engine_a.quit();
                    engine_b.quit();
                    let _ = tx.send(WorkerMessage::Done);
                    return;
                }

                for game_idx in (worker_id..games).step_by(workers) {
                    if let Some(dir) = &worker_ubgi_dir {
                        let a_path = dir.join(format!("game_{:05}.engine-a.log", game_idx + 1));
                        let b_path = dir.join(format!("game_{:05}.engine-b.log", game_idx + 1));
                        if let Err(err) = engine_a.set_ubgi_log_path(Some(&a_path)) {
                            let _ = tx.send(WorkerMessage::Error(format!(
                                "worker {} game {} UBGI log setup(A) failed: {}",
                                worker_id + 1,
                                game_idx + 1,
                                err
                            )));
                            break;
                        }
                        if let Err(err) = engine_b.set_ubgi_log_path(Some(&b_path)) {
                            let _ = tx.send(WorkerMessage::Error(format!(
                                "worker {} game {} UBGI log setup(B) failed: {}",
                                worker_id + 1,
                                game_idx + 1,
                                err
                            )));
                            break;
                        }
                    }

                    let a_is_x = !(swap_sides && game_idx % 2 == 1);
                    if let Err(err) = engine_a.new_game() {
                        let _ = tx.send(WorkerMessage::Error(format!(
                            "worker {} game {} new_game(A) failed: {}",
                            worker_id + 1,
                            game_idx + 1,
                            err
                        )));
                        break;
                    }
                    if let Err(err) = engine_b.new_game() {
                        let _ = tx.send(WorkerMessage::Error(format!(
                            "worker {} game {} new_game(B) failed: {}",
                            worker_id + 1,
                            game_idx + 1,
                            err
                        )));
                        break;
                    }

                    let mut dice_gen = FastrandDice::with_seed(seed_for_game(base_seed, game_idx));
                    match play_game(
                        worker_variant,
                        max_plies,
                        &mut dice_gen,
                        &mut engine_a,
                        &mut engine_b,
                        a_is_x,
                    ) {
                        Ok(result) => {
                            let _ = tx.send(WorkerMessage::Game(CompletedGame {
                                game_idx,
                                a_is_x,
                                result,
                            }));
                        }
                        Err(err) => {
                            let _ = tx.send(WorkerMessage::Error(format!(
                                "worker {} game {} failed: {}",
                                worker_id + 1,
                                game_idx + 1,
                                err
                            )));
                            break;
                        }
                    }
                }

                engine_a.quit();
                engine_b.quit();
                let _ = tx.send(WorkerMessage::Done);
            });
        }
        drop(tx);

        let mut done_workers = 0usize;
        let mut done_games = 0usize;
        let mut run_error: Option<String> = None;

        while done_workers < workers {
            let msg = match rx.recv() {
                Ok(msg) => msg,
                Err(_) => break,
            };
            match msg {
                WorkerMessage::Done => {
                    done_workers += 1;
                }
                WorkerMessage::Error(err) => {
                    if run_error.is_none() {
                        run_error = Some(err);
                    }
                }
                WorkerMessage::Game(done) => {
                    done_games += 1;
                    if run_error.is_none() {
                        let snapshot = process_completed_game(
                            &done, cfg, &trace_dir, &mut csv, &mut stats, run_start, &display,
                            done_games,
                        )?;
                        on_game_done(&snapshot)?;
                    }
                }
            }
        }

        if let Some(err) = run_error {
            return Err(err);
        }

        Ok(())
    })?;

    display.finish();

    let elapsed = run_start.elapsed();
    let (line_engines, line_result, line_rate, line_decide, line_class, line_sides) =
        render_status_lines(stats.status_view(
            &cfg.engine_a.name,
            &cfg.engine_b.name,
            cfg.games,
            elapsed,
        ));

    Ok(RunSummary {
        line_engines,
        line_result,
        line_rate,
        line_decide,
        line_class,
        line_sides,
    })
}

struct DuelGameResult {
    winner_x: Option<bool>,
    points_x: f32,
    points_o: f32,
    plies: usize,
    a_decisions: usize,
    b_decisions: usize,
    a_decision_sec: f64,
    b_decision_sec: f64,
    trace_lines: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn process_completed_game(
    done: &CompletedGame,
    cfg: &DuelConfig,
    trace_dir: &std::path::Path,
    csv: &mut BufWriter<fs::File>,
    stats: &mut DuelStats,
    run_start: Instant,
    display: &StatusDisplay,
    done_games: usize,
) -> Result<ProgressSnapshot, String> {
    let game_idx = done.game_idx;
    let a_is_x = done.a_is_x;
    let result = &done.result;

    let trace_path = trace_dir.join(format!("game_{:05}.log", game_idx + 1));
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
    csv.flush().map_err(|e| e.to_string())?;

    let elapsed = run_start.elapsed();
    let (line_engines, line_result, line_rate, line_decide, line_class, line_sides) =
        render_status_lines(stats.status_view(
            &cfg.engine_a.name,
            &cfg.engine_b.name,
            done_games,
            elapsed,
        ));
    let lines = StatusLines {
        line_engines,
        line_result,
        line_rate,
        line_decide,
        line_class,
        line_sides,
    };
    display.update(done_games, &lines);

    Ok(ProgressSnapshot { done_games, lines })
}

fn seed_for_game(base_seed: u64, game_idx: usize) -> u64 {
    let mut z = base_seed.wrapping_add((game_idx as u64).wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

fn ubgi_log_enabled(mode: &str) -> bool {
    matches!(mode.trim().to_ascii_lowercase().as_str(), "full" | "trace")
}

fn play_game(
    variant: Variant,
    max_plies: usize,
    dice_gen: &mut FastrandDice,
    engine_a: &mut EngineProcess,
    engine_b: &mut EngineProcess,
    a_is_x: bool,
) -> Result<DuelGameResult, String> {
    let mut game = Game::new(variant);
    let mut a_decisions = 0usize;
    let mut b_decisions = 0usize;
    let mut a_decision_sec = 0f64;
    let mut b_decision_sec = 0f64;
    let mut trace_lines = Vec::new();

    for ply in 0..max_plies {
        let dice = if ply == 0 {
            dice_gen.roll_mixed()
        } else {
            dice_gen.roll()
        };
        let legal = game.legal_positions(&dice);
        if legal.is_empty() {
            return Ok(DuelGameResult {
                winner_x: None,
                points_x: 0.0,
                points_o: 0.0,
                plies: ply,
                a_decisions,
                b_decisions,
                a_decision_sec,
                b_decision_sec,
                trace_lines,
            });
        }
        let legal_ids: Vec<String> = legal.iter().map(|p| gnuid::encode(*p)).collect();
        let position_id = gnuid::encode(game.position());
        let x_to_move = game.position().turn();
        let a_to_move = x_to_move == a_is_x;

        let decision_start = Instant::now();
        let chosen_move_raw = if a_to_move {
            let picked = engine_a.choose_move(&position_id, dice, x_to_move)?;
            a_decisions += 1;
            a_decision_sec += decision_start.elapsed().as_secs_f64();
            picked
        } else {
            let picked = engine_b.choose_move(&position_id, dice, x_to_move)?;
            b_decisions += 1;
            b_decision_sec += decision_start.elapsed().as_secs_f64();
            picked
        };

        let chosen_move = normalize_move_text(&chosen_move_raw)
            .ok_or_else(|| format!("engine returned invalid move text: {chosen_move_raw}"))?;

        let (d1, d2) = match dice {
            Dice::Double(d) => (d, d),
            Dice::Mixed(m) => (m.big(), m.small()),
        };

        trace_lines.push(format!(
            "ply={} turn={} dice={}/{} pos={} choice={} legal_count={}",
            ply + 1,
            if a_to_move { "A" } else { "B" },
            d1,
            d2,
            position_id,
            chosen_move,
            legal.len(),
        ));
        if chosen_move != chosen_move_raw {
            trace_lines.push(format!(
                "choice_raw={} choice_canonical={}",
                chosen_move_raw, chosen_move
            ));
        }

        let next = match game.position().apply_move(dice, &chosen_move) {
            Some(pos) => pos,
            None => {
                let preview = legal_ids
                    .iter()
                    .take(12)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(",");
                return Err(format!(
                    "engine returned illegal move: turn={} pos={} dice={}/{} choice_raw={} choice={} legal_count={} legal_preview={}",
                    if a_to_move { "A" } else { "B" },
                    position_id,
                    d1,
                    d2,
                    chosen_move_raw,
                    chosen_move,
                    legal_ids.len(),
                    preview,
                ));
            }
        };

        if !legal
            .iter()
            .any(|candidate| gnuid::encode(*candidate) == gnuid::encode(next))
        {
            return Err(format!(
                "engine returned move not in legal children: turn={} pos={} dice={}/{} choice_raw={} choice={}",
                if a_to_move { "A" } else { "B" },
                position_id,
                d1,
                d2,
                chosen_move_raw,
                chosen_move,
            ));
        }

        game.set_position(next)
            .map_err(|e| format!("failed to set position: {e}"))?;

        if let GameState::GameOver(result) = next.game_state() {
            let magnitude = result.value().abs();
            let winner_is_x = x_to_move;
            let (points_x, points_o) = if winner_is_x {
                (magnitude, -magnitude)
            } else {
                (-magnitude, magnitude)
            };
            return Ok(DuelGameResult {
                winner_x: Some(winner_is_x),
                points_x,
                points_o,
                plies: ply + 1,
                a_decisions,
                b_decisions,
                a_decision_sec,
                b_decision_sec,
                trace_lines,
            });
        }
    }

    Ok(DuelGameResult {
        winner_x: None,
        points_x: 0.0,
        points_o: 0.0,
        plies: max_plies,
        a_decisions,
        b_decisions,
        a_decision_sec,
        b_decision_sec,
        trace_lines,
    })
}
