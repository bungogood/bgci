use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::{Game, Variant, normalize_move_text};

use crate::common::variant_name;
use crate::config::ResolvedEngine;
use crate::engine::{EngineProcess, ResponseError};
use crate::ubgi;

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

#[derive(Clone, Copy)]
struct ProbeSpec {
    phase: &'static str,
    position_id: &'static str,
    dice: Dice,
    expect: ProbeExpectation,
}

struct PreparedProbe {
    spec: ProbeSpec,
    game: Game,
    legal_ids: Vec<String>,
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

pub fn run_check(engine_cfg: &ResolvedEngine, variant: Variant) -> Result<CheckReport, String> {
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

    engine.send_command(ubgi::CMD_UBGI)?;
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

    engine.send_command(ubgi::CMD_ISREADY)?;
    record_ready(&mut engine, &mut report.errors, "isready", false);

    for (name, value) in engine_cfg.launch.ubgi() {
        engine.send_command(&format!("set {name} {value}"))?;
        engine.send_command(ubgi::CMD_ISREADY)?;
        record_ready(
            &mut engine,
            &mut report.errors,
            &format!("set {name}"),
            true,
        );
    }

    engine.send_command(ubgi::CMD_NEWGAME)?;
    engine.send_command(ubgi::CMD_ISREADY)?;
    report.supports_newgame = record_ready(&mut engine, &mut report.errors, "newgame", false);

    engine.send_command(&format!("set game.variant {}", variant_name(variant)))?;
    engine.send_command(ubgi::CMD_ISREADY)?;
    record_ready(&mut engine, &mut report.errors, "set game.variant", false);

    let game = Game::new(variant);
    let start_id = gnuid::encode(game.position());
    let dice = Dice::new(6, 1);
    engine.send_position_and_dice(&start_id, dice)?;
    report.supports_position = true;
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

    engine.send_command(ubgi::CMD_GO_CHEQUER)?;
    match engine.wait_bestmove() {
        Ok(mv) => {
            report.supports_go_chequer = true;
            report.bestmove_raw = Some(mv.clone());
            let canonical = normalize_move_text(&mv);
            report.bestmove_canonical = canonical.clone();
            match canonical {
                Some(ref c) => {
                    let applied = game.position().apply_move(dice, c);
                    match applied {
                        Some(next) if legal_ids.iter().any(|id| id == &gnuid::encode(next)) => {}
                        _ => report.errors.push(format!(
                            "go chequer: illegal bestmove '{}' (canonical '{}')",
                            mv, c
                        )),
                    }
                }
                None => report
                    .errors
                    .push(format!("go chequer: unparsable bestmove '{mv}'")),
            }
        }
        Err(err) => report_response_error(&mut report.errors, "go chequer", err),
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

    for spec in probes {
        if let Some(probe) = prepare_probe(variant, spec, &mut report.errors) {
            run_probe(&mut report, &mut engine, &probe);
        }
    }

    engine.quit();
    Ok(report)
}

fn run_probe(report: &mut CheckReport, engine: &mut EngineProcess, probe: &PreparedProbe) {
    let ProbeSpec {
        phase,
        position_id,
        dice,
        expect,
    } = probe.spec;

    if let Err(err) = engine.send_position_and_dice(position_id, dice) {
        report.errors.push(format!("{phase}: {err}"));
        return;
    }
    if let Err(err) = engine.send_command(ubgi::CMD_GO_CHEQUER) {
        report.errors.push(format!("{phase}: {err}"));
        return;
    }
    let mv_raw = match engine.wait_bestmove() {
        Ok(mv) => mv,
        Err(err) => {
            report_response_error(&mut report.errors, phase, err);
            return;
        }
    };

    if contains_numeric_bar_off_alias(&mv_raw) {
        report.numeric_bar_off_alias_seen = true;
        report
            .errors
            .push(format!("{phase}: numeric alias in bestmove '{mv_raw}'"));
    }

    match expect {
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

            let Some(next) = probe.game.position().apply_move(dice, &canonical) else {
                report.errors.push(format!(
                    "{phase}: bestmove not applicable '{}' (canonical '{}')",
                    mv_raw, canonical
                ));
                return;
            };

            let next_id = gnuid::encode(next);
            if probe.legal_ids.iter().any(|id| id == &next_id) {
                report.awkward_legal_probes_passed += 1;
            } else {
                report.errors.push(format!(
                    "{phase}: engine returned move not in legal children (pos={position_id} dice={dice} choice_raw={mv_raw} choice={canonical})"
                ));
            }
        }
    }
}

fn prepare_probe(
    variant: Variant,
    spec: ProbeSpec,
    errors: &mut Vec<String>,
) -> Option<PreparedProbe> {
    let ProbeSpec {
        phase,
        position_id,
        dice,
        ..
    } = spec;
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
    let legal_ids: Vec<String> = game
        .legal_positions(&dice)
        .iter()
        .map(|position| gnuid::encode(*position))
        .collect();
    if legal_ids.is_empty() {
        errors.push(format!("{phase}: no legal positions"));
        return None;
    }
    Some(PreparedProbe {
        spec,
        game,
        legal_ids,
    })
}

fn contains_numeric_bar_off_alias(raw: &str) -> bool {
    raw.split_whitespace().any(|token| {
        let cleaned = token.replace('*', "");
        cleaned.split('/').any(|part| part == "25" || part == "0")
    })
}

fn contains_token(raw: &str, expected: &str) -> bool {
    raw.split_whitespace().any(|token| {
        let cleaned = token.replace('*', "");
        cleaned.split('/').any(|p| p.eq_ignore_ascii_case(expected))
    })
}

fn record_ready(
    engine: &mut EngineProcess,
    errors: &mut Vec<String>,
    phase: &str,
    optional: bool,
) -> bool {
    match engine.wait_readyok() {
        Ok(()) => true,
        Err(ResponseError::Engine(line)) if optional => {
            eprintln!("warning: {phase} not supported ({line}); continuing");
            false
        }
        Err(err) => {
            report_response_error(errors, phase, err);
            false
        }
    }
}

fn report_response_error(errors: &mut Vec<String>, phase: &str, err: ResponseError) {
    let diagnostic = match err {
        ResponseError::Engine(line) | ResponseError::Transport(line) => line,
        ResponseError::MalformedBest(line) => {
            format!("expected bestmove payload, got '{line}'")
        }
    };
    errors.push(format!("{phase}: {diagnostic}"));
}
