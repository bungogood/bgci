use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::{normalize_move_text, Game, Variant};

use crate::config::EngineConfig;
use crate::engine::EngineProcess;

pub struct CheckReport {
    pub engine_name: String,
    pub ids: Vec<String>,
    pub options: Vec<String>,
    pub supports_newgame: bool,
    pub supports_position: bool,
    pub supports_dice: bool,
    pub supports_go_chequer: bool,
    pub bestmove_raw: Option<String>,
    pub bestmove_canonical: Option<String>,
    pub legal_preview: Vec<String>,
    pub errors: Vec<String>,
}

impl CheckReport {
    pub fn is_pass(&self) -> bool {
        self.supports_newgame
            && self.supports_position
            && self.supports_dice
            && self.supports_go_chequer
            && self.errors.is_empty()
    }
}

pub fn run_check(engine_cfg: &EngineConfig, variant: Variant) -> Result<CheckReport, String> {
    let mut engine = EngineProcess::spawn(engine_cfg)?;
    let mut report = CheckReport {
        engine_name: engine_cfg.name.clone(),
        ids: Vec::new(),
        options: Vec::new(),
        supports_newgame: false,
        supports_position: false,
        supports_dice: false,
        supports_go_chequer: false,
        bestmove_raw: None,
        bestmove_canonical: None,
        legal_preview: Vec::new(),
        errors: Vec::new(),
    };

    engine.send_command("ubgi")?;
    loop {
        let line = engine.read_response()?;
        if line == "ubgiok" || line == "readyok" {
            break;
        }
        if line.starts_with("id ") {
            report.ids.push(line);
            continue;
        }
        if line.starts_with("option ") {
            report.options.push(line);
            continue;
        }
        if line.starts_with("error ") {
            report.errors.push(format!("ubgi: {line}"));
            break;
        }
    }

    engine.send_command("isready")?;
    wait_readyok(&mut engine, &mut report.errors, "isready");

    engine.send_command("newgame")?;
    engine.send_command("isready")?;
    report.supports_newgame = wait_readyok(&mut engine, &mut report.errors, "newgame");

    engine.send_command("setoption name Variant value backgammon")?;
    engine.send_command("isready")?;
    let _ = wait_readyok(&mut engine, &mut report.errors, "setoption Variant");

    let game = Game::new(variant);
    let start_id = gnuid::encode(game.position());
    engine.send_command(&format!("position gnubgid {start_id}"))?;
    report.supports_position = true;

    let dice = Dice::new(6, 1);
    engine.send_command("dice 6 1")?;
    report.supports_dice = true;

    let legal_moves = game.position().legal_moves(dice)?;
    let legal_ids: Vec<String> = game
        .legal_positions(&dice)
        .iter()
        .map(|p| gnuid::encode(*p))
        .collect();
    report.legal_preview = legal_moves.iter().take(8).map(|m| m.0.clone()).collect();

    engine.send_command("go role chequer")?;
    loop {
        let line = engine.read_response()?;
        if let Some(mv) = line.strip_prefix("bestmove ") {
            report.supports_go_chequer = true;
            report.bestmove_raw = Some(mv.trim().to_string());
            let canonical = normalize_move_text(mv.trim());
            report.bestmove_canonical = canonical.clone();
            match canonical {
                Some(ref c) => {
                    let applied = game.position().apply_move(dice, c);
                    match applied {
                        Some(next) if legal_ids.iter().any(|id| id == &gnuid::encode(next)) => {}
                        _ => report.errors.push(format!(
                            "go role chequer: illegal bestmove '{}' (canonical '{}')",
                            mv.trim(),
                            c
                        )),
                    }
                }
                None => report.errors.push(format!(
                    "go role chequer: unparsable bestmove '{}'",
                    mv.trim()
                )),
            }
            break;
        }
        if line.starts_with("best") {
            report.errors.push(format!(
                "go role chequer: expected bestmove payload, got '{line}'"
            ));
            break;
        }
        if line.starts_with("error ") {
            report.errors.push(format!("go role chequer: {line}"));
            break;
        }
    }

    engine.quit();
    Ok(report)
}

fn wait_readyok(engine: &mut EngineProcess, errors: &mut Vec<String>, phase: &str) -> bool {
    loop {
        match engine.read_response() {
            Ok(line) if line == "readyok" => return true,
            Ok(line) if line.starts_with("error ") => {
                errors.push(format!("{phase}: {line}"));
                return false;
            }
            Ok(_) => continue,
            Err(err) => {
                errors.push(format!("{phase}: {err}"));
                return false;
            }
        }
    }
}
