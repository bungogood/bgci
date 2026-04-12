use bkgm::dice::Dice;
use bkgm::{Game, State, VariantPosition};

use super::runtime::{run_ubgi_loop, UbgiAdapter};

pub fn run(_args: &[String]) -> Result<(), String> {
    let mut adapter = HandcraftedAdapter;
    run_ubgi_loop(&mut adapter);
    Ok(())
}

struct HandcraftedAdapter;

impl UbgiAdapter for HandcraftedAdapter {
    fn id_name(&self) -> &'static str {
        "hureval_engine 0.1"
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

        let mut best_idx = 0usize;
        let mut best_score = evaluate_position(legal_moves[0].1);
        for (idx, (_, pos)) in legal_moves.iter().enumerate().skip(1) {
            let score = evaluate_position(*pos);
            if score > best_score {
                best_score = score;
                best_idx = idx;
            }
        }

        Ok(legal_moves[best_idx].0.to_string())
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
    let mut x_blots = 0f32;
    let mut o_blots = 0f32;
    for pip in 1..=24 {
        let n = p.pip(pip);
        if n > 0 {
            x_pips += (n as f32) * pip as f32;
            if n == 1 {
                x_blots += 1.0;
            }
        } else if n < 0 {
            o_pips += ((-n) as f32) * (25 - pip) as f32;
            if n == -1 {
                o_blots += 1.0;
            }
        }
    }

    x_pips += (p.x_bar() as f32) * 25.0;
    o_pips += (p.o_bar() as f32) * 25.0;

    let x_borne = p.x_off() as f32;
    let o_borne = p.o_off() as f32;

    let x_score = -x_pips + (x_borne * 22.0) - (x_blots * 2.0) - (p.x_bar() as f32) * 3.0;
    let o_score = -o_pips + (o_borne * 22.0) - (o_blots * 2.0) - (p.o_bar() as f32) * 3.0;

    let on_roll_value = if p.turn() {
        x_score - o_score
    } else {
        o_score - x_score
    };

    (on_roll_value / 200.0).clamp(-3.0, 3.0)
}
