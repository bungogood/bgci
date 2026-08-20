use bgci_core::config::{list_engine_alias_details, resolve_engine_reference};
use bgci_core::engines;
use clap::Args;
use std::process::{Command, Stdio};

#[derive(Debug, Args)]
pub struct EngineArgs {
    kind: Option<String>,

    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    engine_args: Vec<String>,

    #[arg(short = 'l', long)]
    list: bool,

    #[arg(short = 'v', long)]
    verbose: bool,
}

pub fn run(args: EngineArgs) -> Result<(), String> {
    if args.list {
        if args.kind.is_some() {
            return Err("--list cannot be used with an engine kind".to_string());
        }
        if !args.engine_args.is_empty() {
            return Err("extra engine args are not allowed with --list".to_string());
        }
        if args.verbose {
            for detail in list_engine_alias_details()? {
                println!("{}", detail.name);
                if let Some(family) = &detail.family {
                    println!("  family: {family}");
                }
                if let Some(version) = &detail.version {
                    println!("  version: {version}");
                }
                if !detail.configuration.is_empty() {
                    println!("  configuration:");
                    for (key, value) in &detail.configuration {
                        println!("    {key}={value}");
                    }
                }
                if let Some(url) = &detail.url {
                    println!("  url: {url}");
                }
                println!("  source: {}", detail.source);
                println!("  command: {}", detail.command.join(" "));
                if !detail.env.is_empty() {
                    println!("  env:");
                    for (key, value) in detail.env {
                        println!("    {}={}", key, value);
                    }
                }
                if !detail.options.is_empty() {
                    println!("  options:");
                    for (key, value) in detail.options {
                        println!("    {key}={value}");
                    }
                }
            }
            return Ok(());
        }
        println!("family               engine");
        for detail in list_engine_alias_details()? {
            println!(
                "{:<20} {}",
                detail.family.as_deref().unwrap_or("-"),
                detail.name
            );
        }
        return Ok(());
    }

    if args.verbose {
        return Err("--verbose requires --list".to_string());
    }

    let Some(kind) = args.kind else {
        return Err("missing engine kind (or use --list)".to_string());
    };

    if let Some(builtin) = engines::builtin_engine_name(&kind.to_ascii_lowercase()) {
        return engines::run_by_name_with_args(builtin, &args.engine_args);
    }

    let engine = resolve_engine_reference(&kind)?;
    run_external_engine(&engine.command, &engine.env, &args.engine_args)
}

fn run_external_engine(
    command: &[String],
    env: &std::collections::BTreeMap<String, String>,
    extra_args: &[String],
) -> Result<(), String> {
    if command.is_empty() {
        return Err("engine command cannot be empty".to_string());
    }

    let mut cmd = Command::new(&command[0]);
    if command.len() > 1 {
        cmd.args(&command[1..]);
    }
    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }
    for (key, value) in env {
        cmd.env(key, value);
    }

    let status = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("spawn '{}': {e}", command[0]))?;

    if status.success() {
        return Ok(());
    }
    match status.code() {
        Some(code) => Err(format!("engine exited with status {code}")),
        None => Err("engine terminated by signal".to_string()),
    }
}
