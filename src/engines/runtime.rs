use std::io::{self, BufRead, Write};

use crate::common::parse_variant_setoption;
use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::{Game, Variant};

pub trait UbgiAdapter {
    fn id_name(&self) -> &'static str;
    fn id_version(&self) -> &'static str;
    fn on_ready(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn choose_move(&mut self, game: &Game, dice: Dice) -> Result<String, String>;
}

pub fn run_ubgi_loop(adapter: &mut impl UbgiAdapter) {
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
            reply(&mut stdout, &format!("id name {}", adapter.id_name()));
            reply(&mut stdout, "id author bgci");
            reply(&mut stdout, &format!("id version {}", adapter.id_version()));
            reply(
                &mut stdout,
                "option name Variant type combo default backgammon var backgammon var nackgammon var longgammon var hypergammon var hypergammon2 var hypergammon4 var hypergammon5",
            );
            reply(&mut stdout, "ubgiok");
            continue;
        }

        if cmd == "isready" {
            match adapter.on_ready() {
                Ok(()) => reply(&mut stdout, "readyok"),
                Err(err) => reply(&mut stdout, &format!("error internal isready_failed {err}")),
            }
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
            match adapter.choose_move(&game, current_dice) {
                Ok(mv) => reply(&mut stdout, &format!("bestmove {mv}")),
                Err(err) => reply(
                    &mut stdout,
                    &format!("error internal move_select_failed {err}"),
                ),
            }
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
