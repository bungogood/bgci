use std::io::{self, BufRead, Write};

use bgci::common::parse_variant_setoption;
use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::{Game, State, Variant, VariantPosition};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut variant = Variant::Backgammon;
    let mut game = Game::new(variant);
    let mut dice: Option<Dice> = None;

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let cmd = line.trim();
        if cmd.is_empty() {
            continue;
        }

        if cmd == "ubgi" {
            reply(&mut stdout, "id name pubeval_engine 0.1");
            reply(&mut stdout, "id author bgci");
            reply(&mut stdout, "id version 0.1");
            reply(
                &mut stdout,
                "option name Variant type combo default backgammon var backgammon var nackgammon var longgammon var hypergammon var hypergammon2 var hypergammon4 var hypergammon5",
            );
            reply(&mut stdout, "ubgiok");
            continue;
        }

        if cmd == "isready" {
            reply(&mut stdout, "readyok");
            continue;
        }

        if cmd == "newgame" {
            game = Game::new(variant);
            dice = None;
            continue;
        }

        if let Some(parsed_variant) = parse_variant_setoption(cmd) {
            match parsed_variant {
                Ok(v) => {
                    variant = v;
                    game = Game::new(variant);
                }
                Err(_) => reply(&mut stdout, "error bad_argument variant"),
            }
            continue;
        }

        if let Some(id) = cmd.strip_prefix("position gnubgid ") {
            match gnuid::decode(variant, id.trim()) {
                Some(pos) => {
                    let _ = game.set_position(pos);
                }
                None => reply(&mut stdout, "error bad_argument invalid_position"),
            }
            continue;
        }

        if cmd == "position xgid" || cmd.starts_with("position xgid ") {
            reply(&mut stdout, "error unsupported_feature position_xgid");
            continue;
        }

        if let Some(rest) = cmd.strip_prefix("dice ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() != 2 {
                reply(&mut stdout, "error bad_argument dice");
                continue;
            }
            let d1 = parts[0].parse::<usize>();
            let d2 = parts[1].parse::<usize>();
            match (d1, d2) {
                (Ok(a), Ok(b)) if (1..=6).contains(&a) && (1..=6).contains(&b) => {
                    dice = Some(Dice::new(a, b));
                }
                _ => reply(&mut stdout, "error bad_argument dice"),
            }
            continue;
        }

        if cmd == "go" || cmd == "go role chequer" {
            let Some(current_dice) = dice else {
                reply(&mut stdout, "error missing_context dice");
                continue;
            };
            let legal_moves = match game.position().legal_moves(current_dice) {
                Ok(moves) => moves,
                Err(err) => {
                    reply(&mut stdout, &format!("error internal move_encode {err}"));
                    continue;
                }
            };
            if legal_moves.is_empty() {
                reply(
                    &mut stdout,
                    "error internal move_encode no_encodable_legal_moves",
                );
                continue;
            }

            let mut best_idx = 0usize;
            let mut best_value = evaluate_position(legal_moves[0].1);
            for (idx, (_, pos)) in legal_moves.iter().enumerate().skip(1) {
                let value = evaluate_position(*pos);
                if value > best_value {
                    best_value = value;
                    best_idx = idx;
                }
            }
            let mv = &legal_moves[best_idx].0;
            reply(&mut stdout, &format!("bestmove {mv}"));
            continue;
        }

        if cmd == "quit" {
            break;
        }

        reply(&mut stdout, "error unknown_command");
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

fn reply(out: &mut impl Write, line: &str) {
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}
