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

use crate::config::{ResolvedEngine, engine_identity_from_spec_with_options, resolve_engine_spec};

pub fn resolve_engine(spec: &str) -> Result<ResolvedEngine, String> {
    let (_, config) = resolve_engine_spec(spec)?;
    let mut config = filter_supported_engine_options(&config);
    config.name = engine_identity_from_spec_with_options(spec, config.launch.options())?;
    Ok(config)
}

pub fn filter_supported_engine_options(cfg: &ResolvedEngine) -> ResolvedEngine {
    let mut filtered = cfg.clone();
    let Ok(supported) = discover_supported_keys(cfg) else {
        return filtered;
    };
    if supported.is_empty() {
        return filtered;
    }
    let unsupported: Vec<String> = filtered
        .launch
        .options()
        .keys()
        .filter(|k| !supported.contains(*k))
        .cloned()
        .collect();
    if !unsupported.is_empty() {
        eprintln!(
            "warning: engine '{}' does not support {}; ignoring",
            cfg.name,
            unsupported.join(", ")
        );
    }
    filtered
        .launch
        .options_mut()
        .retain(|k, _| supported.contains(k));
    filtered
}

fn discover_supported_keys(cfg: &ResolvedEngine) -> Result<HashSet<String>, String> {
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
        self.read_until(|l| l == "readyok")?;
        Ok(())
    }

    fn try_set_option(&mut self, name: &str, value: &str) -> Result<(), String> {
        self.send(&format!("set {name} {value}"))?;
        self.send(ubgi::CMD_ISREADY)?;
        loop {
            let line = self.read_line()?;
            if line == "readyok" {
                return Ok(());
            }
            if line.starts_with("error ") {
                debug!(option = %name, value = %value, response = %line, "engine rejected optional set");
                return Ok(());
            }
        }
    }

    pub fn new_game(&mut self) -> Result<(), String> {
        self.send(ubgi::CMD_NEWGAME)?;
        self.send(ubgi::CMD_ISREADY)?;
        loop {
            let line = self.read_line()?;
            if line == "readyok" {
                break;
            }
            if line.starts_with("error ") {
                return Err(format!("engine error: {line}"));
            }
        }
        Ok(())
    }

    pub fn set_variant(&mut self, variant: Variant) -> Result<(), String> {
        if variant == Variant::Backgammon {
            return Ok(());
        }
        info!(variant = %variant_name(variant), "set engine variant");
        self.send(&format!("set game.variant {}", variant_name(variant)))?;
        self.send(ubgi::CMD_ISREADY)?;
        loop {
            let line = self.read_line()?;
            if line == "readyok" {
                return Ok(());
            }
            if line.starts_with("error ") {
                return Err(format!("engine rejected variant option: {line}"));
            }
        }
    }

    pub fn choose_move(
        &mut self,
        position_id: &str,
        dice: Dice,
        _x_to_move: bool,
    ) -> Result<String, String> {
        self.send(&ubgi::cmd_position_gnubgid(position_id))?;
        self.send(&ubgi::cmd_dice(dice))?;
        self.send(ubgi::CMD_GO_CHEQUER)?;
        loop {
            let line = self.read_line()?;
            if let Some(mv) = line.strip_prefix("bestmove ") {
                info!(choice = %mv.trim(), "engine chose move");
                return Ok(mv.trim().to_string());
            }
            if line.starts_with("best") {
                error!(response = %line, "protocol error: expected bestmove payload");
                return Err(format!("engine returned unexpected best* response: {line}"));
            }
            if line.starts_with("error ") {
                error!(response = %line, "engine protocol error");
                return Err(format!("engine error: {line}"));
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

impl Drop for EngineProcess {
    fn drop(&mut self) {
        self.reap_child();
    }
}
