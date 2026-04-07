use std::io::{self, BufRead, Write};

use bkgm::dice::Dice;
use bkgm::dice_gen::{DiceGen, FastrandDice};
use bkgm::{Game, Variant, VariantPosition};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut game = Game::new(Variant::Backgammon);
    let mut dice: Option<Dice> = None;
    let mut rng = FastrandDice::new();

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let cmd = line.trim();
        if cmd.is_empty() {
            continue;
        }

        if cmd == "ubgi" {
            reply(&mut stdout, "id name random_engine 0.1");
            reply(&mut stdout, "id author bgci");
            reply(&mut stdout, "id version 0.1");
            reply(&mut stdout, "ubgiok");
            continue;
        }

        if cmd == "isready" {
            reply(&mut stdout, "readyok");
            continue;
        }

        if cmd == "newgame" {
            game = Game::new(Variant::Backgammon);
            dice = None;
            continue;
        }

        if let Some(id) = cmd.strip_prefix("position gnubgid ") {
            match Variant::Backgammon.from_position_id(id.trim()) {
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
            let legal = game.legal_positions(&current_dice);
            if legal.is_empty() {
                reply(&mut stdout, "error missing_context legal_moves");
                continue;
            }
            let index = rng.choose_index(&vec![1.0; legal.len()]);
            let chosen: VariantPosition = legal[index];
            reply(&mut stdout, &format!("bestmove {}", chosen.position_id()));
            reply(&mut stdout, &format!("bestmoveid {}", chosen.position_id()));
            continue;
        }

        if cmd == "quit" {
            break;
        }

        reply(&mut stdout, "error unknown_command");
    }
}

fn reply(out: &mut impl Write, line: &str) {
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}
