use bgci::common::parse_variant;
use clap::Args;

use crate::checker::run_check;
use crate::config::{load_toml, resolve_engine_reference, resolve_engine_shortcuts, DuelConfig};

#[derive(Debug, Args)]
pub struct CheckArgs {
    #[arg(long)]
    config: Option<String>,

    engine: Option<String>,

    #[arg(long)]
    variant: Option<String>,
}

pub fn run(args: CheckArgs) -> Result<(), String> {
    if let Some(config_path) = args.config {
        let mut cfg: DuelConfig = load_toml(&config_path)?;
        resolve_engine_shortcuts(&mut cfg)?;
        let selected = match args.engine.as_deref() {
            Some(engine) if engine.eq_ignore_ascii_case("a") => vec![(cfg.engine_a, cfg.variant)],
            Some(engine) if engine.eq_ignore_ascii_case("b") => vec![(cfg.engine_b, cfg.variant)],
            Some(engine) => vec![(resolve_engine_reference(engine)?, cfg.variant)],
            None => vec![
                (cfg.engine_a, cfg.variant.clone()),
                (cfg.engine_b, cfg.variant),
            ],
        };

        for (idx, (engine_cfg, default_variant)) in selected.into_iter().enumerate() {
            if idx > 0 {
                println!();
            }
            run_single(engine_cfg, default_variant, args.variant.clone())?;
        }

        return Ok(());
    }

    let engine = args.engine.as_deref().ok_or_else(|| {
        "missing engine. usage: bgci check <engine> or bgci check --config <path> [a|b]".to_string()
    })?;
    let engine_cfg = resolve_engine_reference(engine)?;
    run_single(engine_cfg, "backgammon".to_string(), args.variant)
}

fn run_single(
    engine_cfg: crate::config::EngineConfig,
    default_variant: String,
    variant_override: Option<String>,
) -> Result<(), String> {
    let variant_name = variant_override.unwrap_or(default_variant);
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
