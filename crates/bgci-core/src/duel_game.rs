use std::time::{Duration, Instant};

use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::dice_gen::{DiceGen, FastrandDice};
use bkgm::{Game, GameState, Variant, normalize_move_text};

use crate::engine::EngineProcess;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Participant {
    A,
    B,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnRecord {
    pub participant: Participant,
    pub dice: (u8, u8),
    pub move_text: String,
}

pub(crate) struct DuelGameResult {
    pub(crate) winner_x: Option<bool>,
    pub(crate) points_x: f32,
    pub(crate) points_o: f32,
    pub(crate) plies: usize,
    pub(crate) a_decisions: usize,
    pub(crate) b_decisions: usize,
    pub(crate) a_decision_time: Duration,
    pub(crate) b_decision_time: Duration,
    pub(crate) transcript: Option<Vec<TurnRecord>>,
}

pub(crate) fn seed_for_game(base_seed: u64, game_idx: usize) -> u64 {
    let mut z = base_seed.wrapping_add((game_idx as u64).wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

pub(crate) fn singleton_leg(base_seed: u64, pair_index: usize) -> usize {
    const SINGLETON_LEG_DOMAIN: u64 = 0x5349_4E47_4C45_544F;
    (seed_for_game(base_seed ^ SINGLETON_LEG_DOMAIN, pair_index) & 1) as usize
}

pub(crate) fn play_game(
    variant: Variant,
    max_plies: usize,
    dice_gen: &mut FastrandDice,
    engine_a: &mut EngineProcess,
    engine_b: &mut EngineProcess,
    a_is_x: bool,
    record_transcript: bool,
) -> Result<DuelGameResult, String> {
    let mut game = Game::new(variant);
    let mut a_decisions = 0usize;
    let mut b_decisions = 0usize;
    let mut a_decision_time = Duration::ZERO;
    let mut b_decision_time = Duration::ZERO;
    let mut transcript = record_transcript.then(Vec::new);

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
                a_decision_time,
                b_decision_time,
                transcript,
            });
        }
        let position_id = gnuid::encode(game.position());
        let x_to_move = game.position().turn();
        let a_to_move = x_to_move == a_is_x;

        let decision_start = Instant::now();
        let chosen_move_raw = if a_to_move {
            let picked = engine_a.choose_move(&position_id, dice)?;
            a_decisions += 1;
            a_decision_time += decision_start.elapsed();
            picked
        } else {
            let picked = engine_b.choose_move(&position_id, dice)?;
            b_decisions += 1;
            b_decision_time += decision_start.elapsed();
            picked
        };

        let chosen_move = normalize_move_text(&chosen_move_raw)
            .ok_or_else(|| format!("engine returned invalid move text: {chosen_move_raw}"))?;

        let (d1, d2) = match dice {
            Dice::Double(d) => (d, d),
            Dice::Mixed(m) => (m.big(), m.small()),
        };

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

        if let Some(transcript) = &mut transcript {
            let move_text = game
                .position()
                .encode_move(next, dice)
                .map_err(|error| format!("failed to encode validated move: {error}"))?;
            transcript.push(TurnRecord {
                participant: if a_to_move {
                    Participant::A
                } else {
                    Participant::B
                },
                dice: (d1.max(d2) as u8, d1.min(d2) as u8),
                move_text,
            });
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
                a_decision_time,
                b_decision_time,
                transcript,
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
        a_decision_time,
        b_decision_time,
        transcript,
    })
}

#[cfg(test)]
mod seed_tests {
    use super::{seed_for_game, singleton_leg};

    #[test]
    fn singleton_leg_is_deterministic_domain_separated_and_not_side_fixed() {
        assert_eq!(singleton_leg(42, 3), singleton_leg(42, 3));
        let legs = (0..64)
            .map(|seed| singleton_leg(seed, 0))
            .collect::<Vec<_>>();
        assert!(legs.contains(&0));
        assert!(legs.contains(&1));
        assert_eq!(seed_for_game(42, 0), seed_for_game(42, 0));
    }
}
