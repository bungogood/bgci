use bgci_core::common::parse_variant;
use bgci_core::config::{DuelConfig, EngineConfig, load_toml, resolve_engine_shortcuts};
use bgci_core::duel_runner::run_duel;
use bgci_core::output_paths::build_run_paths;
use clap::Args;
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use tracing::info;

use crate::logging;

#[derive(Debug, Args)]
pub struct DuelArgs {
    #[arg(short = 'c', long)]
    config: Option<String>,

    #[arg(short = 'a', long = "engine-a")]
    engine_a: Option<String>,

    #[arg(short = 'b', long = "engine-b")]
    engine_b: Option<String>,

    #[arg(short = 'g', long)]
    games: Option<usize>,

    #[arg(short = 'p', long)]
    parallel: Option<usize>,

    #[arg(short = 's', long)]
    seed: Option<u64>,

    #[arg(short = 'm', long = "max-plies")]
    max_plies: Option<usize>,

    #[arg(short = 't', long = "timeout-secs")]
    timeout_secs: Option<u64>,

    #[arg(short = 'v', long)]
    variant: Option<String>,

    #[arg(short = 'l', long)]
    log: Option<String>,

    #[arg(short = 'w', long = "swap-sides")]
    swap_sides: bool,

    #[arg(short = 'n', long = "no-swap-sides")]
    no_swap_sides: bool,

    #[arg(short = 'S', long)]
    save: bool,

    #[arg(short = 'o', long = "output")]
    output_csv: Option<String>,

    #[arg(long)]
    record: bool,

    #[arg(long = "db")]
    db_path: Option<String>,

    #[arg(long = "ply")]
    ply: Option<usize>,

    #[arg(long = "ply-a")]
    ply_a: Option<usize>,

    #[arg(long = "ply-b")]
    ply_b: Option<usize>,
}

pub async fn run(args: DuelArgs) -> Result<(), String> {
    let mut cfg = build_duel_config(&args)?;
    resolve_engine_shortcuts(&mut cfg)?;

    let mut run_paths = build_run_paths(&cfg.engine_a.name, &cfg.engine_b.name);
    if let Some(path) = &args.output_csv {
        run_paths.output_csv = PathBuf::from(path);
    }
    let save_results = args.save || args.output_csv.is_some() || args.record;
    let _log_guard = logging::init_tracing(&cfg.log, &run_paths.log_file)?;
    let variant = parse_variant(&cfg.variant)?;

    info!(
        run = %run_paths.timestamp,
        log = %cfg.log,
        log_path = %run_paths.log_file.display(),
        save_results,
        output_csv = %run_paths.output_csv.display(),
        games = cfg.games,
        parallel = cfg.parallel,
        seed = cfg.seed,
        max_plies = cfg.max_plies,
        timeout_secs = cfg.timeout_secs,
        variant = %cfg.variant,
        engine_a = %cfg.engine_a.name,
        engine_a_cmd = %cfg.engine_a.command.join(" "),
        engine_b = %cfg.engine_b.name,
        engine_b_cmd = %cfg.engine_b.command.join(" "),
        "duel run header"
    );

    let summary = run_duel(&cfg, variant, &run_paths, save_results).await?;
    println!("{}", summary.line_engines);
    println!("{}", summary.line_result);
    println!("{}", summary.line_rate);
    println!("{}", summary.line_decide);
    println!("{}", summary.line_class);
    println!("{}", summary.line_sides);
    if save_results {
        println!("saved -> {}", run_paths.output_csv.display());
    }
    if args.record {
        let db_path = args.db_path.clone().unwrap_or_else(default_eval_db_path);
        record_duel_in_db(&db_path, &cfg, &run_paths)?;
        println!("db    -> {}", db_path);
    }
    if logging::normalize_level(&cfg.log).is_some() {
        println!("log   -> {}", run_paths.log_file.display());
    }

    info!(save_results, output_csv = %run_paths.output_csv.display(), "duel run complete");
    Ok(())
}

fn build_duel_config(args: &DuelArgs) -> Result<DuelConfig, String> {
    let mut cfg = if let Some(config_path) = &args.config {
        load_toml(config_path)?
    } else {
        if args.engine_a.is_none() && args.engine_b.is_none() {
            return Err(
                "duel requires either --config or both --engine-a and --engine-b".to_string(),
            );
        }

        let engine_a = args
            .engine_a
            .clone()
            .ok_or_else(|| "missing --engine-a (or use --config)".to_string())?;
        let engine_b = args
            .engine_b
            .clone()
            .ok_or_else(|| "missing --engine-b (or use --config)".to_string())?;

        let mut cfg = DuelConfig::default();
        cfg.engine_a = EngineConfig {
            name: engine_a.clone(),
            engine: Some(engine_a),
            command: Vec::new(),
            env: Default::default(),
            options: BTreeMap::new(),
        };
        cfg.engine_b = EngineConfig {
            name: engine_b.clone(),
            engine: Some(engine_b),
            command: Vec::new(),
            env: Default::default(),
            options: BTreeMap::new(),
        };
        cfg
    };

    if let Some(engine_a) = &args.engine_a {
        cfg.engine_a = EngineConfig {
            name: engine_a.clone(),
            engine: Some(engine_a.clone()),
            command: Vec::new(),
            env: Default::default(),
            options: BTreeMap::new(),
        };
    }
    if let Some(engine_b) = &args.engine_b {
        cfg.engine_b = EngineConfig {
            name: engine_b.clone(),
            engine: Some(engine_b.clone()),
            command: Vec::new(),
            env: Default::default(),
            options: BTreeMap::new(),
        };
    }

    if let Some(games) = args.games {
        cfg.games = games;
    }
    if let Some(parallel) = args.parallel {
        cfg.parallel = parallel.max(1);
    }
    if let Some(seed) = args.seed {
        cfg.seed = seed;
    }
    if let Some(max_plies) = args.max_plies {
        cfg.max_plies = max_plies.max(1);
    }
    if let Some(timeout_secs) = args.timeout_secs {
        cfg.timeout_secs = Some(timeout_secs.max(1));
    }
    if let Some(variant) = &args.variant {
        cfg.variant = variant.clone();
    }
    if let Some(log) = &args.log {
        cfg.log = log.clone();
    }
    if args.swap_sides && args.no_swap_sides {
        return Err("cannot pass both --swap-sides and --no-swap-sides".to_string());
    }
    if args.swap_sides {
        cfg.swap_sides = true;
    }
    if args.no_swap_sides {
        cfg.swap_sides = false;
    }

    let ply_a = args.ply_a.or(args.ply);
    let ply_b = args.ply_b.or(args.ply);
    if let Some(ply) = ply_a {
        if ply < 1 {
            return Err("--ply-a/--ply must be >= 1".to_string());
        }
        cfg.engine_a
            .options
            .insert("Ply".to_string(), ply.to_string());
    }
    if let Some(ply) = ply_b {
        if ply < 1 {
            return Err("--ply-b/--ply must be >= 1".to_string());
        }
        cfg.engine_b
            .options
            .insert("Ply".to_string(), ply.to_string());
    }

    Ok(cfg)
}

fn record_duel_in_db(
    db_path: &str,
    cfg: &DuelConfig,
    run_paths: &bgci_core::output_paths::RunPaths,
) -> Result<(), String> {
    let conn = open_eval_db(db_path)?;
    init_eval_schema(&conn)?;

    let engine_a_id = ensure_engine(&conn, &cfg.engine_a.name)?;
    let engine_b_id = ensure_engine(&conn, &cfg.engine_b.name)?;

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin transaction: {e}"))?;

    tx.execute(
        "INSERT OR REPLACE INTO duel_runs(run_id, engine_a_id, engine_b_id, games, parallel, seed, max_plies, variant, output_csv)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            run_paths.timestamp,
            engine_a_id,
            engine_b_id,
            cfg.games as i64,
            cfg.parallel as i64,
            cfg.seed as i64,
            cfg.max_plies as i64,
            cfg.variant,
            run_paths.output_csv.to_string_lossy().to_string(),
        ],
    )
    .map_err(|e| format!("insert duel run: {e}"))?;

    let duel_run_id: i64 = tx
        .query_row(
            "SELECT id FROM duel_runs WHERE run_id = ?",
            params![run_paths.timestamp],
            |row| row.get(0),
        )
        .map_err(|e| format!("load duel run id: {e}"))?;

    let content = fs::read_to_string(&run_paths.output_csv)
        .map_err(|e| format!("read csv {}: {e}", run_paths.output_csv.display()))?;
    for (line_no, line) in content.lines().enumerate() {
        if line_no == 0 || line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 10 {
            continue;
        }

        let game_idx = match cols[0].trim().parse::<i64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let points_x = match cols[5].trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let points_o = match cols[6].trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let points_a = match cols[7].trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let points_b = match cols[8].trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let plies = match cols[9].trim().parse::<i64>() {
            Ok(v) => v,
            Err(_) => continue,
        };

        tx.execute(
            "INSERT OR REPLACE INTO game_results(duel_run_id, game_idx, engine_x, engine_o, winner, outcome, points_x, points_o, points_a, points_b, plies)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                duel_run_id,
                game_idx,
                cols[1].trim(),
                cols[2].trim(),
                cols[3].trim(),
                cols[4].trim(),
                points_x,
                points_o,
                points_a,
                points_b,
                plies,
            ],
        )
        .map_err(|e| format!("insert game result: {e}"))?;
    }

    tx.commit().map_err(|e| format!("commit: {e}"))
}

fn open_eval_db(db_path: &str) -> Result<Connection, String> {
    let db = PathBuf::from(db_path);
    if let Some(parent) = db.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("create db dir {}: {e}", parent.display()))?;
    }
    Connection::open(db).map_err(|e| format!("open db {db_path}: {e}"))
}

fn init_eval_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS engines (
          id INTEGER PRIMARY KEY,
          name TEXT NOT NULL UNIQUE,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS duel_runs (
          id INTEGER PRIMARY KEY,
          run_id TEXT NOT NULL UNIQUE,
          engine_a_id INTEGER NOT NULL,
          engine_b_id INTEGER NOT NULL,
          games INTEGER NOT NULL,
          parallel INTEGER NOT NULL,
          seed INTEGER NOT NULL,
          max_plies INTEGER NOT NULL,
          variant TEXT NOT NULL,
          output_csv TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          FOREIGN KEY(engine_a_id) REFERENCES engines(id),
          FOREIGN KEY(engine_b_id) REFERENCES engines(id)
        );

        CREATE TABLE IF NOT EXISTS game_results (
          id INTEGER PRIMARY KEY,
          duel_run_id INTEGER NOT NULL,
          game_idx INTEGER NOT NULL,
          engine_x TEXT NOT NULL,
          engine_o TEXT NOT NULL,
          winner TEXT NOT NULL,
          outcome TEXT NOT NULL,
          points_x REAL NOT NULL,
          points_o REAL NOT NULL,
          points_a REAL NOT NULL,
          points_b REAL NOT NULL,
          plies INTEGER NOT NULL,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          UNIQUE(duel_run_id, game_idx),
          FOREIGN KEY(duel_run_id) REFERENCES duel_runs(id)
        );
        ",
    )
    .map_err(|e| format!("init schema: {e}"))
}

fn ensure_engine(conn: &Connection, name: &str) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO engines(name) VALUES(?) ON CONFLICT(name) DO NOTHING",
        params![name],
    )
    .map_err(|e| format!("insert engine {name}: {e}"))?;

    conn.query_row(
        "SELECT id FROM engines WHERE name = ?",
        params![name],
        |row| row.get(0),
    )
    .map_err(|e| format!("engine id {name}: {e}"))
}

fn default_eval_db_path() -> String {
    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg_data_home)
            .join("bgci")
            .join("eval.db")
            .to_string_lossy()
            .into_owned();
    }

    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("bgci")
            .join("eval.db")
            .to_string_lossy()
            .into_owned();
    }

    "data/eval.db".to_string()
}
