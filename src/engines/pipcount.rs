use bkgm::dice::Dice;
use bkgm::{Game, State, VariantPosition};

use super::runtime::{run_ubgi_loop, UbgiAdapter};

pub fn run(_args: &[String]) -> Result<(), String> {
    let mut adapter = PipcountAdapter;
    run_ubgi_loop(&mut adapter);
    Ok(())
}

struct PipcountAdapter;

impl UbgiAdapter for PipcountAdapter {
    fn id_name(&self) -> &'static str {
        "pipcount_engine 0.1"
    }

    fn id_version(&self) -> &'static str {
        "0.1"
    }

    fn choose_move(&mut self, game: &Game, dice: Dice) -> Result<String, String> {
        let legal_moves = game
            .position()
            .legal_moves(dice)
            .map_err(|err| format!("move_encode {err}"))?;
        if legal_moves.is_empty() {
            return Err("no_encodable_legal_moves".to_string());
        }

        let mover_is_x = game.position().turn();
        let mut best_idx = 0usize;
        let mut best_score = evaluate_position(legal_moves[0].1, mover_is_x);
        for (idx, (_, pos)) in legal_moves.iter().enumerate().skip(1) {
            let score = evaluate_position(*pos, mover_is_x);
            if score > best_score {
                best_score = score;
                best_idx = idx;
            }
        }

        Ok(legal_moves[best_idx].0.to_string())
    }
}

fn evaluate_position(position: VariantPosition, mover_is_x: bool) -> f32 {
    match position {
        VariantPosition::Backgammon(p) => eval_state(p, mover_is_x),
        VariantPosition::Nackgammon(p) => eval_state(p, mover_is_x),
        VariantPosition::Longgammon(p) => eval_state(p, mover_is_x),
        VariantPosition::Hypergammon(p) => eval_state(p, mover_is_x),
        VariantPosition::Hypergammon2(p) => eval_state(p, mover_is_x),
        VariantPosition::Hypergammon4(p) => eval_state(p, mover_is_x),
        VariantPosition::Hypergammon5(p) => eval_state(p, mover_is_x),
    }
}

fn eval_state<S: State>(p: S, mover_is_x: bool) -> f32 {
    let mut own_pips = 0f32;
    let mut opp_pips = 0f32;
    for pip in 1..=24 {
        let n = p.pip(pip);
        if mover_is_x {
            if n > 0 {
                own_pips += (n as f32) * (pip as f32);
            } else if n < 0 {
                opp_pips += ((-n) as f32) * ((25 - pip) as f32);
            }
        } else if n < 0 {
            own_pips += ((-n) as f32) * ((25 - pip) as f32);
        } else if n > 0 {
            opp_pips += (n as f32) * (pip as f32);
        }
    }

    let (own_off, opp_off, own_bar, opp_bar) = if mover_is_x {
        (
            p.x_off() as f32,
            p.o_off() as f32,
            p.x_bar() as f32,
            p.o_bar() as f32,
        )
    } else {
        (
            p.o_off() as f32,
            p.x_off() as f32,
            p.o_bar() as f32,
            p.x_bar() as f32,
        )
    };

    let pip_term = (opp_pips - own_pips) * 0.02;
    let off_term = (own_off - opp_off) * 0.6;
    let bar_term = (opp_bar - own_bar) * 0.5;
    pip_term + off_term + bar_term
}
