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
        let legal_positions = game.legal_positions(&dice);
        if legal_positions.is_empty() {
            return Err("no_encodable_legal_moves".to_string());
        }

        let mut best_idx = 0usize;
        let mut best_score = evaluate_position(legal_positions[0]);
        for (idx, pos) in legal_positions.iter().enumerate().skip(1) {
            let score = evaluate_position(*pos);
            if score > best_score {
                best_score = score;
                best_idx = idx;
            }
        }

        game.position()
            .encode_move(legal_positions[best_idx], dice)
            .map_err(|err| format!("move_encode {err}"))
    }
}

fn evaluate_position(position: VariantPosition) -> f32 {
    match position {
        VariantPosition::Backgammon(p) => eval_state(p),
        VariantPosition::Nackgammon(p) => eval_state(p),
        VariantPosition::Longgammon(p) => eval_state(p),
        VariantPosition::Hypergammon(p) => eval_state(p),
        VariantPosition::Hypergammon2(p) => eval_state(p),
        VariantPosition::Hypergammon4(p) => eval_state(p),
        VariantPosition::Hypergammon5(p) => eval_state(p),
    }
}

fn eval_state<S: State>(p: S) -> f32 {
    let mut x_pips = 0f32;
    let mut o_pips = 0f32;
    for pip in 1..=24 {
        let n = p.pip(pip);
        if n > 0 {
            x_pips += (n as f32) * pip as f32;
        } else if n < 0 {
            o_pips += ((-n) as f32) * (25 - pip) as f32;
        }
    }

    x_pips += (p.x_bar() as f32) * 25.0;
    o_pips += (p.o_bar() as f32) * 25.0;

    let x_off = p.x_off() as f32;
    let o_off = p.o_off() as f32;
    let x_bar = p.x_bar() as f32;
    let o_bar = p.o_bar() as f32;

    let pip_term = (o_pips - x_pips) * 0.02;
    let off_term = (x_off - o_off) * 0.35;
    let bar_term = (o_bar - x_bar) * 0.20;

    let on_roll_value = if p.turn() {
        pip_term + off_term + bar_term
    } else {
        -(pip_term + off_term + bar_term)
    };

    on_roll_value.clamp(-3.0, 3.0)
}
