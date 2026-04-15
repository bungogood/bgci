use std::time::Instant;

use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::dice_gen::{DiceGen, FastrandDice};
use bkgm::{Game, GameState, Variant, normalize_move_text};

use crate::engine::EngineProcess;

pub(crate) struct DuelGameResult {
    pub(crate) winner_x: Option<bool>,
    pub(crate) points_x: f32,
    pub(crate) points_o: f32,
    pub(crate) plies: usize,
    pub(crate) a_decisions: usize,
    pub(crate) b_decisions: usize,
    pub(crate) a_decision_sec: f64,
    pub(crate) b_decision_sec: f64,
    pub(crate) trace_lines: Vec<String>,
    pub(crate) plies_data: Vec<PlyRecord>,
}

#[derive(Clone)]
pub(crate) struct PlyRecord {
    pub(crate) turn_a: bool,
    pub(crate) die_1: usize,
    pub(crate) die_2: usize,
    pub(crate) move_text: String,
}

pub(crate) fn seed_for_game(base_seed: u64, game_idx: usize) -> u64 {
    let mut z = base_seed.wrapping_add((game_idx as u64).wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

pub(crate) fn play_game(
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
    let mut plies_data = Vec::new();

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
                plies_data,
            });
        }
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

        plies_data.push(PlyRecord {
            turn_a: a_to_move,
            die_1: d1,
            die_2: d2,
            move_text: chosen_move.clone(),
        });

        let next = match game.position().apply_move(dice, &chosen_move) {
            Some(pos) => pos,
            None => {
                let legal_ids: Vec<String> = legal.iter().map(|p| gnuid::encode(*p)).collect();
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

        if !legal.contains(&next) {
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
                plies_data,
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
        plies_data,
    })
}
