use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::{Game, Variant, normalize_move_text};

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
    pub awkward_legal_probes_passed: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy)]
enum ProbeExpectation {
    Token(&'static str),
    LegalChild,
}

struct ProbeSpec {
    phase: &'static str,
    position_id: &'static str,
    dice: Dice,
    expect: ProbeExpectation,
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
        awkward_legal_probes_passed: 0,
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
        if line.starts_with("key ") {
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

    for (name, value) in &engine_cfg.options {
        engine.send_command(&format!("set {name} {value}"))?;
        engine.send_command("isready")?;
        let _ = wait_readyok_optional(&mut engine, &mut report.errors, &format!("set {name}"));
    }

    engine.send_command("newgame")?;
    engine.send_command("isready")?;
    report.supports_newgame = wait_readyok(&mut engine, &mut report.errors, "newgame");

    engine.send_command("set game.variant backgammon")?;
    engine.send_command("isready")?;
    let _ = wait_readyok(&mut engine, &mut report.errors, "set game.variant");

    let game = Game::new(variant);
    let start_id = gnuid::encode(game.position());
    engine.send_command(&format!("position gnubgid {start_id}"))?;
    report.supports_position = true;

    let dice = Dice::new(6, 1);
    engine.send_command("dice 6 1")?;
    report.supports_dice = true;

    let legal_moves = game
        .position()
        .legal_moves(dice)
        .map_err(|err| err.to_string())?;
    let legal_ids: Vec<String> = game
        .legal_positions(&dice)
        .iter()
        .map(|p| gnuid::encode(*p))
        .collect();
    report.legal_preview = legal_moves.iter().take(8).map(|m| m.0.clone()).collect();

    engine.send_command("go chequer")?;
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
                            "go chequer: illegal bestmove '{}' (canonical '{}')",
                            mv.trim(),
                            c
                        )),
                    }
                }
                None => report
                    .errors
                    .push(format!("go chequer: unparsable bestmove '{}'", mv.trim())),
            }
            break;
        }
        if line.starts_with("best") {
            report.errors.push(format!(
                "go chequer: expected bestmove payload, got '{line}'"
            ));
            break;
        }
        if line.starts_with("error ") {
            report.errors.push(format!("go chequer: {line}"));
            break;
        }
    }

    let probes = [
        ProbeSpec {
            phase: "bar-notation probe",
            position_id: "Np7BQSCYZ/AAWA",
            dice: Dice::new(5, 3),
            expect: ProbeExpectation::Token("bar"),
        },
        ProbeSpec {
            phase: "off-notation probe",
            position_id: "/z0AADDeaxsAAA",
            dice: Dice::new(5, 5),
            expect: ProbeExpectation::Token("off"),
        },
        ProbeSpec {
            phase: "awkward-legal probe: bearoff must use both dice",
            position_id: "t02oASACAAAAAA",
            dice: Dice::new(5, 1),
            expect: ProbeExpectation::LegalChild,
        },
    ];

    for probe in probes {
        run_probe(&mut report, &mut engine, variant, &probe);
    }

    engine.quit();
    Ok(report)
}

fn run_probe(
    report: &mut CheckReport,
    engine: &mut EngineProcess,
    variant: Variant,
    probe: &ProbeSpec,
) {
    let position_id = probe.position_id;
    let dice = probe.dice;
    let phase = probe.phase;

    let Ok(position) = gnuid::decode(variant, position_id) else {
        report.errors.push(format!(
            "{phase}: invalid probe position id '{position_id}'"
        ));
        return;
    };

    let mut game = Game::new(variant);
    if let Err(err) = game.set_position(position) {
        report
            .errors
            .push(format!("{phase}: failed to set probe position: {err}"));
        return;
    }

    let legal_ids: Vec<String> = match game.legal_positions(&dice) {
        positions if !positions.is_empty() => positions.iter().map(|p| gnuid::encode(*p)).collect(),
        _ => {
            report.errors.push(format!("{phase}: no legal positions"));
            return;
        }
    };

    let Some(mv_raw) = probe_move_notation(
        engine,
        variant,
        position_id,
        dice,
        phase,
        &mut report.errors,
    ) else {
        return;
    };

    if contains_numeric_bar_off_alias(&mv_raw) {
        report.numeric_bar_off_alias_seen = true;
        report
            .errors
            .push(format!("{phase}: numeric alias in bestmove '{mv_raw}'"));
    }

    match probe.expect {
        ProbeExpectation::Token(expected_token) => {
            let has_expected = contains_token(&mv_raw, expected_token);
            if expected_token.eq_ignore_ascii_case("bar") {
                report.bar_notation_ok = has_expected;
            } else if expected_token.eq_ignore_ascii_case("off") {
                report.off_notation_ok = has_expected;
            }
            if !has_expected {
                report.errors.push(format!(
                    "{phase}: expected '{expected_token}' token in bestmove '{mv_raw}'"
                ));
            }
        }
        ProbeExpectation::LegalChild => {
            let Some(canonical) = normalize_move_text(&mv_raw) else {
                report
                    .errors
                    .push(format!("{phase}: unparsable bestmove '{mv_raw}'"));
                return;
            };

            let Some(next) = game.position().apply_move(dice, &canonical) else {
                report.errors.push(format!(
                    "{phase}: bestmove not applicable '{}' (canonical '{}')",
                    mv_raw, canonical
                ));
                return;
            };

            let next_id = gnuid::encode(next);
            if legal_ids.iter().any(|id| id == &next_id) {
                report.awkward_legal_probes_passed += 1;
            } else {
                report.errors.push(format!(
                    "{phase}: engine returned move not in legal children (pos={position_id} dice={dice} choice_raw={mv_raw} choice={canonical})"
                ));
            }
        }
    }
}

fn probe_move_notation(
    engine: &mut EngineProcess,
    variant: Variant,
    position_id: &str,
    dice: Dice,
    phase: &str,
    errors: &mut Vec<String>,
) -> Option<String> {
    let Ok(position) = gnuid::decode(variant, position_id) else {
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
    if let Err(err) = engine.send_command("go chequer") {
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

fn wait_readyok_optional(
    engine: &mut EngineProcess,
    errors: &mut Vec<String>,
    phase: &str,
) -> bool {
    loop {
        match engine.read_response() {
            Ok(line) if line == "readyok" => return true,
            Ok(line) if line.starts_with("error ") => {
                eprintln!("warning: {phase} not supported ({line}); continuing");
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
