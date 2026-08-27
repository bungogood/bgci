use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use bkgm::Game;
use bkgm::codecs::gnuid;
use bkgm::dice::Dice;
use bkgm::ubgi::{
    OptionSpec, OptionValue, UbgiEngine, UbgiError, UbgiMove, parse_ubgi_move, run_ubgi_stdio,
};

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
            .stderr(Stdio::null())
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
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }

        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();

        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }

        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for GnubgSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) fn run(_args: &[String]) -> Result<(), String> {
    let mut adapter = GnubgCliAdapter {
        gnubg_bin: resolve_gnubg_bin(),
        gnubg_pkgdatadir: resolve_gnubg_pkgdatadir(),
        ply: 3,
        session: None,
    };
    run_ubgi_stdio(&mut adapter);
    if let Some(s) = adapter.session.as_mut() {
        s.shutdown();
    }
    Ok(())
}

struct GnubgCliAdapter {
    gnubg_bin: String,
    gnubg_pkgdatadir: Option<String>,
    ply: i64,
    session: Option<GnubgSession>,
}

impl UbgiEngine for GnubgCliAdapter {
    fn id_name(&self) -> &'static str {
        "gnubg_engine 0.3"
    }

    fn id_version(&self) -> &'static str {
        "0.3"
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::integer("engine.ply", 1, 8, 3)]
    }

    fn get(&self, key: &str) -> Option<OptionValue> {
        (key == "engine.ply").then_some(OptionValue::Int(self.ply))
    }

    fn set(&mut self, key: &str, value: &OptionValue) -> Result<(), UbgiError> {
        match (key, value) {
            ("engine.ply", OptionValue::Int(ply @ 1..=8)) => {
                self.ply = *ply;
                if let Some(session) = self.session.as_mut() {
                    configure_ply(session, self.ply)?;
                }
                Ok(())
            }
            ("engine.ply", _) => Err(UbgiError::bad_value("engine.ply must_be 1..8")),
            _ => Err(UbgiError::unsupported("key")),
        }
    }

    fn on_ready(&mut self) -> Result<(), UbgiError> {
        let session = ensure_session(
            &mut self.session,
            &self.gnubg_bin,
            self.gnubg_pkgdatadir.as_deref(),
        )?;
        configure_ply(session, self.ply)
    }

    fn choose_move(&mut self, game: &Game, dice: Dice) -> Result<UbgiMove, UbgiError> {
        let legal = game.legal_positions(&dice);
        if legal.is_empty() {
            return Err(UbgiError::bad_state("missing_context legal_moves"));
        }
        let legal_moves = game
            .position()
            .legal_moves(dice)
            .map_err(|err| format!("move_encode {err}"))?;

        let session_ref = ensure_session(
            &mut self.session,
            &self.gnubg_bin,
            self.gnubg_pkgdatadir.as_deref(),
        )
        .map_err(|err| format!("gnubg_start_failed {err}"))?;

        let legal_ids: Vec<String> = legal.iter().map(|p| gnuid::encode(*p)).collect();
        let encodable_ids: Vec<String> = legal_moves
            .iter()
            .map(|(_, pos)| gnuid::encode(*pos))
            .collect();
        if encodable_ids.is_empty() {
            return Err(UbgiError::bad_state("move_encode no_encodable_legal_moves"));
        }
        let x_to_move = game.position().turn();
        let child_x_to_move = !x_to_move;

        let chosen_id =
            choose_best_legal_by_eval(session_ref, &encodable_ids, child_x_to_move, self.ply - 1)
                .map_err(|err| format!("gnubg_eval_select_failed {err}"))?;

        let chosen_idx = legal_ids
            .iter()
            .position(|id| id == &chosen_id)
            .ok_or_else(|| "gnubg_eval_select_failed selected_non_legal_child".to_string())?;
        let chosen_pid = gnuid::encode(legal[chosen_idx]);
        let mv = legal_moves
            .iter()
            .find(|(_, pos)| gnuid::encode(*pos) == chosen_pid)
            .map(|(mv, _)| mv)
            .ok_or_else(|| "move_encode selected_child_not_encodable".to_string())?;
        parse_ubgi_move(mv)
    }
}

fn configure_ply(session: &mut GnubgSession, ply: i64) -> Result<(), UbgiError> {
    let native_ply = ply - 1;
    session
        .run_batch(&[
            format!("set evaluation chequerplay plies {native_ply}"),
            format!("set evaluation cubedecision plies {native_ply}"),
        ])
        .map(|_| ())
        .map_err(|err| UbgiError::bad_state(format!("gnubg_config_failed {err}")))
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
    native_ply: i64,
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
            if let Some(eq) = parse_eval_equity(&segment, native_ply) {
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

fn parse_eval_equity(segment: &str, native_ply: i64) -> Option<f32> {
    let prefix = if native_ply == 0 {
        "static:".to_string()
    } else {
        format!("{native_ply} ply:")
    };
    for line in segment.lines() {
        let t = line.trim_start();
        if !t.starts_with(&prefix) {
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

#[cfg(test)]
mod tests {
    use super::{GnubgCliAdapter, parse_eval_equity};
    use bkgm::ubgi::{OptionValue, UbgiEngine};

    #[test]
    fn accepts_root_inclusive_ply_without_starting_gnubg() {
        let mut adapter = GnubgCliAdapter {
            gnubg_bin: "unused".to_string(),
            gnubg_pkgdatadir: None,
            ply: 3,
            session: None,
        };

        adapter.set("engine.ply", &OptionValue::Int(1)).unwrap();

        assert_eq!(adapter.get("engine.ply"), Some(OptionValue::Int(1)));
        assert!(adapter.set("engine.ply", &OptionValue::Int(0)).is_err());
    }

    #[test]
    fn selects_the_requested_native_evaluation_row() {
        let output = "static: 0.1 0.2\n1 ply: 0.3 0.4\n2 ply: 0.5 0.6\n";
        assert_eq!(parse_eval_equity(output, 0), Some(0.1));
        assert_eq!(parse_eval_equity(output, 1), Some(0.3));
        assert_eq!(parse_eval_equity(output, 2), Some(0.5));
        assert_eq!(parse_eval_equity(output, 3), None);
    }
}
