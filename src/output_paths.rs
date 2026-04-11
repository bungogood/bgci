use std::path::{Path, PathBuf};

use time::OffsetDateTime;

pub struct RunPaths {
    pub timestamp: String,
    pub output_csv: PathBuf,
    pub log_file: PathBuf,
    pub trace_games_dir: PathBuf,
}

pub fn build_run_paths(engine_a: &str, engine_b: &str) -> RunPaths {
    let timestamp = run_timestamp();
    let matchup = format!("{}-vs-{}", slug(engine_a), slug(engine_b));
    let root = Path::new("data").join(matchup);
    let output_csv = root.join(format!("results-{timestamp}.csv"));
    let log_file = output_csv
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("duel-{timestamp}.log"));
    let trace_games_dir = output_csv
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("games-{timestamp}"));
    RunPaths {
        timestamp,
        output_csv,
        log_file,
        trace_games_dir,
    }
}

fn run_timestamp() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "engine".to_string()
    } else {
        out
    }
}
