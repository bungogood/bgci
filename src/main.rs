use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

use bkgm::dice::Dice;
use bkgm::dice_gen::{DiceGen, FastrandDice};
use bkgm::{Game, GameState, Variant, VariantPosition};
use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde::Deserialize;

const DUEL_CONFIG_PATH: &str = "config/duel.toml";

#[derive(Debug, Parser)]
#[command(name = "bgci", about = "UBGI dueller")]
struct CliArgs {
    #[arg(long)]
    games: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct DuelConfig {
    games: usize,
    seed: u64,
    max_plies: usize,
    swap_sides: bool,
    variant: String,
    output_csv: String,
    engine_a: EngineConfig,
    engine_b: EngineConfig,
}

impl Default for DuelConfig {
    fn default() -> Self {
        Self {
            games: 20,
            seed: 42,
            max_plies: 512,
            swap_sides: true,
            variant: "backgammon".to_string(),
            output_csv: "artifacts/duels/latest/results.csv".to_string(),
            engine_a: EngineConfig::default_a(),
            engine_b: EngineConfig::default_b(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct EngineConfig {
    name: String,
    command: Vec<String>,
}

impl EngineConfig {
    fn default_a() -> Self {
        Self {
            name: "random-a".to_string(),
            command: vec![
                "cargo".to_string(),
                "run".to_string(),
                "--quiet".to_string(),
                "--bin".to_string(),
                "random_engine".to_string(),
            ],
        }
    }

    fn default_b() -> Self {
        Self {
            name: "random-b".to_string(),
            command: vec![
                "cargo".to_string(),
                "run".to_string(),
                "--quiet".to_string(),
                "--bin".to_string(),
                "random_engine".to_string(),
            ],
        }
    }
}

struct EngineProcess {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

struct DuelGameResult {
    winner_x: Option<bool>,
    points_x: f32,
    points_o: f32,
    plies: usize,
    a_decisions: usize,
    b_decisions: usize,
    a_decision_sec: f64,
    b_decision_sec: f64,
}

impl EngineProcess {
    fn spawn(config: &EngineConfig) -> Result<Self, String> {
        if config.command.is_empty() {
            return Err(format!("engine '{}' has empty command", config.name));
        }
        let mut cmd = Command::new(&config.command[0]);
        if config.command.len() > 1 {
            cmd.args(&config.command[1..]);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
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
        Ok(Self {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn send(&mut self, command: &str) -> Result<(), String> {
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
            return Ok(line.to_string());
        }
    }

    fn read_until(&mut self, predicate: impl Fn(&str) -> bool) -> Result<String, String> {
        loop {
            let line = self.read_line()?;
            if line.starts_with("error ") {
                return Err(format!("engine error: {line}"));
            }
            if predicate(&line) {
                return Ok(line);
            }
        }
    }

    fn init_ubgi(&mut self) -> Result<(), String> {
        self.send("ubgi")?;
        self.read_until(|l| l == "ubgiok" || l == "readyok")?;
        self.send("isready")?;
        self.read_until(|l| l == "readyok")?;
        Ok(())
    }

    fn new_game(&mut self) -> Result<(), String> {
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

    fn choose_move_id(&mut self, position_id: &str, dice: Dice) -> Result<String, String> {
        let (d1, d2) = match dice {
            Dice::Double(d) => (d, d),
            Dice::Mixed(m) => (m.big(), m.small()),
        };
        self.send(&format!("position gnubgid {position_id}"))?;
        self.send(&format!("dice {d1} {d2}"))?;
        self.send("go role chequer")?;
        loop {
            let line = self.read_line()?;
            if let Some(id) = line.strip_prefix("bestmoveid ") {
                return Ok(id.trim().to_string());
            }
            if let Some(payload) = line.strip_prefix("bestmove ") {
                return Ok(payload.trim().to_string());
            }
            if line.starts_with("error ") {
                return Err(format!("engine error: {line}"));
            }
        }
    }

    fn quit(&mut self) {
        let _ = self.send("quit");
    }
}

fn main() -> Result<(), String> {
    let args = CliArgs::parse();
    let mut cfg: DuelConfig = load_toml(DUEL_CONFIG_PATH)?;
    if let Some(games) = args.games {
        cfg.games = games;
    }
    let variant = parse_variant(&cfg.variant)?;

    let output_path = Path::new(&cfg.output_csv);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = fs::File::create(output_path).map_err(|e| e.to_string())?;
    let mut csv = BufWriter::new(file);
    writeln!(
        csv,
        "game,engine_x,engine_o,winner,outcome,points_x,points_o,points_a,points_b,plies"
    )
    .map_err(|e| e.to_string())?;
    csv.flush().map_err(|e| e.to_string())?;

    let mut engine_a = EngineProcess::spawn(&cfg.engine_a)?;
    let mut engine_b = EngineProcess::spawn(&cfg.engine_b)?;
    engine_a.init_ubgi()?;
    engine_b.init_ubgi()?;

    let mut dice_gen = FastrandDice::with_seed(cfg.seed);
    let mut a_points = 0f32;
    let mut b_points = 0f32;
    let mut draws = 0usize;
    let mut total_plies = 0usize;
    let mut a_decisions = 0usize;
    let mut b_decisions = 0usize;
    let mut a_decision_sec = 0f64;
    let mut b_decision_sec = 0f64;
    let mut a_wins = 0usize;
    let mut b_wins = 0usize;
    let mut a_gammons = 0usize;
    let mut b_gammons = 0usize;
    let mut a_backgammons = 0usize;
    let mut b_backgammons = 0usize;
    let mut a_points_as_x = 0f32;
    let mut a_points_as_o = 0f32;
    let mut b_points_as_x = 0f32;
    let mut b_points_as_o = 0f32;
    let run_start = Instant::now();
    let mp = MultiProgress::new();
    let progress = mp.add(ProgressBar::new(cfg.games as u64));
    progress.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold} {wide_bar:.green/black} {pos}/{len} ({percent}%) eta {eta_precise}",
        )
        .map_err(|e| e.to_string())?
        .progress_chars("█▉░"),
    );
    progress.set_prefix("duel");

    let stats_engines = mp.add(ProgressBar::new_spinner());
    stats_engines.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
    let stats_result = mp.add(ProgressBar::new_spinner());
    stats_result.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
    let stats_rate = mp.add(ProgressBar::new_spinner());
    stats_rate.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
    let stats_decide = mp.add(ProgressBar::new_spinner());
    stats_decide.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
    let stats_class = mp.add(ProgressBar::new_spinner());
    stats_class.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);
    let stats_sides = mp.add(ProgressBar::new_spinner());
    stats_sides.set_style(ProgressStyle::with_template("{msg}").map_err(|e| e.to_string())?);

    for game_idx in 0..cfg.games {
        let swap = cfg.swap_sides && game_idx % 2 == 1;
        let a_is_x = !swap;
        engine_a.new_game()?;
        engine_b.new_game()?;

        let result = play_game(
            variant,
            cfg.max_plies,
            &mut dice_gen,
            &mut engine_a,
            &mut engine_b,
            a_is_x,
        )?;

        let winner_x = result.winner_x;
        let points_x = result.points_x;
        let points_o = result.points_o;
        let plies = result.plies;

        let winner_name = match winner_x {
            Some(true) => {
                if a_is_x {
                    &cfg.engine_a.name
                } else {
                    &cfg.engine_b.name
                }
            }
            Some(false) => {
                if a_is_x {
                    &cfg.engine_b.name
                } else {
                    &cfg.engine_a.name
                }
            }
            None => "draw",
        };

        if a_is_x {
            a_points += points_x;
            b_points += points_o;
            a_points_as_x += points_x;
            b_points_as_o += points_o;
        } else {
            a_points += points_o;
            b_points += points_x;
            a_points_as_o += points_o;
            b_points_as_x += points_x;
        }
        if winner_x.is_none() {
            draws += 1;
        }

        let a_game_points = if a_is_x { points_x } else { points_o };
        let b_game_points = if a_is_x { points_o } else { points_x };
        if a_game_points > 0.0 {
            a_wins += 1;
            match a_game_points.abs().round() as i32 {
                2 => a_gammons += 1,
                3 => a_backgammons += 1,
                _ => {}
            }
        } else if b_game_points > 0.0 {
            b_wins += 1;
            match b_game_points.abs().round() as i32 {
                2 => b_gammons += 1,
                3 => b_backgammons += 1,
                _ => {}
            }
        }

        total_plies += plies;
        a_decisions += result.a_decisions;
        b_decisions += result.b_decisions;
        a_decision_sec += result.a_decision_sec;
        b_decision_sec += result.b_decision_sec;

        let engine_x = if a_is_x {
            &cfg.engine_a.name
        } else {
            &cfg.engine_b.name
        };
        let engine_o = if a_is_x {
            &cfg.engine_b.name
        } else {
            &cfg.engine_a.name
        };

        let outcome = match points_x.abs().round() as i32 {
            3 => "backgammon",
            2 => "gammon",
            1 => "normal",
            _ => "unknown",
        };

        writeln!(
            csv,
            "{},{},{},{},{},{:.1},{:.1},{:.1},{:.1},{}",
            game_idx + 1,
            engine_x,
            engine_o,
            winner_name,
            outcome,
            points_x,
            points_o,
            a_game_points,
            b_game_points,
            plies
        )
        .map_err(|e| e.to_string())?;
        csv.flush().map_err(|e| e.to_string())?;

        let done = game_idx + 1;
        let elapsed = run_start.elapsed().as_secs_f64();
        let games_per_sec = done as f64 / elapsed.max(1e-9);
        let eta_sec = (cfg.games - done) as f64 / games_per_sec.max(1e-9);
        let avg_ply = total_plies as f64 / done as f64;
        let a_avg_ms = if a_decisions == 0 {
            0.0
        } else {
            (a_decision_sec * 1000.0) / a_decisions as f64
        };
        let b_avg_ms = if b_decisions == 0 {
            0.0
        } else {
            (b_decision_sec * 1000.0) / b_decisions as f64
        };
        let a_avg_pts = a_points as f64 / done as f64;
        let b_avg_pts = b_points as f64 / done as f64;
        let a_win_pct = (a_wins as f64 / done as f64) * 100.0;
        let b_win_pct = (b_wins as f64 / done as f64) * 100.0;

        let (line_engines, line_result, line_rate, line_decide, line_class, line_sides) =
            render_status_lines(StatusView {
                engine_a: &cfg.engine_a.name,
                engine_b: &cfg.engine_b.name,
                games_done: done,
                a_points,
                b_points,
                a_avg_pts,
                b_avg_pts,
                a_win_pct,
                b_win_pct,
                a_gammons,
                b_gammons,
                a_backgammons,
                b_backgammons,
                draws,
                a_points_as_x,
                a_points_as_o,
                b_points_as_x,
                b_points_as_o,
                a_avg_ms,
                b_avg_ms,
                games_per_sec,
                avg_ply,
                elapsed,
                eta_sec,
            });
        progress.set_position(done as u64);
        stats_engines.set_message(line_engines);
        stats_result.set_message(line_result);
        stats_rate.set_message(line_rate);
        stats_decide.set_message(line_decide);
        stats_class.set_message(line_class);
        stats_sides.set_message(line_sides);
    }

    progress.finish_and_clear();
    stats_engines.finish_and_clear();
    stats_result.finish_and_clear();
    stats_rate.finish_and_clear();
    stats_decide.finish_and_clear();
    stats_class.finish_and_clear();
    stats_sides.finish_and_clear();

    engine_a.quit();
    engine_b.quit();

    let elapsed = run_start.elapsed().as_secs_f64();
    let a_avg_pts = a_points as f64 / cfg.games.max(1) as f64;
    let b_avg_pts = b_points as f64 / cfg.games.max(1) as f64;
    let a_win_pct = (a_wins as f64 / cfg.games.max(1) as f64) * 100.0;
    let b_win_pct = (b_wins as f64 / cfg.games.max(1) as f64) * 100.0;
    let avg_ply = total_plies as f64 / cfg.games.max(1) as f64;
    let a_avg_ms = if a_decisions == 0 {
        0.0
    } else {
        (a_decision_sec * 1000.0) / a_decisions as f64
    };
    let b_avg_ms = if b_decisions == 0 {
        0.0
    } else {
        (b_decision_sec * 1000.0) / b_decisions as f64
    };
    let games_per_sec = cfg.games as f64 / elapsed.max(1e-9);

    let (line_engines, line_result, line_rate, line_decide, line_class, line_sides) =
        render_status_lines(StatusView {
            engine_a: &cfg.engine_a.name,
            engine_b: &cfg.engine_b.name,
            games_done: cfg.games,
            a_points,
            b_points,
            a_avg_pts,
            b_avg_pts,
            a_win_pct,
            b_win_pct,
            a_gammons,
            b_gammons,
            a_backgammons,
            b_backgammons,
            draws,
            a_points_as_x,
            a_points_as_o,
            b_points_as_x,
            b_points_as_o,
            a_avg_ms,
            b_avg_ms,
            games_per_sec,
            avg_ply,
            elapsed,
            eta_sec: 0.0,
        });
    println!("{line_engines}");
    println!("{line_result}");
    println!("{line_rate}");
    println!("{line_decide}");
    println!("{line_class}");
    println!("{line_sides}");
    println!("saved -> {}", cfg.output_csv);
    Ok(())
}

struct StatusView<'a> {
    engine_a: &'a str,
    engine_b: &'a str,
    games_done: usize,
    a_points: f32,
    b_points: f32,
    a_avg_pts: f64,
    b_avg_pts: f64,
    a_win_pct: f64,
    b_win_pct: f64,
    a_gammons: usize,
    b_gammons: usize,
    a_backgammons: usize,
    b_backgammons: usize,
    draws: usize,
    a_points_as_x: f32,
    a_points_as_o: f32,
    b_points_as_x: f32,
    b_points_as_o: f32,
    a_avg_ms: f64,
    b_avg_ms: f64,
    games_per_sec: f64,
    avg_ply: f64,
    elapsed: f64,
    eta_sec: f64,
}

fn render_status_lines(s: StatusView<'_>) -> (String, String, String, String, String, String) {
    let line_engines = format!("ENGINES A={}  B={}", s.engine_a, s.engine_b);
    let line_result = format!(
        "RESULT A {:+.3} pts/g   B {:+.3} pts/g   (score {:+.1}/{:+.1} over {} games, win {:.1}/{:.1}%)",
        s.a_avg_pts,
        s.b_avg_pts,
        s.a_points,
        s.b_points,
        s.games_done,
        s.a_win_pct,
        s.b_win_pct,
    );
    let line_rate = format!(
        "RATE   {:.2} g/s   avg ply {:.1}   elapsed {:.2}s   eta {:.1}s",
        s.games_per_sec, s.avg_ply, s.elapsed, s.eta_sec,
    );
    let line_decide = format!(
        "DECIDE A {:.2} ms/move   B {:.2} ms/move",
        s.a_avg_ms, s.b_avg_ms,
    );
    let line_class = format!(
        "CLASS  gammons {}-{}   backgammons {}-{}   draws {}",
        s.a_gammons, s.b_gammons, s.a_backgammons, s.b_backgammons, s.draws,
    );
    let line_sides = format!(
        "SIDES  A X:{:+.1} O:{:+.1}   B X:{:+.1} O:{:+.1}",
        s.a_points_as_x, s.a_points_as_o, s.b_points_as_x, s.b_points_as_o,
    );
    (
        line_engines,
        line_result,
        line_rate,
        line_decide,
        line_class,
        line_sides,
    )
}

fn play_game(
    variant: Variant,
    max_plies: usize,
    dice_gen: &mut FastrandDice,
    engine_a: &mut EngineProcess,
    engine_b: &mut EngineProcess,
    a_is_x: bool,
) -> Result<DuelGameResult, String> {
    let mut game = Game::new(variant);
    let mut a_decisions = 0usize;
    let mut b_decisions = 0usize;
    let mut a_decision_sec = 0f64;
    let mut b_decision_sec = 0f64;

    for ply in 0..max_plies {
        let dice = if ply == 0 {
            dice_gen.roll_mixed()
        } else {
            dice_gen.roll()
        };
        let legal = game.legal_positions(&dice);
        if legal.is_empty() {
            return Ok(DuelGameResult {
                winner_x: None,
                points_x: 0.0,
                points_o: 0.0,
                plies: ply,
                a_decisions,
                b_decisions,
                a_decision_sec,
                b_decision_sec,
            });
        }
        let legal_ids: Vec<String> = legal.iter().map(|p| p.position_id()).collect();
        let position_id = game.position().position_id();
        let x_to_move = game.position().turn();
        let a_to_move = x_to_move == a_is_x;

        let decision_start = Instant::now();
        let chosen_id = if a_to_move {
            let picked = engine_a.choose_move_id(&position_id, dice)?;
            a_decisions += 1;
            a_decision_sec += decision_start.elapsed().as_secs_f64();
            picked
        } else {
            let picked = engine_b.choose_move_id(&position_id, dice)?;
            b_decisions += 1;
            b_decision_sec += decision_start.elapsed().as_secs_f64();
            picked
        };

        let next = choose_legal_from_id(&legal, &legal_ids, &chosen_id)
            .unwrap_or_else(|| fallback_position(&legal));

        game.set_position(next)
            .map_err(|e| format!("failed to set position: {e}"))?;

        if let GameState::GameOver(result) = next.game_state() {
            let magnitude = result.value().abs();
            let winner_is_x = x_to_move;
            let (points_x, points_o) = if winner_is_x {
                (magnitude, -magnitude)
            } else {
                (-magnitude, magnitude)
            };
            return Ok(DuelGameResult {
                winner_x: Some(winner_is_x),
                points_x,
                points_o,
                plies: ply + 1,
                a_decisions,
                b_decisions,
                a_decision_sec,
                b_decision_sec,
            });
        }
    }

    Ok(DuelGameResult {
        winner_x: None,
        points_x: 0.0,
        points_o: 0.0,
        plies: max_plies,
        a_decisions,
        b_decisions,
        a_decision_sec,
        b_decision_sec,
    })
}

fn choose_legal_from_id(
    legal: &[VariantPosition],
    legal_ids: &[String],
    id: &str,
) -> Option<VariantPosition> {
    legal_ids
        .iter()
        .position(|candidate| candidate == id)
        .map(|idx| legal[idx])
}

fn fallback_position(legal: &[VariantPosition]) -> VariantPosition {
    legal[0]
}

fn parse_variant(name: &str) -> Result<Variant, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "backgammon" => Ok(Variant::Backgammon),
        "nackgammon" => Ok(Variant::Nackgammon),
        "longgammon" => Ok(Variant::Longgammon),
        "hypergammon" | "hypergammon3" => Ok(Variant::Hypergammon),
        "hypergammon2" => Ok(Variant::Hypergammon2),
        "hypergammon4" => Ok(Variant::Hypergammon4),
        "hypergammon5" => Ok(Variant::Hypergammon5),
        _ => Err(format!("unknown variant: {name}")),
    }
}

fn load_toml<T: for<'de> Deserialize<'de>>(path: impl AsRef<Path>) -> Result<T, String> {
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    toml::from_str(&content).map_err(|e| e.to_string())
}
