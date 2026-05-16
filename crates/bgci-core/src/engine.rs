use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;

use crate::common::variant_name;
use bkgm::dice::Dice;
use bkgm::Variant;
use tracing::{debug, error, info};

use crate::config::EngineConfig;

pub struct EngineProcess {
    name: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    options: Vec<(String, String)>,
}

impl EngineProcess {
    pub fn spawn(config: &EngineConfig) -> Result<Self, String> {
        if config.command.is_empty() {
            return Err(format!("engine '{}' has empty command", config.name));
        }
        let mut cmd = Command::new(&config.command[0]);
        if config.command.len() > 1 {
            cmd.args(&config.command[1..]);
        }
        for (key, value) in &config.env {
            if key.ends_with("_TRACE_LOG") || key.ends_with("_DEBUG_LOG") {
                let p = Path::new(value);
                if let Some(parent) = p.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(p, "");
            }
            cmd.env(key, value);
        }
        info!(engine = %config.name, command = ?config.command, "spawn engine");
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
            name: config.name.clone(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            options: config
                .options
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        })
    }

    pub fn init_ubgi(&mut self) -> Result<(), String> {
        info!("ubgi handshake start");
        self.send("ubgi")?;
        self.read_until(|l| l == "ubgiok" || l == "readyok")?;
        for (name, value) in self.options.clone() {
            self.try_set_option(&name, &value)?;
        }
        self.send("isready")?;
        self.read_until(|l| l == "readyok")?;
        Ok(())
    }

    fn try_set_option(&mut self, name: &str, value: &str) -> Result<(), String> {
        self.send(&format!("setoption name {name} value {value}"))?;
        self.send("isready")?;
        loop {
            let line = self.read_line()?;
            if line == "readyok" {
                return Ok(());
            }
            if line.starts_with("error ") {
                eprintln!(
                    "warning: engine '{}' rejected setoption {}={}; continuing",
                    self.name, name, value
                );
                debug!(option = %name, value = %value, response = %line, "engine rejected optional setoption");
                return Ok(());
            }
        }
    }

    pub fn new_game(&mut self) -> Result<(), String> {
        self.send("newgame")?;
        self.send("isready")?;
        loop {
            let line = self.read_line()?;
            if line == "readyok" {
                break;
            }
            if line.starts_with("error unknown_command") {
                continue;
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
        self.send(&format!(
            "setoption name Variant value {}",
            variant_name(variant)
        ))?;
        self.send("isready")?;
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
        let (d1, d2) = match dice {
            Dice::Double(d) => (d, d),
            Dice::Mixed(m) => (m.big(), m.small()),
        };
        self.send(&format!("position gnubgid {position_id}"))?;
        self.send(&format!("dice {d1} {d2}"))?;
        self.send("go role chequer")?;
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
                if line.starts_with("error unknown_command") {
                    continue;
                }
                error!(response = %line, "engine protocol error");
                return Err(format!("engine error: {line}"));
            }
        }
    }

    pub fn quit(&mut self) {
        let _ = self.send("quit");
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
