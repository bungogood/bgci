use std::path::{Path, PathBuf};

use time::OffsetDateTime;

pub struct RunPaths {
    pub timestamp: String,
    pub output_csv: PathBuf,
    pub log_file: PathBuf,
}

pub fn build_run_paths(base_output_csv: &Path) -> RunPaths {
    let timestamp = run_timestamp();
    let output_csv = timestamped_file_path(base_output_csv, &timestamp);
    let log_file = output_csv
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("duel-{timestamp}.log"));
    RunPaths {
        timestamp,
        output_csv,
        log_file,
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

fn timestamped_file_path(base: &Path, stamp: &str) -> PathBuf {
    let parent = base.parent().unwrap_or_else(|| Path::new("."));
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("results");
    let ext = base.extension().and_then(|e| e.to_str());
    let file_name = match ext {
        Some(ext) if !ext.is_empty() => format!("{stem}-{stamp}.{ext}"),
        _ => format!("{stem}-{stamp}"),
    };
    parent.join(file_name)
}
