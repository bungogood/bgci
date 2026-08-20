use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;

use crate::common::variant_name;
use crate::ubgi;
use bkgm::Variant;
use bkgm::dice::Dice;
use bkgm::ubgi::parse_key_line;
use tracing::{debug, error, info};

use crate::config::{ResolvedEngine, canonicalize_resolved_engine_name, resolve_engine_specs};

pub(crate) enum ResponseError {
    Engine(String),
    MalformedBest(String),
    Transport(String),
}

pub fn resolve_and_finalize_engines(specs: &[String]) -> Result<Vec<ResolvedEngine>, String> {
    let mut engines = Vec::with_capacity(specs.len());
    for engine in resolve_engine_specs(specs)? {
        let engine = finalize_resolved_engine(engine);
        if engines
            .iter()
            .any(|existing: &ResolvedEngine| existing.launch == engine.launch)
        {
            return Err(format!("duplicate resolved engine: {}", engine.name));
        }
        engines.push(engine);
    }
    Ok(engines)
}

pub fn finalize_resolved_engine(mut engine: ResolvedEngine) -> ResolvedEngine {
    if let Ok(supported) = probe_supported_engine_options(&engine)
        && !supported.is_empty()
    {
        let unsupported: Vec<String> = engine
            .launch
            .options()
            .keys()
            .filter(|k| !supported.contains(*k))
            .cloned()
            .collect();
        if !unsupported.is_empty() {
            eprintln!(
                "warning: engine '{}' does not support {}; ignoring",
                engine.name,
                unsupported.join(", ")
            );
        }
        engine
            .launch
            .options_mut()
            .retain(|k, _| supported.contains(k));
    }
    canonicalize_resolved_engine_name(&mut engine);
    engine
}

fn probe_supported_engine_options(cfg: &ResolvedEngine) -> Result<HashSet<String>, String> {
    let mut engine = EngineProcess::spawn(cfg)?;
    engine.send_command(ubgi::CMD_UBGI)?;
    let mut keys = HashSet::new();
    loop {
        let line = engine.read_response()?;
        if line == "ubgiok" || line == "readyok" {
            break;
        }
        if let Some(spec) = parse_key_line(&line) {
            keys.insert(spec.name);
        }
    }
    engine.quit();
    Ok(keys)
}

pub struct EngineProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    options: Vec<(String, String)>,
}

impl EngineProcess {
    pub fn spawn(config: &ResolvedEngine) -> Result<Self, String> {
        let command = config.launch.command();
        let mut cmd = Command::new(&command[0]);
        if command.len() > 1 {
            cmd.args(&command[1..]);
        }
        for (key, value) in config.launch.env() {
            if key.ends_with("_TRACE_LOG") || key.ends_with("_DEBUG_LOG") {
                let p = Path::new(value);
                if let Some(parent) = p.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(p, "");
            }
            cmd.env(key, value);
        }
        info!(engine = %config.name, command = ?command, "spawn engine");
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn '{}': {e}", config.name))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("failed to open stdin for '{}'", config.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("failed to open stdout for '{}'", config.name))?;
        if let Some(stderr) = child.stderr.take() {
            let engine = config.name.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(line) if !line.trim().is_empty() => {
                            debug!(engine = %engine, stderr = %line, "engine stderr");
                        }
                        Ok(_) => {}
                        Err(err) => {
                            debug!(engine = %engine, error = %err, "stderr read failed");
                            break;
                        }
                    }
                }
            });
        }
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            options: config
                .launch
                .options()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        })
    }

    pub fn init_ubgi(&mut self) -> Result<(), String> {
        info!("ubgi handshake start");
        self.send(ubgi::CMD_UBGI)?;
        self.read_until(|l| l == "ubgiok" || l == "readyok")?;
        for (name, value) in self.options.clone() {
            self.try_set_option(&name, &value)?;
        }
        self.send(ubgi::CMD_ISREADY)?;
        self.wait_readyok().map_err(runtime_response_error)?;
        Ok(())
    }

    fn try_set_option(&mut self, name: &str, value: &str) -> Result<(), String> {
        self.send(&format!("set {name} {value}"))?;
        self.send(ubgi::CMD_ISREADY)?;
        match self.wait_readyok() {
            Ok(()) => Ok(()),
            Err(ResponseError::Engine(line)) => {
                debug!(option = %name, value = %value, response = %line, "engine rejected optional set");
                Ok(())
            }
            Err(err) => Err(runtime_response_error(err)),
        }
    }

    pub fn new_game(&mut self) -> Result<(), String> {
        self.send(ubgi::CMD_NEWGAME)?;
        self.send(ubgi::CMD_ISREADY)?;
        self.wait_readyok().map_err(runtime_response_error)
    }

    pub fn set_variant(&mut self, variant: Variant) -> Result<(), String> {
        if variant == Variant::Backgammon {
            return Ok(());
        }
        info!(variant = %variant_name(variant), "set engine variant");
        self.send(&format!("set game.variant {}", variant_name(variant)))?;
        self.send(ubgi::CMD_ISREADY)?;
        self.wait_readyok().map_err(|err| match err {
            ResponseError::Engine(line) => format!("engine rejected variant option: {line}"),
            other => runtime_response_error(other),
        })
    }

    pub fn choose_move(&mut self, position_id: &str, dice: Dice) -> Result<String, String> {
        self.send_position_and_dice(position_id, dice)?;
        self.send(ubgi::CMD_GO_CHEQUER)?;
        let mv = self.wait_bestmove().map_err(runtime_response_error)?;
        info!(choice = %mv, "engine chose move");
        Ok(mv)
    }

    pub(crate) fn send_position_and_dice(
        &mut self,
        position_id: &str,
        dice: Dice,
    ) -> Result<(), String> {
        self.send(&ubgi::cmd_position_gnubgid(position_id))?;
        self.send(&ubgi::cmd_dice(dice))
    }

    pub(crate) fn wait_readyok(&mut self) -> Result<(), ResponseError> {
        loop {
            let line = self.read_line().map_err(ResponseError::Transport)?;
            match ubgi::parse_response(&line) {
                ubgi::Response::ReadyOk => return Ok(()),
                ubgi::Response::Error(line) => {
                    return Err(ResponseError::Engine(line.to_string()));
                }
                _ => {}
            }
        }
    }

    pub(crate) fn wait_bestmove(&mut self) -> Result<String, ResponseError> {
        loop {
            let line = self.read_line().map_err(ResponseError::Transport)?;
            match ubgi::parse_response(&line) {
                ubgi::Response::BestMove(mv) => return Ok(mv.to_string()),
                ubgi::Response::MalformedBest(line) => {
                    error!(response = %line, "protocol error: expected bestmove payload");
                    return Err(ResponseError::MalformedBest(line.to_string()));
                }
                ubgi::Response::Error(line) => {
                    error!(response = %line, "engine protocol error");
                    return Err(ResponseError::Engine(line.to_string()));
                }
                _ => {}
            }
        }
    }

    pub fn quit(&mut self) {
        let _ = self.send(ubgi::CMD_QUIT);
        self.reap_child();
    }

    pub fn send_command(&mut self, command: &str) -> Result<(), String> {
        self.send(command)
    }

    pub fn read_response(&mut self) -> Result<String, String> {
        self.read_line()
    }

    fn send(&mut self, command: &str) -> Result<(), String> {
        info!(command = %command, "-> engine");
        writeln!(self.stdin, "{command}").map_err(|e| format!("send failed: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("flush failed: {e}"))?;
        Ok(())
    }

    fn read_line(&mut self) -> Result<String, String> {
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| format!("read failed: {e}"))?;
            if n == 0 {
                return Err("engine closed stdout".to_string());
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            info!(response = %line, "<- engine");
            return Ok(line.to_string());
        }
    }

    fn read_until(&mut self, predicate: impl Fn(&str) -> bool) -> Result<String, String> {
        loop {
            let line = self.read_line()?;
            if line.starts_with("error ") {
                error!(response = %line, "engine protocol error");
                return Err(format!("engine error: {line}"));
            }
            if predicate(&line) {
                return Ok(line);
            }
        }
    }

    fn reap_child(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
            Err(_) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }
}

fn runtime_response_error(err: ResponseError) -> String {
    match err {
        ResponseError::Engine(line) => format!("engine error: {line}"),
        ResponseError::MalformedBest(line) => {
            format!("engine returned unexpected best* response: {line}")
        }
        ResponseError::Transport(err) => err,
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        self.reap_child();
    }
}
