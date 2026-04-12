use bgci::common::parse_variant;
use checker::run_check;
use clap::{Args, Parser, Subcommand, ValueEnum};
use config::{load_toml, DuelConfig};
use duel_runner::run_duel;
use output_paths::build_run_paths;
use tracing::info;

mod checker;
mod config;
mod duel_runner;
mod engine;
mod logging;
mod output_paths;
mod report;
mod stats;

#[derive(Debug, Parser)]
#[command(name = "bgci", about = "UBGI dueller")]
struct CliArgs {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long)]
    config: Option<String>,

    #[arg(long)]
    games: Option<usize>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Check(CheckArgs),
}

#[derive(Debug, Args)]
struct CheckArgs {
    #[arg(long)]
    config: String,

    #[arg(long, value_enum, default_value_t = CheckEngine::A)]
    engine: CheckEngine,

    #[arg(long)]
    variant: Option<String>,
}

#[derive(Clone, Debug, ValueEnum)]
enum CheckEngine {
    A,
    B,
}

fn main() -> Result<(), String> {
    let args = CliArgs::parse();
    if let Some(Commands::Check(check)) = args.command {
        return run_check_command(check);
    }

    let config_path = args
        .config
        .as_deref()
        .ok_or_else(|| "--config is required for duel mode".to_string())?;

    let mut cfg: DuelConfig = load_toml(config_path)?;
    if let Some(games) = args.games {
        cfg.games = games;
    }

    let run_paths = build_run_paths(&cfg.engine_a.name, &cfg.engine_b.name);

    let _log_guard = logging::init_tracing(&cfg.log, &run_paths.log_file)?;
    let variant = parse_variant(&cfg.variant)?;

    info!(
        run = %run_paths.timestamp,
        config = %config_path,
        log = %cfg.log,
        log_path = %run_paths.log_file.display(),
        output_csv = %run_paths.output_csv.display(),
        games = cfg.games,
        seed = cfg.seed,
        max_plies = cfg.max_plies,
        variant = %cfg.variant,
        engine_a = %cfg.engine_a.name,
        engine_a_cmd = %cfg.engine_a.command.join(" "),
        engine_b = %cfg.engine_b.name,
        engine_b_cmd = %cfg.engine_b.command.join(" "),
        "duel run header"
    );

    let summary = run_duel(&cfg, variant, &run_paths)?;
    println!("{}", summary.line_engines);
    println!("{}", summary.line_result);
    println!("{}", summary.line_rate);
    println!("{}", summary.line_decide);
    println!("{}", summary.line_class);
    println!("{}", summary.line_sides);
    println!("saved -> {}", run_paths.output_csv.display());
    if logging::normalize_level(&cfg.log).is_some() {
        println!("log   -> {}", run_paths.log_file.display());
    }

    info!(output_csv = %run_paths.output_csv.display(), "duel run complete");
    Ok(())
}

fn run_check_command(args: CheckArgs) -> Result<(), String> {
    let cfg: DuelConfig = load_toml(&args.config)?;
    let engine_cfg = match args.engine {
        CheckEngine::A => &cfg.engine_a,
        CheckEngine::B => &cfg.engine_b,
    };
    let variant_name = args.variant.unwrap_or(cfg.variant);
    let variant = parse_variant(&variant_name)?;

    let report = run_check(engine_cfg, variant)?;

    println!("engine: {}", report.engine_name);
    println!("status: {}", if report.is_pass() { "PASS" } else { "FAIL" });
    if !report.ids.is_empty() {
        println!("id lines:");
        for line in &report.ids {
            println!("  {line}");
        }
    }
    if !report.options.is_empty() {
        println!("options:");
        for line in &report.options {
            println!("  {line}");
        }
    }
    println!(
        "capabilities: newgame={} position={} dice={} go_chequer={}",
        report.supports_newgame,
        report.supports_position,
        report.supports_dice,
        report.supports_go_chequer,
    );
    println!(
        "notation: bar={} off={} numeric_alias_seen={}",
        report.bar_notation_ok, report.off_notation_ok, report.numeric_bar_off_alias_seen,
    );
    if let Some(raw) = &report.bestmove_raw {
        println!("bestmove raw: {raw}");
    }
    if let Some(canon) = &report.bestmove_canonical {
        println!("bestmove canonical: {canon}");
    }
    if !report.legal_preview.is_empty() {
        println!("legal preview: {}", report.legal_preview.join(", "));
    }
    if !report.errors.is_empty() {
        println!("errors:");
        for err in &report.errors {
            println!("  {err}");
        }
    }

    Ok(())
}
