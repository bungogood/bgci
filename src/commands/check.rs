use bgci::common::parse_variant;
use clap::Args;

use crate::checker::run_check;
use crate::config::{load_toml, resolve_engine_reference, resolve_engine_shortcuts, DuelConfig};

#[derive(Debug, Args)]
pub struct CheckArgs {
    #[arg(long)]
    config: Option<String>,

    #[arg(long)]
    engine: String,

    #[arg(long)]
    variant: Option<String>,
}

pub fn run(args: CheckArgs) -> Result<(), String> {
    let (engine_cfg, default_variant) = if let Some(config_path) = args.config {
        let mut cfg: DuelConfig = load_toml(&config_path)?;
        resolve_engine_shortcuts(&mut cfg)?;
        let selected = match args.engine.to_ascii_lowercase().as_str() {
            "a" => cfg.engine_a,
            "b" => cfg.engine_b,
            _ => resolve_engine_reference(&args.engine)?,
        };
        (selected, cfg.variant)
    } else {
        (
            resolve_engine_reference(&args.engine)?,
            "backgammon".to_string(),
        )
    };
    let variant_name = args.variant.unwrap_or(default_variant);
    let variant = parse_variant(&variant_name)?;

    let report = run_check(&engine_cfg, variant)?;

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
