use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;

use bgci::common::variant_name;
use bkgm::dice::Dice;
use bkgm::Variant;
use tracing::{debug, error, info};

use crate::config::EngineConfig;

pub struct EngineProcess {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    ubgi_log: Option<std::io::BufWriter<fs::File>>,
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
            _child: child,
            stdin,
            stdout: BufReader::new(stdout),
            ubgi_log: None,
        })
    }

    pub fn set_ubgi_log_path(&mut self, path: Option<&Path>) -> Result<(), String> {
        if let Some(mut writer) = self.ubgi_log.take() {
            let _ = writer.flush();
        }
        let Some(path) = path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create ubgi log dir failed: {e}"))?;
        }
        let file = fs::File::create(path)
            .map_err(|e| format!("open ubgi log '{}' failed: {e}", path.display()))?;
        self.ubgi_log = Some(std::io::BufWriter::new(file));
        Ok(())
    }

    pub fn init_ubgi(&mut self) -> Result<(), String> {
        info!("ubgi handshake start");
        self.send("ubgi")?;
        self.read_until(|l| l == "ubgiok" || l == "readyok")?;
        self.send("isready")?;
        self.read_until(|l| l == "readyok")?;
        Ok(())
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
        x_to_move: bool,
    ) -> Result<String, String> {
        let (d1, d2) = match dice {
            Dice::Double(d) => (d, d),
            Dice::Mixed(m) => (m.big(), m.small()),
        };
        self.send(&format!("position gnubgid {position_id}"))?;
        self.send(&format!("setturn {}", if x_to_move { "p0" } else { "p1" }))?;
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
        if let Some(writer) = self.ubgi_log.as_mut() {
            let _ = writer.flush();
        }
    }

    pub fn send_command(&mut self, command: &str) -> Result<(), String> {
        self.send(command)
    }

    pub fn read_response(&mut self) -> Result<String, String> {
        self.read_line()
    }

    fn send(&mut self, command: &str) -> Result<(), String> {
        info!(command = %command, "-> engine");
        self.write_ubgi_log("->", command);
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
            self.write_ubgi_log("<-", line);
            return Ok(line.to_string());
        }
    }

    fn write_ubgi_log(&mut self, direction: &str, line: &str) {
        let Some(writer) = self.ubgi_log.as_mut() else {
            return;
        };
        let _ = writeln!(writer, "{direction} {line}");
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
}
