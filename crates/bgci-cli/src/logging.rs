use std::fs;
use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

pub type LogGuard = WorkerGuard;

pub fn init_tracing(level: &str, log_path: &Path) -> Result<Option<LogGuard>, String> {
    let Some(level) = normalize_level(level) else {
        return Ok(None);
    };

    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create log dir: {e}"))?;
    }

    let file_name = log_path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| format!("invalid log file path: {}", log_path.display()))?;
    let dir = log_path.parent().unwrap_or_else(|| Path::new("."));

    let file_appender = tracing_appender::rolling::never(dir, file_name);
    let (writer, guard) = tracing_appender::non_blocking(file_appender);

    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|raw| EnvFilter::try_new(raw).ok())
        .unwrap_or_else(|| EnvFilter::new(format!("bgci={level}")));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .with_target(true)
        .with_level(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    Ok(Some(guard))
}

pub fn normalize_level(level: &str) -> Option<&'static str> {
    match level.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "" => None,
        "error" => Some("error"),
        "warn" | "warning" => Some("warn"),
        "info" => Some("info"),
        "debug" => Some("debug"),
        "trace" => Some("trace"),
        _ => Some("info"),
    }
}
