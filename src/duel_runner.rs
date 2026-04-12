use std::fs;
use std::io::{BufWriter, Write};
use std::time::Instant;

use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::dice_gen::{DiceGen, FastrandDice};
use bkgm::{normalize_move_text, Game, GameState, Variant};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tracing::debug;

use crate::config::DuelConfig;
use crate::engine::EngineProcess;
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

pub fn run_duel(
    cfg: &DuelConfig,
    variant: Variant,
    paths: &RunPaths,
) -> Result<RunSummary, String> {
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

    let mut engine_a = EngineProcess::spawn(&cfg.engine_a)?;
    let mut engine_b = EngineProcess::spawn(&cfg.engine_b)?;
    engine_a.init_ubgi()?;
    engine_b.init_ubgi()?;
    engine_a.set_variant(variant)?;
    engine_b.set_variant(variant)?;

    let mut dice_gen = FastrandDice::with_seed(cfg.seed);
    let mut stats = DuelStats::new();

    let run_start = Instant::now();
    let mp = MultiProgress::new();
    let progress = mp.add(ProgressBar::new(cfg.games as u64));
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

    for game_idx in 0..cfg.games {
        let a_is_x = !(cfg.swap_sides && game_idx % 2 == 1);
        engine_a.new_game()?;
        engine_b.new_game()?;

        let result = play_game(
            variant,
            cfg.max_plies,
            &mut dice_gen,
            &mut engine_a,
            &mut engine_b,
            a_is_x,
        )?;

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

        let done = game_idx + 1;
        let elapsed = run_start.elapsed();
        let (line_engines, line_result, line_rate, line_decide, line_class, line_sides) =
            render_status_lines(stats.status_view(
                &cfg.engine_a.name,
                &cfg.engine_b.name,
                done,
                elapsed,
            ));
        progress.set_position(done as u64);
        stats_engines.set_message(line_engines);
        stats_result.set_message(line_result);
        stats_rate.set_message(line_rate);
        stats_decide.set_message(line_decide);
        stats_class.set_message(line_class);
        stats_sides.set_message(line_sides);
    }

    progress.finish_and_clear();
    stats_engines.finish_and_clear();
    stats_result.finish_and_clear();
    stats_rate.finish_and_clear();
    stats_decide.finish_and_clear();
    stats_class.finish_and_clear();
    stats_sides.finish_and_clear();

    engine_a.quit();
    engine_b.quit();

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
