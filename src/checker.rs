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
    pub bar_notation_ok: bool,
    pub off_notation_ok: bool,
    pub numeric_bar_off_alias_seen: bool,
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
        bar_notation_ok: false,
        off_notation_ok: false,
        numeric_bar_off_alias_seen: false,
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

    apply_notation_probe(
        &mut report,
        &mut engine,
        variant,
        "Np7BQSCYZ/AAWA",
        Dice::new(5, 3),
        "bar-notation probe",
        "bar",
    );
    apply_notation_probe(
        &mut report,
        &mut engine,
        variant,
        "/z0AADDeaxsAAA",
        Dice::new(5, 5),
        "off-notation probe",
        "off",
    );

    engine.quit();
    Ok(report)
}

fn probe_move_notation(
    engine: &mut EngineProcess,
    variant: Variant,
    position_id: &str,
    dice: Dice,
    phase: &str,
    errors: &mut Vec<String>,
) -> Option<String> {
    let Some(position) = gnuid::decode(variant, position_id) else {
        errors.push(format!(
            "{phase}: invalid probe position id '{position_id}'"
        ));
        return None;
    };

    let mut game = Game::new(variant);
    if let Err(err) = game.set_position(position) {
        errors.push(format!("{phase}: failed to set probe position: {err}"));
        return None;
    }
    let legal = match game.position().legal_moves(dice) {
        Ok(moves) => moves,
        Err(err) => {
            errors.push(format!("{phase}: failed to derive legal moves: {err}"));
            return None;
        }
    };
    if legal.is_empty() {
        errors.push(format!("{phase}: probe has no legal moves"));
        return None;
    }

    if let Err(err) = engine.send_command(&format!("position gnubgid {position_id}")) {
        errors.push(format!("{phase}: {err}"));
        return None;
    }
    let (d1, d2) = match dice {
        Dice::Double(d) => (d, d),
        Dice::Mixed(m) => (m.big(), m.small()),
    };
    if let Err(err) = engine.send_command(&format!("dice {d1} {d2}")) {
        errors.push(format!("{phase}: {err}"));
        return None;
    }
    if let Err(err) = engine.send_command("go role chequer") {
        errors.push(format!("{phase}: {err}"));
        return None;
    }

    loop {
        match engine.read_response() {
            Ok(line) => {
                if let Some(mv) = line.strip_prefix("bestmove ") {
                    return Some(mv.trim().to_string());
                }
                if line.starts_with("best") {
                    errors.push(format!("{phase}: expected bestmove payload, got '{line}'"));
                    return None;
                }
                if line.starts_with("error ") {
                    errors.push(format!("{phase}: {line}"));
                    return None;
                }
            }
            Err(err) => {
                errors.push(format!("{phase}: {err}"));
                return None;
            }
        }
    }
}

fn apply_notation_probe(
    report: &mut CheckReport,
    engine: &mut EngineProcess,
    variant: Variant,
    position_id: &str,
    dice: Dice,
    phase: &str,
    expected_token: &str,
) {
    let probed = probe_move_notation(
        engine,
        variant,
        position_id,
        dice,
        phase,
        &mut report.errors,
    );
    let Some(raw) = probed else {
        return;
    };

    if contains_numeric_bar_off_alias(&raw) {
        report.numeric_bar_off_alias_seen = true;
        report
            .errors
            .push(format!("{phase}: numeric alias in bestmove '{raw}'"));
    }

    let has_expected = contains_token(&raw, expected_token);
    if expected_token.eq_ignore_ascii_case("bar") {
        report.bar_notation_ok = has_expected;
    } else if expected_token.eq_ignore_ascii_case("off") {
        report.off_notation_ok = has_expected;
    }

    if !has_expected {
        report.errors.push(format!(
            "{phase}: expected '{expected_token}' token in bestmove '{raw}'"
        ));
    }
}

fn contains_numeric_bar_off_alias(raw: &str) -> bool {
    raw.split_whitespace().any(|token| {
        let cleaned = token.replace('*', "");
        let parts: Vec<&str> = cleaned.split('/').collect();
        parts.iter().any(|p| *p == "25" || *p == "0")
    })
}

fn contains_token(raw: &str, expected: &str) -> bool {
    raw.split_whitespace().any(|token| {
        let cleaned = token.replace('*', "");
        cleaned.split('/').any(|p| p.eq_ignore_ascii_case(expected))
    })
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
