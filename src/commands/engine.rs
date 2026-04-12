use bgci::engines;
use clap::Args;
use std::process::{Command, Stdio};

use crate::config::{list_engine_alias_details, list_engine_aliases, resolve_engine_reference};

#[derive(Debug, Args)]
pub struct EngineArgs {
    kind: Option<String>,

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
        if args.verbose {
            for detail in list_engine_alias_details()? {
                println!("{}", detail.name);
                println!("  source: {}", detail.source);
                println!("  command: {}", detail.command.join(" "));
                if !detail.env.is_empty() {
                    println!("  env:");
                    for (key, value) in detail.env {
                        println!("    {}={}", key, value);
                    }
                }
            }
            return Ok(());
        }
        for name in list_engine_aliases()? {
            println!("{name}");
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
        return engines::run_by_name(builtin);
    }

    let engine = resolve_engine_reference(&kind)?;
    run_external_engine(&engine.command, &engine.env)
}

fn run_external_engine(
    command: &[String],
    env: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    if command.is_empty() {
        return Err("engine command cannot be empty".to_string());
    }

    let mut cmd = Command::new(&command[0]);
    if command.len() > 1 {
        cmd.args(&command[1..]);
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
