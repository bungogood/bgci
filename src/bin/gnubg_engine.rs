use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use bgci::common::parse_variant_setoption;
use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::{Game, Variant};

struct GnubgSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    marker_counter: u64,
}

impl GnubgSession {
    fn start(bin: &str, pkgdatadir: Option<&str>) -> Result<Self, String> {
        let mut cmd = Command::new(bin);
        cmd.args(["-t", "-q", "-r"]);
        if let Some(dir) = pkgdatadir {
            cmd.args(["--pkgdatadir", dir]);
        }

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open gnubg stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to open gnubg stdout".to_string())?;

        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            marker_counter: 1,
        };

        let _ = session.run_batch(&[
            "set player 0 human".to_string(),
            "set player 1 human".to_string(),
            "set player 0 name p_x".to_string(),
            "set player 1 name p_o".to_string(),
            "set display off".to_string(),
        ])?;

        Ok(session)
    }

    fn run_batch(&mut self, commands: &[String]) -> Result<String, String> {
        let marker = format!("__BGCI_END_{}__", self.marker_counter);
        self.marker_counter += 1;

        trace_log(&format!(
            "--> batch start marker={} cmds={:?}",
            marker, commands
        ));

        for command in commands {
            writeln!(self.stdin, "{command}").map_err(|e| format!("write stdin: {e}"))?;
        }
        writeln!(self.stdin, "help {marker}").map_err(|e| format!("write marker: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("flush stdin: {e}"))?;

        let mut output = String::new();
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| format!("read stdout: {e}"))?;
            if n == 0 {
                return Err("gnubg stdout closed".to_string());
            }
            trace_log(&format!("<-- {}", line.trim_end()));
            if line.contains(&marker) {
                trace_log(&format!("<-- batch end marker={}", marker));
                break;
            }
            output.push_str(&line);
        }
        Ok(output)
    }

    fn shutdown(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut variant = Variant::Backgammon;
    let mut game = Game::new(variant);
    let mut dice: Option<Dice> = None;

    let gnubg_bin = resolve_gnubg_bin();
    let gnubg_pkgdatadir = resolve_gnubg_pkgdatadir();

    let mut session: Option<GnubgSession> = None;

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let cmd = line.trim();
        if cmd.is_empty() {
            continue;
        }

        if cmd == "ubgi" {
            reply(&mut stdout, "id name gnubg_engine 0.3");
            reply(&mut stdout, "id author bgci");
            reply(&mut stdout, "id version 0.3");
            reply(
                &mut stdout,
                "option name Variant type combo default backgammon var backgammon var nackgammon var longgammon var hypergammon var hypergammon2 var hypergammon4 var hypergammon5",
            );
            reply(&mut stdout, "ubgiok");
            continue;
        }

        if cmd == "isready" {
            match ensure_session(&mut session, &gnubg_bin, gnubg_pkgdatadir.as_deref()) {
                Ok(_) => reply(&mut stdout, "readyok"),
                Err(err) => reply(
                    &mut stdout,
                    &format!("error internal gnubg_start_failed {err}"),
                ),
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
            let legal = game.legal_positions(&current_dice);
            if legal.is_empty() {
                reply(&mut stdout, "error missing_context legal_moves");
                continue;
            }
            let legal_moves = match game.position().legal_moves(current_dice) {
                Ok(moves) => moves,
                Err(err) => {
                    reply(&mut stdout, &format!("error internal move_encode {err}"));
                    continue;
                }
            };

            let session_ref =
                match ensure_session(&mut session, &gnubg_bin, gnubg_pkgdatadir.as_deref()) {
                    Ok(s) => s,
                    Err(err) => {
                        reply(
                            &mut stdout,
                            &format!("error internal gnubg_start_failed {err}"),
                        );
                        continue;
                    }
                };

            let _ = current_dice;

            let legal_ids: Vec<String> = legal.iter().map(|p| gnuid::encode(*p)).collect();
            let encodable_ids: Vec<String> = legal_moves
                .iter()
                .map(|(_, pos)| gnuid::encode(*pos))
                .collect();
            if encodable_ids.is_empty() {
                reply(
                    &mut stdout,
                    "error internal move_encode no_encodable_legal_moves",
                );
                continue;
            }
            let x_to_move = game.position().turn();
            let child_x_to_move = !x_to_move;

            let chosen_id =
                match choose_best_legal_by_eval(session_ref, &encodable_ids, child_x_to_move) {
                    Ok(id) => id,
                    Err(err) => {
                        reply(
                            &mut stdout,
                            &format!("error internal gnubg_eval_select_failed {err}"),
                        );
                        continue;
                    }
                };

            let chosen_idx = match legal_ids.iter().position(|id| id == &chosen_id) {
                Some(idx) => idx,
                None => {
                    reply(
                        &mut stdout,
                        "error internal gnubg_eval_select_failed selected_non_legal_child",
                    );
                    continue;
                }
            };
            let chosen_pid = gnuid::encode(legal[chosen_idx]);
            let mv = match legal_moves
                .iter()
                .find(|(_, pos)| gnuid::encode(*pos) == chosen_pid)
            {
                Some((mv, _)) => mv,
                None => {
                    reply(
                        &mut stdout,
                        "error internal move_encode selected_child_not_encodable",
                    );
                    continue;
                }
            };
            reply(&mut stdout, &format!("bestmove {mv}"));
            continue;
        }

        if cmd == "quit" {
            break;
        }

        reply(&mut stdout, "error unknown_command");
    }

    if let Some(mut s) = session {
        s.shutdown();
    }
}

fn ensure_session<'a>(
    session: &'a mut Option<GnubgSession>,
    bin: &str,
    pkgdatadir: Option<&str>,
) -> Result<&'a mut GnubgSession, String> {
    if session.is_none() {
        *session = Some(GnubgSession::start(bin, pkgdatadir)?);
    }
    match session.as_mut() {
        Some(s) => Ok(s),
        None => Err("session_unavailable".to_string()),
    }
}

fn choose_best_legal_by_eval(
    session: &mut GnubgSession,
    legal_ids: &[String],
    child_x_to_move: bool,
) -> Result<String, String> {
    if legal_ids.is_empty() {
        return Err("no_legal_ids".to_string());
    }

    let turn = if child_x_to_move { "p_x" } else { "p_o" };
    let mut commands = vec!["new game".to_string()];
    for (idx, id) in legal_ids.iter().enumerate() {
        commands.push(format!("set board {id}"));
        commands.push(format!("set turn {turn}"));
        commands.push("eval".to_string());
        commands.push(format!("help __BGCI_POS_{}__", idx));
    }
    let out = session.run_batch(&commands)?;

    let mut best_id: Option<String> = None;
    let mut best_eq = f32::INFINITY;
    let mut segment = String::new();
    let mut seg_idx = 0usize;

    for line in out.lines() {
        segment.push_str(line);
        segment.push('\n');

        if line.contains("__BGCI_POS_") {
            if seg_idx >= legal_ids.len() {
                break;
            }
            if let Some(eq) = parse_eval_equity(&segment) {
                trace_log(&format!(
                    "eval-choice idx={} id={} eq={:.6}",
                    seg_idx, legal_ids[seg_idx], eq
                ));
                if eq < best_eq {
                    best_eq = eq;
                    best_id = Some(legal_ids[seg_idx].clone());
                }
            } else {
                trace_log(&format!(
                    "eval-choice idx={} id={} eq=<parse-failed> segment={} ",
                    seg_idx,
                    legal_ids[seg_idx],
                    summarize(&segment)
                ));
            }
            seg_idx += 1;
            segment.clear();
        }
    }

    best_id
        .ok_or_else(|| {
            debug_log(&format!(
                "eval-select-parse-failed legal_count={} output={}",
                legal_ids.len(),
                summarize(&out)
            ));
            "eval_select_parse_failed".to_string()
        })
        .map(|id| {
            trace_log(&format!(
                "eval-choice selected id={} best_eq={:.6}",
                id, best_eq
            ));
            id
        })
}

fn parse_eval_equity(segment: &str) -> Option<f32> {
    for line in segment.lines() {
        let t = line.trim_start();
        if !t.starts_with("2 ply:") {
            continue;
        }
        let values: Vec<f32> = t
            .split_whitespace()
            .filter_map(|token| token.parse::<f32>().ok())
            .collect();
        if values.len() >= 2 {
            return Some(values[values.len() - 2]);
        }
    }

    for line in segment.lines() {
        let t = line.trim_start();
        if !t.starts_with("1 ply:") && !t.starts_with("static:") {
            continue;
        }
        let values: Vec<f32> = t
            .split_whitespace()
            .filter_map(|token| token.parse::<f32>().ok())
            .collect();
        if values.len() >= 2 {
            return Some(values[values.len() - 2]);
        }
    }

    None
}

fn reply(out: &mut impl Write, line: &str) {
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn resolve_gnubg_bin() -> String {
    if let Ok(bin) = std::env::var("BGCI_GNUBG_BIN") {
        return bin;
    }
    "gnubg".to_string()
}

fn resolve_gnubg_pkgdatadir() -> Option<String> {
    if let Ok(dir) = std::env::var("BGCI_GNUBG_PKGDATADIR") {
        return Some(dir);
    }

    let candidate = std::env::var("XDG_DATA_HOME")
        .ok()
        .map(|home| format!("{home}/gnubg"))
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| format!("{home}/.local/share/gnubg"))
        });

    if let Some(dir) = candidate {
        let weights = std::path::Path::new(&dir).join("gnubg.weights");
        let wd = std::path::Path::new(&dir).join("gnubg.wd");
        if weights.exists() || wd.exists() {
            return Some(dir);
        }
    }

    None
}

fn summarize(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= 260 {
        return collapsed;
    }
    let head = &collapsed[..130];
    let tail = &collapsed[collapsed.len() - 120..];
    format!("{} ... {}", head, tail)
}

fn debug_log(message: &str) {
    let Ok(path) = std::env::var("BGCI_GNUBG_DEBUG_LOG") else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{message}");
    }
}

fn trace_log(message: &str) {
    let Ok(path) = std::env::var("BGCI_GNUBG_TRACE_LOG") else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{message}");
    }
}
