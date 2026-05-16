use std::io::{self, BufRead, Write};

use crate::common::{parse_variant, variant_name};
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
            reply(&mut stdout, "proto 0.2");
            reply(
                &mut stdout,
                "key game.variant enum backgammon|nackgammon|longgammon|hypergammon|hypergammon2|hypergammon4|hypergammon5 backgammon",
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

        if cmd == "keys" {
            reply(
                &mut stdout,
                "key game.variant enum backgammon|nackgammon|longgammon|hypergammon|hypergammon2|hypergammon4|hypergammon5 backgammon",
            );
            continue;
        }

        if let Some(key) = cmd.strip_prefix("get ") {
            if key.trim() == "game.variant" {
                reply(
                    &mut stdout,
                    &format!("value game.variant {}", variant_name(variant)),
                );
            } else {
                reply(&mut stdout, "error unsupported key");
            }
            continue;
        }

        if let Some(rest) = cmd.strip_prefix("set ") {
            let mut it = rest.splitn(2, ' ');
            let key = it.next().unwrap_or("").trim();
            let value = it.next().unwrap_or("").trim();
            if key.is_empty() || value.is_empty() {
                reply(&mut stdout, "error bad_command set");
                continue;
            }
            if key == "game.variant" {
                match parse_variant(value) {
                    Ok(v) => {
                        variant = v;
                        game = Game::new(variant);
                    }
                    Err(_) => reply(&mut stdout, "error bad_value game.variant"),
                }
            } else {
                reply(&mut stdout, "error unsupported key");
            }
            continue;
        }

        if let Some(id) = cmd.strip_prefix("position gnubgid ") {
            match gnuid::decode(variant, id.trim()) {
                Some(pos) => {
                    let _ = game.set_position(pos);
                }
                None => reply(&mut stdout, "error bad_value position"),
            }
            continue;
        }

        if cmd == "position xgid" || cmd.starts_with("position xgid ") {
            reply(&mut stdout, "error unsupported position.xgid");
            continue;
        }

        if let Some(rest) = cmd.strip_prefix("dice ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() != 2 {
                reply(&mut stdout, "error bad_value dice");
                continue;
            }
            let d1 = parts[0].parse::<usize>();
            let d2 = parts[1].parse::<usize>();
            match (d1, d2) {
                (Ok(a), Ok(b)) if (1..=6).contains(&a) && (1..=6).contains(&b) => {
                    dice = Some(Dice::new(a, b));
                }
                _ => reply(&mut stdout, "error bad_value dice"),
            }
            continue;
        }

        if cmd == "go" || cmd == "go chequer" {
            let Some(current_dice) = dice else {
                reply(&mut stdout, "error bad_state missing.dice");
                continue;
            };
            match adapter.choose_move(&game, current_dice) {
                Ok(mv) => reply(&mut stdout, &format!("bestmove {mv}")),
                Err(err) => reply(&mut stdout, &format!("error bad_state move.select {err}")),
            }
            continue;
        }

        if cmd == "quit" {
            break;
        }

        reply(&mut stdout, "error bad_command unknown");
    }
}

fn reply(out: &mut impl Write, line: &str) {
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}
