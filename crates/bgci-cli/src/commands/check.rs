use bgci_core::checker::run_check;
use bgci_core::common::{Variant, parse_variant};
use bgci_core::config::{
    MatchupConfig, ResolvedEngine, load_toml, resolve_engine_input, resolve_engine_spec,
};
use clap::Args;

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
        let cfg: MatchupConfig = load_toml(&config_path)?;
        let variant = parse_variant(&cfg.variant)?;
        let selected = match args.engine.as_deref() {
            Some(engine) if engine.eq_ignore_ascii_case("a") => {
                vec![(resolve_engine_input(cfg.engine_a)?, variant)]
            }
            Some(engine) if engine.eq_ignore_ascii_case("b") => {
                vec![(resolve_engine_input(cfg.engine_b)?, variant)]
            }
            Some(engine) => vec![(resolve_engine_spec(engine)?, variant)],
            None => vec![
                (resolve_engine_input(cfg.engine_a)?, variant),
                (resolve_engine_input(cfg.engine_b)?, variant),
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
    let engine_cfg = resolve_engine_spec(engine)?;
    run_single(engine_cfg, parse_variant("backgammon")?, args.variant)
}

fn run_single(
    engine_cfg: ResolvedEngine,
    default_variant: Variant,
    variant_override: Option<String>,
) -> Result<(), String> {
    let variant = match variant_override {
        Some(variant) => parse_variant(&variant)?,
        None => default_variant,
    };

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
        println!("keys:");
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
    println!(
        "awkward_legal_probes_passed: {}",
        report.awkward_legal_probes_passed
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

    if report.is_pass() {
        Ok(())
    } else {
        Err("engine check failed".to_string())
    }
}
