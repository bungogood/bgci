use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Args;
use rusqlite::{Connection, params};
use tokio::time::{Duration, sleep};
use tracing::{info, warn};

use crate::common::parse_variant;
use crate::config::{DuelConfig, EngineConfig, resolve_engine_reference, resolve_engine_spec};
use crate::duel_runner::run_duel;
use crate::engine::filter_supported_engine_options;
use crate::output_paths::RunPaths;

#[derive(Debug, Clone, Args)]
pub struct RunRatingsArgs {
    #[arg(long, num_args = 1..)]
    pub engines: Vec<String>,

    #[arg(long, alias = "leaderboard")]
    pub show: bool,

    #[arg(long)]
    pub reset: bool,

    #[arg(long)]
    pub reset_all: bool,

    #[arg(long = "db")]
    pub db_path: Option<String>,

    #[arg(long)]
    pub tui: bool,

    #[arg(long, default_value_t = 2000)]
    pub budget_games: usize,

    #[arg(long, default_value_t = 40)]
    pub pair_games: usize,

    #[arg(long, default_value_t = 0)]
    pub min_pair_games: usize,

    #[arg(long, default_value_t = 0)]
    pub refit_every_games: usize,

    #[arg(long, default_value_t = 1)]
    pub parallel: usize,

    #[arg(long, default_value_t = 42)]
    pub seed: u64,

    #[arg(long, default_value_t = 512)]
    pub max_plies: usize,

    #[arg(long, default_value = "backgammon")]
    pub variant: String,
}

#[derive(Debug, Clone, Args)]
pub struct EvalArgs {
    #[arg(long)]
    pub engine: String,

    #[arg(long, num_args = 1..)]
    pub opponents: Vec<String>,

    #[arg(long, default_value_t = 8)]
    pub opponent_count: usize,

    #[arg(long = "db")]
    pub db_path: Option<String>,

    #[arg(long, default_value_t = 2000)]
    pub budget_games: usize,

    #[arg(long, default_value_t = 200)]
    pub pair_games: usize,

    #[arg(long, default_value_t = 1)]
    pub parallel: usize,

    #[arg(long, default_value_t = 42)]
    pub seed: u64,

    #[arg(long, default_value_t = 512)]
    pub max_plies: usize,

    #[arg(long, default_value = "backgammon")]
    pub variant: String,
}

#[derive(Clone, Debug)]
struct EngineRating {
    name: String,
    rating: f64,
    rd: f64,
    volatility: f64,
    games: usize,
}

impl EngineRating {
    fn conservative(&self) -> f64 {
        self.rating - 2.0 * self.rd
    }
}

#[derive(Clone, Debug)]
struct DuelRow {
    points_a: f64,
    points_b: f64,
    winner: String,
    outcome: String,
}

#[derive(Clone, Debug)]
struct OrdinalGame {
    a_idx: usize,
    b_idx: usize,
    category: usize,
}

#[derive(Clone, Debug)]
struct EvalOrdinalGame {
    opp_idx: usize,
    category: usize,
}

impl EngineRating {
    fn uncertainty(&self) -> f64 {
        self.rd
    }
}

pub async fn run_ratings(args: RunRatingsArgs) -> Result<(), String> {
    let db_path = args.db_path.clone().unwrap_or_else(default_eval_db_path);
    let mut conn = open_ratings_db(&db_path)?;
    init_ratings_schema(&conn)?;

    if args.show {
        return show_leaderboard(&conn);
    }

    if args.reset_all {
        reset_ratings_state(&conn)?;
        println!("ratings DB reset: {}", db_path);
        return Ok(());
    }

    if args.budget_games == 0 {
        return Err("--budget-games must be > 0".to_string());
    }
    if args.pair_games == 0 {
        return Err("--pair-games must be > 0".to_string());
    }

    let mut unique_specs = Vec::new();
    for spec in &args.engines {
        if !unique_specs.iter().any(|e: &String| e.eq_ignore_ascii_case(spec)) {
            unique_specs.push(spec.clone());
        }
    }
    if unique_specs.len() < 2 {
        return Err("need at least two distinct engines".to_string());
    }

    if args.reset {
        reset_ratings_state(&conn)?;
    }

    let variant = parse_variant(&args.variant)?;
    let mut configs: HashMap<String, EngineConfig> = HashMap::new();
    let mut keys = Vec::new();
    for spec in &unique_specs {
        let (key, mut cfg) = resolve_engine_spec(spec)?;
        if configs.contains_key(&key) {
            continue;
        }
        cfg = filter_supported_engine_options(&cfg);
        cfg.name = key.clone();
        keys.push(key.clone());
        configs.insert(key, cfg);
    }
    if keys.len() < 2 {
        return Err("need at least two distinct engines".to_string());
    }

    let mut ratings = load_or_init_ratings(&conn, &keys)?;
    ratings = refit_ratings_from_raw_ordinal(&conn, &ratings)?;
    let index: HashMap<&str, usize> = ratings
        .iter()
        .enumerate()
        .map(|(i, r)| (r.name.as_str(), i))
        .collect();
    let mut pair_counts = load_pair_counts(&conn, &index)?;
    let mut total_games = load_meta_usize(&conn, "total_games").unwrap_or(0);
    let mut batch_idx = load_meta_usize(&conn, "batch_idx").unwrap_or(0);
    let mut adaptive_parallel =
        load_meta_usize(&conn, "adaptive_parallel").unwrap_or(args.parallel.max(1));
    let mut consecutive_spawn_failures = 0usize;
    let mut next_refit_games = if args.refit_every_games > 0 {
        Some(((total_games / args.refit_every_games) + 1) * args.refit_every_games)
    } else {
        None
    };

    if !args.tui {
        info!(
            db = %db_path,
            engines = ratings.len(),
            budget_games = args.budget_games,
            pair_games = args.pair_games,
            min_pair_games = args.min_pair_games,
            refit_every_games = args.refit_every_games,
            parallel = args.parallel.max(1),
            resumed_games = total_games,
            reset = args.reset,
            mirrored_pairs = true,
            model = "ordinal_bt_cl",
            variant = %args.variant,
            "running ratings"
        );
    }

    let mut live_status = format!(
        "db={} resumed_games={} reset={} mirrored_pairs=true",
        db_path, total_games, args.reset
    );
    if args.tui {
        render_dashboard(
            &ratings,
            total_games,
            args.budget_games,
            batch_idx,
            None,
            &live_status,
        )?;
    }

    while total_games < args.budget_games {
        let Some((a_idx, b_idx)) = choose_pair(&ratings, &pair_counts, args.min_pair_games) else {
            break;
        };
        let remaining = args.budget_games - total_games;
        let batch_games = remaining.min(args.pair_games);

        let a_name = ratings[a_idx].name.clone();
        let b_name = ratings[b_idx].name.clone();
        if args.tui {
            live_status = format!(
                "running batch {:04}: {} vs {}",
                batch_idx + 1,
                a_name,
                b_name
            );
            render_dashboard(
                &ratings,
                total_games,
                args.budget_games,
                batch_idx,
                Some((&a_name, &b_name)),
                &live_status,
            )?;
        }
        let cfg_a = configs
            .get(&a_name)
            .cloned()
            .ok_or_else(|| format!("missing config for {a_name}"))?;
        let cfg_b = configs
            .get(&b_name)
            .cloned()
            .ok_or_else(|| format!("missing config for {b_name}"))?;
        let sig_a = engine_signature(&cfg_a);
        let sig_b = engine_signature(&cfg_b);

        let paths = build_rating_paths(&a_name, &b_name, batch_idx);
        let mut batch_parallel = adaptive_parallel.min(batch_games.max(1));
        let mut resource_retries = 0usize;
        let summary: Result<_, String> = loop {
            let duel_cfg = DuelConfig {
                games: batch_games,
                parallel: batch_parallel,
                seed: mix_seed(args.seed, batch_idx),
                max_plies: args.max_plies.max(1),
                swap_sides: true,
                mirrored_pairs: true,
                variant: args.variant.clone(),
                log: "off".to_string(),
                timeout_secs: None,
                engine_a: cfg_a.clone(),
                engine_b: cfg_b.clone(),
            };

            match run_duel(&duel_cfg, variant, &paths, true).await {
                Ok(summary) => {
                    adaptive_parallel = batch_parallel;
                    break Ok(summary);
                }
                Err(err) if is_resource_exhaustion_error(&err) && batch_parallel > 1 => {
                    let next_parallel = (batch_parallel / 2).max(1);
                    if args.tui {
                        live_status = format!(
                            "resource pressure batch {:04}: {} vs {} parallel {} -> {}",
                            batch_idx + 1,
                            a_name,
                            b_name,
                            batch_parallel,
                            next_parallel
                        );
                        render_dashboard(
                            &ratings,
                            total_games,
                            args.budget_games,
                            batch_idx,
                            Some((&a_name, &b_name)),
                            &live_status,
                        )?;
                    } else {
                        warn!(
                            batch = batch_idx + 1,
                            engine_a = %a_name,
                            engine_b = %b_name,
                            parallel = batch_parallel,
                            next_parallel,
                            "resource pressure; reducing parallelism"
                        );
                    }
                    batch_parallel = next_parallel;
                    sleep(Duration::from_millis(250)).await;
                    continue;
                }
                Err(err) if is_resource_exhaustion_error(&err) => {
                    resource_retries += 1;
                    if resource_retries > 10 {
                        break Err(format!(
                            "resource pressure persists for batch {} ({} vs {}) after {} retries at parallel=1: {}",
                            batch_idx + 1,
                            a_name,
                            b_name,
                            resource_retries - 1,
                            err
                        ));
                    }
                    let wait_ms = (resource_retries as u64 * 750).min(6000);
                    if args.tui {
                        live_status = format!(
                            "resource pressure batch {:04}: {} vs {} cooldown={}ms retry={}/10",
                            batch_idx + 1,
                            a_name,
                            b_name,
                            wait_ms,
                            resource_retries
                        );
                        render_dashboard(
                            &ratings,
                            total_games,
                            args.budget_games,
                            batch_idx,
                            Some((&a_name, &b_name)),
                            &live_status,
                        )?;
                    } else {
                        warn!(
                            batch = batch_idx + 1,
                            engine_a = %a_name,
                            engine_b = %b_name,
                            cooldown_ms = wait_ms,
                            retry = resource_retries,
                            retry_max = 10,
                            "resource pressure at parallel=1; cooling down"
                        );
                    }
                    sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
                Err(err) => break Err(err),
            }
        };

        let summary = match summary {
            Ok(summary) => {
                consecutive_spawn_failures = 0;
                summary
            }
            Err(err) if is_resource_exhaustion_error(&err) => {
                consecutive_spawn_failures += 1;
                let cooldown_secs = (10usize * consecutive_spawn_failures).min(120) as u64;
                if !args.tui {
                    warn!(
                        batch = batch_idx + 1,
                        engine_a = %a_name,
                        engine_b = %b_name,
                        failures = consecutive_spawn_failures,
                        cooldown_secs,
                        "resource pressure persisted; skipping batch and cooling down"
                    );
                } else {
                    live_status = format!(
                        "resource pressure persisted ({}); cooldown {}s, skip batch",
                        consecutive_spawn_failures, cooldown_secs
                    );
                    render_dashboard(
                        &ratings,
                        total_games,
                        args.budget_games,
                        batch_idx,
                        Some((&a_name, &b_name)),
                        &live_status,
                    )?;
                }

                if consecutive_spawn_failures >= 20 {
                    return Err(format!(
                        "resource pressure persisted for {consecutive_spawn_failures} consecutive batches; last error: {err}. Try increasing ulimit -n and using larger --pair-games to reduce process churn."
                    ));
                }

                adaptive_parallel = 1;
                sleep(Duration::from_secs(cooldown_secs)).await;
                continue;
            }
            Err(err) => return Err(err),
        };
        live_status = format!(
            "batch {:04} {} vs {}: {}",
            batch_idx + 1,
            a_name,
            b_name,
            summary.line_result
        );
        if !args.tui {
            info!(
                batch = batch_idx + 1,
                engine_a = %a_name,
                engine_b = %b_name,
                result = %summary.line_result,
                "batch complete"
            );
        }

        let rows = read_duel_rows(&paths.output_csv)?;

        total_games += batch_games;
        batch_idx += 1;
        let key = ordered_pair(a_idx, b_idx);
        *pair_counts.entry(key).or_insert(0) += batch_games;
        persist_ratings_batch(
            &mut conn,
            &a_name,
            &b_name,
            &sig_a,
            &sig_b,
            batch_idx,
            &rows,
            &ratings,
            &pair_counts,
            total_games,
            adaptive_parallel,
        )?;

        let should_refit = if let Some(target_games) = next_refit_games {
            total_games >= target_games
        } else {
            true
        };
        if should_refit {
            ratings = refit_ratings_from_raw_ordinal(&conn, &ratings)?;
            let index: HashMap<&str, usize> = ratings
                .iter()
                .enumerate()
                .map(|(i, r)| (r.name.as_str(), i))
                .collect();
            pair_counts = load_pair_counts(&conn, &index)?;
            persist_ratings_state(
                &mut conn,
                &ratings,
                &pair_counts,
                total_games,
                batch_idx,
                adaptive_parallel,
            )?;
            if !args.tui {
                info!(games = total_games, "periodic global refit complete");
            } else {
                live_status = format!("periodic refit complete at {} games", total_games);
            }
            if args.refit_every_games > 0 {
                next_refit_games =
                    Some(((total_games / args.refit_every_games) + 1) * args.refit_every_games);
            }
        }

        if args.tui {
            render_dashboard(
                &ratings,
                total_games,
                args.budget_games,
                batch_idx,
                Some((&a_name, &b_name)),
                &live_status,
            )?;
        }
    }

    ratings.sort_by(|a, b| b.rating.partial_cmp(&a.rating).unwrap_or(Ordering::Equal));

    if !args.tui {
        info!(games = total_games, "final table");
        for (idx, r) in ratings.iter().enumerate() {
            info!(
                rank = idx + 1,
                engine = %r.name,
                rating = r.rating,
                conservative = r.conservative(),
                games = r.games,
                uncertainty = r.uncertainty(),
                "rating"
            );
        }
    }

    let next_pairs = suggest_pairs(&ratings, 10);
    if !next_pairs.is_empty() {
        if !args.tui {
            info!("next suggested pairings");
            for (idx, (a, b, score)) in next_pairs.iter().enumerate() {
                info!(rank = idx + 1, engine_a = %a, engine_b = %b, priority = *score, "pairing");
            }
        } else {
            render_dashboard(
                &ratings,
                total_games,
                args.budget_games,
                batch_idx,
                None,
                "completed",
            )?;
        }
    }

    Ok(())
}

pub async fn run_eval(args: EvalArgs) -> Result<(), String> {
    if args.budget_games == 0 {
        return Err("--budget-games must be > 0".to_string());
    }
    if args.pair_games == 0 {
        return Err("--pair-games must be > 0".to_string());
    }

    let db_path = args.db_path.clone().unwrap_or_else(default_eval_db_path);
    let conn = open_ratings_db(&db_path)?;
    init_ratings_schema(&conn)?;

    let variant = parse_variant(&args.variant)?;
    let eval_cfg = resolve_engine_reference(&args.engine)?;

    let mut pool = load_ratings_rows(&conn)?;
    pool.retain(|r| !r.name.eq_ignore_ascii_case(&args.engine));

    if !args.opponents.is_empty() {
        let wanted: Vec<String> = args
            .opponents
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect();
        pool.retain(|r| wanted.iter().any(|w| w == &r.name.to_ascii_lowercase()));
    } else {
        pool.sort_by(|a, b| b.games.cmp(&a.games));
        pool.truncate(args.opponent_count.max(1));
    }

    if pool.is_empty() {
        return Err(
            "no opponent ratings found in DB; run `bgci ratings --engines ...` first".to_string(),
        );
    }

    let mut opponent_cfgs: HashMap<String, EngineConfig> = HashMap::new();
    for opp in &pool {
        opponent_cfgs.insert(opp.name.clone(), resolve_engine_reference(&opp.name)?);
    }

    let mut candidate = EngineRating {
        name: args.engine.clone(),
        rating: 1500.0,
        rd: 350.0,
        volatility: 0.06,
        games: 0,
    };
    let mut eval_games: Vec<EvalOrdinalGame> = Vec::new();
    let mut total_games = 0usize;
    let mut batch_idx = 0usize;
    let mut pair_counts: HashMap<String, usize> = HashMap::new();

    info!(
        db = %db_path,
        engine = %args.engine,
        opponents = pool.len(),
        budget_games = args.budget_games,
        pair_games = args.pair_games,
        parallel = args.parallel,
        variant = %args.variant,
        "starting eval"
    );

    while total_games < args.budget_games {
        let remaining = args.budget_games - total_games;
        let batch_games = remaining.min(args.pair_games);

        let opp_idx = choose_eval_opponent(&candidate, &pool, &pair_counts)
            .ok_or_else(|| "failed to choose eval opponent".to_string())?;
        let opp = &pool[opp_idx];
        let opp_cfg = opponent_cfgs
            .get(&opp.name)
            .cloned()
            .ok_or_else(|| format!("missing engine config for {}", opp.name))?;

        let duel_cfg = DuelConfig {
            games: batch_games,
            parallel: args.parallel.max(1).min(batch_games.max(1)),
            seed: mix_seed(args.seed, batch_idx),
            max_plies: args.max_plies.max(1),
            swap_sides: true,
            mirrored_pairs: true,
            variant: args.variant.clone(),
            log: "off".to_string(),
            timeout_secs: None,
            engine_a: eval_cfg.clone(),
            engine_b: opp_cfg,
        };

        let paths = build_rating_paths(&candidate.name, &opp.name, batch_idx);
        let summary = run_duel(&duel_cfg, variant, &paths, true).await?;
        info!(
            batch = batch_idx + 1,
            engine = %candidate.name,
            opponent = %opp.name,
            result = %summary.line_result,
            "eval batch complete"
        );

        let rows = read_duel_rows(&paths.output_csv)?;
        for row in rows {
            if let Some(category) = outcome_category_from_row(&row) {
                eval_games.push(EvalOrdinalGame { opp_idx, category });
            }
        }

        candidate = refit_eval_candidate_ordinal(&pool, &eval_games)?;
        candidate.name = args.engine.clone();

        total_games += batch_games;
        batch_idx += 1;
        *pair_counts.entry(opp.name.clone()).or_insert(0) += batch_games;
        info!(
            rating = candidate.rating,
            rd = candidate.rd,
            volatility = candidate.volatility,
            games = candidate.games,
            "eval estimate"
        );
    }

    info!(
        engine = %candidate.name,
        rating = candidate.rating,
        rd = candidate.rd,
        conservative = candidate.rating - 2.0 * candidate.rd,
        games = candidate.games,
        "eval final"
    );

    Ok(())
}

fn render_dashboard(
    ratings: &[EngineRating],
    total_games: usize,
    budget_games: usize,
    batch_idx: usize,
    active_pair: Option<(&str, &str)>,
    status: &str,
) -> Result<(), String> {
    let mut sorted = ratings.to_vec();
    sorted.sort_by(|a, b| b.rating.partial_cmp(&a.rating).unwrap_or(Ordering::Equal));
    let next_pairs = suggest_pairs(&sorted, 6);

    let mut out = String::new();
    out.push_str("\x1b[2J\x1b[H");
    out.push_str("bgci ratings live\n");
    out.push_str(&format!(
        "games: {}/{}   batch: {}\n",
        total_games, budget_games, batch_idx
    ));
    match active_pair {
        Some((a, b)) => out.push_str(&format!("active: {} vs {}\n", a, b)),
        None => out.push_str("active: -\n"),
    }
    out.push_str(&format!("status: {}\n\n", status));

    out.push_str("leaderboard\n");
    out.push_str("rank engine                    rating    cons      rd    games\n");
    for (idx, r) in sorted.iter().take(12).enumerate() {
        out.push_str(&format!(
            "{:>4} {:<24} {:>8.1} {:>8.1} {:>7.1} {:>7}\n",
            idx + 1,
            r.name,
            r.rating,
            r.conservative(),
            r.rd,
            r.games
        ));
    }

    out.push_str("\nnext pairups\n");
    for (idx, (a, b, score)) in next_pairs.iter().enumerate() {
        out.push_str(&format!(
            "{:>2}. {:<18} vs {:<18} {:>9.3}\n",
            idx + 1,
            a,
            b,
            score
        ));
    }

    let mut stdout = io::stdout();
    stdout
        .write_all(out.as_bytes())
        .map_err(|e| format!("write tui: {e}"))?;
    stdout.flush().map_err(|e| format!("flush tui: {e}"))
}

fn ordered_pair(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

fn choose_pair(
    ratings: &[EngineRating],
    pair_counts: &HashMap<(usize, usize), usize>,
    min_pair_games: usize,
) -> Option<(usize, usize)> {
    let mut best_undercovered: Option<(usize, usize, f64, usize)> = None;
    let mut best_any: Option<(usize, usize, f64, usize)> = None;
    for i in 0..ratings.len() {
        for j in (i + 1)..ratings.len() {
            let a = &ratings[i];
            let b = &ratings[j];
            let games_between = *pair_counts.get(&(i, j)).unwrap_or(&0);
            let score = pair_information(a, b);

            match best_any {
                Some((_, _, best_score, best_games_between)) => {
                    if score > best_score
                        || ((score - best_score).abs() < 1e-12
                            && games_between < best_games_between)
                    {
                        best_any = Some((i, j, score, games_between));
                    }
                }
                None => best_any = Some((i, j, score, games_between)),
            }

            if games_between < min_pair_games {
                match best_undercovered {
                    Some((_, _, best_score, best_games_between)) => {
                        if score > best_score
                            || ((score - best_score).abs() < 1e-12
                                && games_between < best_games_between)
                        {
                            best_undercovered = Some((i, j, score, games_between));
                        }
                    }
                    None => best_undercovered = Some((i, j, score, games_between)),
                }
            }
        }
    }
    best_undercovered.or(best_any).map(|(i, j, _, _)| (i, j))
}

fn choose_eval_opponent(
    candidate: &EngineRating,
    pool: &[EngineRating],
    pair_counts: &HashMap<String, usize>,
) -> Option<usize> {
    let mut best: Option<(usize, f64, usize)> = None;
    for (idx, opp) in pool.iter().enumerate() {
        let repeats = *pair_counts.get(&opp.name).unwrap_or(&0);
        let score = pair_information(candidate, opp);
        match best {
            Some((_, best_score, best_repeats)) => {
                if score > best_score
                    || ((score - best_score).abs() < 1e-12 && repeats < best_repeats)
                {
                    best = Some((idx, score, repeats));
                }
            }
            None => best = Some((idx, score, repeats)),
        }
    }
    best.map(|(idx, _, _)| idx)
}

fn read_duel_rows(path: &Path) -> Result<Vec<DuelRow>, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut rows = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if idx == 0 {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 9 {
            continue;
        }
        let points_a = match cols[7].trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let points_b = match cols[8].trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        rows.push(DuelRow {
            points_a,
            points_b,
            winner: cols.get(3).map_or("", |v| *v).trim().to_string(),
            outcome: cols.get(4).map_or("", |v| *v).trim().to_string(),
        });
    }
    Ok(rows)
}

fn score_from_row(row: &DuelRow) -> Option<f64> {
    let outcome = row.outcome.to_ascii_lowercase();
    let winner = row.winner.to_ascii_lowercase();
    if outcome.contains("timeout")
        || outcome.contains("incomplete")
        || outcome.contains("abort")
        || outcome.contains("forfeit")
        || winner.contains("timeout")
        || winner.contains("incomplete")
    {
        return None;
    }

    let score = (((row.points_a - row.points_b) + 6.0) / 12.0).clamp(0.0, 1.0);
    Some(score)
}

fn build_rating_paths(engine_a: &str, engine_b: &str, batch_idx: usize) -> RunPaths {
    let root =
        Path::new("data")
            .join("ratings")
            .join(format!("{}-vs-{}", slug(engine_a), slug(engine_b)));
    let run_id = format!("batch-{batch_idx:05}");
    let output_csv = root.join(format!("results-{run_id}.csv"));
    let log_file = root.join(format!("duel-{run_id}.log"));
    let trace_games_dir = root.join(format!("games-{run_id}"));
    RunPaths {
        timestamp: run_id,
        output_csv,
        log_file,
        trace_games_dir,
    }
}

fn suggest_pairs(ratings: &[EngineRating], count: usize) -> Vec<(String, String, f64)> {
    let mut out = Vec::new();
    for i in 0..ratings.len() {
        for j in (i + 1)..ratings.len() {
            let a = &ratings[i];
            let b = &ratings[j];
            let score = pair_information(a, b);
            out.push((a.name.clone(), b.name.clone(), score));
        }
    }
    out.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(Ordering::Equal));
    out.truncate(count);
    out
}

fn pair_information(a: &EngineRating, b: &EngineRating) -> f64 {
    let p = 1.0 / (1.0 + 10f64.powf((b.rating - a.rating) / 400.0));
    let bernoulli_info = p * (1.0 - p);
    let posterior_variance = a.rd * a.rd + b.rd * b.rd;
    bernoulli_info * posterior_variance
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

fn open_ratings_db(db_path: &str) -> Result<Connection, String> {
    let db = PathBuf::from(db_path);
    if let Some(parent) = db.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("create db dir {}: {e}", parent.display()))?;
    }
    Connection::open(db).map_err(|e| format!("open db {db_path}: {e}"))
}

fn init_ratings_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS ratings_state (
          engine TEXT PRIMARY KEY,
          rating REAL NOT NULL,
          rd REAL NOT NULL,
          volatility REAL NOT NULL,
          games INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ratings_pairs (
          engine_a TEXT NOT NULL,
          engine_b TEXT NOT NULL,
          games INTEGER NOT NULL,
          PRIMARY KEY (engine_a, engine_b)
        );

        CREATE TABLE IF NOT EXISTS ratings_meta (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ratings_games (
          id INTEGER PRIMARY KEY,
          batch_idx INTEGER NOT NULL,
          engine_a TEXT NOT NULL,
          engine_b TEXT NOT NULL,
          points_a REAL NOT NULL,
          points_b REAL NOT NULL,
          winner TEXT,
          outcome TEXT,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS ratings_batches (
          batch_idx INTEGER PRIMARY KEY,
          engine_a TEXT NOT NULL,
          engine_b TEXT NOT NULL,
          engine_a_sig TEXT NOT NULL,
          engine_b_sig TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        ",
    )
    .map_err(|e| format!("init ratings schema: {e}"))
}

fn reset_ratings_state(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM ratings_state", [])
        .map_err(|e| format!("reset ratings_state: {e}"))?;
    conn.execute("DELETE FROM ratings_pairs", [])
        .map_err(|e| format!("reset ratings_pairs: {e}"))?;
    conn.execute("DELETE FROM ratings_meta", [])
        .map_err(|e| format!("reset ratings_meta: {e}"))?;
    conn.execute("DELETE FROM ratings_games", [])
        .map_err(|e| format!("reset ratings_games: {e}"))?;
    conn.execute("DELETE FROM ratings_batches", [])
        .map_err(|e| format!("reset ratings_batches: {e}"))?;
    Ok(())
}

fn show_leaderboard(conn: &Connection) -> Result<(), String> {
    let mut ratings = load_ratings_rows(conn)?;
    ratings.sort_by(|a, b| b.rating.partial_cmp(&a.rating).unwrap_or(Ordering::Equal));
    if ratings.is_empty() {
        println!("leaderboard is empty; run `bgci ratings --engines ...` first");
        return Ok(());
    }

    println!("bgci ratings");
    println!("engines: {}", ratings.len());
    println!("rank engine                               rating    cons      rd    games");
    for (idx, r) in ratings.iter().enumerate() {
        println!(
            "{:>4} {:<36} {:>8.1} {:>8.1} {:>7.1} {:>7}",
            idx + 1,
            r.name,
            r.rating,
            r.conservative(),
            r.rd,
            r.games,
        );
    }
    Ok(())
}

fn load_ratings_rows(conn: &Connection) -> Result<Vec<EngineRating>, String> {
    let mut stmt = conn
        .prepare("SELECT engine, rating, rd, volatility, games FROM ratings_state")
        .map_err(|e| format!("prepare ratings rows query: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(EngineRating {
                name: row.get(0)?,
                rating: row.get(1)?,
                rd: row.get(2)?,
                volatility: row.get(3)?,
                games: row.get::<_, i64>(4)? as usize,
            })
        })
        .map_err(|e| format!("query ratings rows: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("ratings row: {e}"))?);
    }
    Ok(out)
}

fn load_or_init_ratings(
    conn: &Connection,
    engines: &[String],
) -> Result<Vec<EngineRating>, String> {
    let mut out = Vec::with_capacity(engines.len());
    let mut stmt = conn
        .prepare("SELECT rating, rd, volatility, games FROM ratings_state WHERE engine = ?")
        .map_err(|e| format!("prepare load rating query: {e}"))?;

    for name in engines {
        let found = stmt
            .query_row(params![name], |row| {
                Ok(EngineRating {
                    name: name.clone(),
                    rating: row.get(0)?,
                    rd: row.get(1)?,
                    volatility: row.get(2)?,
                    games: row.get::<_, i64>(3)? as usize,
                })
            })
            .ok();

        out.push(found.unwrap_or_else(|| EngineRating {
            name: name.clone(),
            rating: 1500.0,
            rd: 350.0,
            volatility: 0.06,
            games: 0,
        }));
    }
    Ok(out)
}

fn load_pair_counts(
    conn: &Connection,
    index: &HashMap<&str, usize>,
) -> Result<HashMap<(usize, usize), usize>, String> {
    let mut out = HashMap::new();
    let mut stmt = conn
        .prepare("SELECT engine_a, engine_b, games FROM ratings_pairs")
        .map_err(|e| format!("prepare pair counts query: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as usize,
            ))
        })
        .map_err(|e| format!("query pair counts: {e}"))?;

    for row in rows {
        let (a, b, games) = row.map_err(|e| format!("pair row: {e}"))?;
        if let (Some(&i), Some(&j)) = (index.get(a.as_str()), index.get(b.as_str())) {
            out.insert(ordered_pair(i, j), games);
        }
    }
    Ok(out)
}

fn load_meta_usize(conn: &Connection, key: &str) -> Option<usize> {
    conn.query_row(
        "SELECT value FROM ratings_meta WHERE key = ?",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse::<usize>().ok())
}

fn persist_ratings_state(
    conn: &mut Connection,
    ratings: &[EngineRating],
    pair_counts: &HashMap<(usize, usize), usize>,
    total_games: usize,
    batch_idx: usize,
    adaptive_parallel: usize,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin ratings transaction: {e}"))?;

    for r in ratings {
        tx.execute(
            "INSERT INTO ratings_state(engine, rating, rd, volatility, games)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(engine) DO UPDATE SET
               rating=excluded.rating,
               rd=excluded.rd,
               volatility=excluded.volatility,
               games=excluded.games",
            params![r.name, r.rating, r.rd, r.volatility, r.games as i64],
        )
        .map_err(|e| format!("upsert ratings_state: {e}"))?;
    }

    tx.execute("DELETE FROM ratings_pairs", [])
        .map_err(|e| format!("clear ratings_pairs: {e}"))?;
    for (&(i, j), &games) in pair_counts {
        let a = &ratings[i].name;
        let b = &ratings[j].name;
        tx.execute(
            "INSERT INTO ratings_pairs(engine_a, engine_b, games) VALUES (?, ?, ?)",
            params![a, b, games as i64],
        )
        .map_err(|e| format!("insert ratings_pairs: {e}"))?;
    }

    let meta = [
        ("total_games", total_games),
        ("batch_idx", batch_idx),
        ("adaptive_parallel", adaptive_parallel),
    ];
    for (k, v) in meta {
        tx.execute(
            "INSERT INTO ratings_meta(key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![k, v.to_string()],
        )
        .map_err(|e| format!("upsert ratings_meta {k}: {e}"))?;
    }

    tx.commit()
        .map_err(|e| format!("commit ratings state: {e}"))
}

fn persist_ratings_batch(
    conn: &mut Connection,
    engine_a: &str,
    engine_b: &str,
    engine_a_sig: &str,
    engine_b_sig: &str,
    batch_idx: usize,
    rows: &[DuelRow],
    ratings: &[EngineRating],
    pair_counts: &HashMap<(usize, usize), usize>,
    total_games: usize,
    adaptive_parallel: usize,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin ratings batch transaction: {e}"))?;

    for r in ratings {
        tx.execute(
            "INSERT INTO ratings_state(engine, rating, rd, volatility, games)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(engine) DO UPDATE SET
               rating=excluded.rating,
               rd=excluded.rd,
               volatility=excluded.volatility,
               games=excluded.games",
            params![r.name, r.rating, r.rd, r.volatility, r.games as i64],
        )
        .map_err(|e| format!("upsert ratings_state: {e}"))?;
    }

    tx.execute("DELETE FROM ratings_pairs", [])
        .map_err(|e| format!("clear ratings_pairs: {e}"))?;
    for (&(i, j), &games) in pair_counts {
        let a = &ratings[i].name;
        let b = &ratings[j].name;
        tx.execute(
            "INSERT INTO ratings_pairs(engine_a, engine_b, games) VALUES (?, ?, ?)",
            params![a, b, games as i64],
        )
        .map_err(|e| format!("insert ratings_pairs: {e}"))?;
    }

    let meta = [
        ("total_games", total_games),
        ("batch_idx", batch_idx),
        ("adaptive_parallel", adaptive_parallel),
    ];
    for (k, v) in meta {
        tx.execute(
            "INSERT INTO ratings_meta(key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![k, v.to_string()],
        )
        .map_err(|e| format!("upsert ratings_meta {k}: {e}"))?;
    }

    tx.execute(
        "INSERT INTO ratings_batches(batch_idx, engine_a, engine_b, engine_a_sig, engine_b_sig)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(batch_idx) DO UPDATE SET
           engine_a=excluded.engine_a,
           engine_b=excluded.engine_b,
           engine_a_sig=excluded.engine_a_sig,
           engine_b_sig=excluded.engine_b_sig",
        params![
            batch_idx as i64,
            engine_a,
            engine_b,
            engine_a_sig,
            engine_b_sig
        ],
    )
    .map_err(|e| format!("upsert ratings_batches: {e}"))?;

    for row in rows {
        tx.execute(
            "INSERT INTO ratings_games(batch_idx, engine_a, engine_b, points_a, points_b, winner, outcome)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                batch_idx as i64,
                engine_a,
                engine_b,
                row.points_a,
                row.points_b,
                row.winner,
                row.outcome,
            ],
        )
        .map_err(|e| format!("insert ratings_games row: {e}"))?;
    }
    tx.commit()
        .map_err(|e| format!("commit ratings batch: {e}"))
}

fn refit_ratings_from_raw_ordinal(
    conn: &Connection,
    current: &[EngineRating],
) -> Result<Vec<EngineRating>, String> {
    let mut name_to_idx = HashMap::new();
    for (idx, r) in current.iter().enumerate() {
        name_to_idx.insert(r.name.clone(), idx);
    }

    let games = load_ordinal_games(conn, &name_to_idx)?;
    if games.is_empty() {
        return Ok(current.to_vec());
    }

    const ELO_TO_LOGIT: f64 = std::f64::consts::LN_10 / 400.0;
    const SIGMA_THETA_ELO: f64 = 300.0;
    const SIGMA_THRESH: f64 = 3.0;
    const MIN_GAP: f64 = 0.05;
    const LR: f64 = 0.03;
    const B1: f64 = 0.9;
    const B2: f64 = 0.999;
    const EPS: f64 = 1e-8;
    const ITERS: usize = 220;

    let n = current.len();
    let mut x: Vec<f64> = current
        .iter()
        .map(|r| (r.rating - 1500.0) * ELO_TO_LOGIT)
        .collect();
    let mut t = vec![-1.3, -0.6, 0.0, 0.6, 1.3];

    let mut mx = vec![0.0; n];
    let mut vx = vec![0.0; n];
    let mut mt = vec![0.0; 5];
    let mut vt = vec![0.0; 5];

    let sigma_x = SIGMA_THETA_ELO * ELO_TO_LOGIT;
    let prior_prec_x = 1.0 / (sigma_x * sigma_x);
    let prior_prec_t = 1.0 / (SIGMA_THRESH * SIGMA_THRESH);

    for it in 1..=ITERS {
        let mut gx = vec![0.0; n];
        let mut gt = vec![0.0; 5];

        for g in &games {
            let delta = x[g.a_idx] - x[g.b_idx];
            let (p, d_delta, d_tau) = category_model_terms(delta, &t);
            let y = g.category - 1;
            let py = p[y].max(1e-12);
            let inv_py = 1.0 / py;

            gx[g.a_idx] += d_delta[y] * inv_py;
            gx[g.b_idx] -= d_delta[y] * inv_py;
            for k in 0..5 {
                gt[k] += d_tau[y][k] * inv_py;
            }
        }

        for i in 0..n {
            gx[i] -= prior_prec_x * x[i];
        }
        for k in 0..5 {
            gt[k] -= prior_prec_t * t[k];
        }

        for i in 0..n {
            mx[i] = B1 * mx[i] + (1.0 - B1) * gx[i];
            vx[i] = B2 * vx[i] + (1.0 - B2) * gx[i] * gx[i];
            let mh = mx[i] / (1.0 - B1.powi(it as i32));
            let vh = vx[i] / (1.0 - B2.powi(it as i32));
            x[i] += LR * mh / (vh.sqrt() + EPS);
        }
        for k in 0..5 {
            mt[k] = B1 * mt[k] + (1.0 - B1) * gt[k];
            vt[k] = B2 * vt[k] + (1.0 - B2) * gt[k] * gt[k];
            let mh = mt[k] / (1.0 - B1.powi(it as i32));
            let vh = vt[k] / (1.0 - B2.powi(it as i32));
            t[k] += LR * mh / (vh.sqrt() + EPS);
        }

        for k in 1..5 {
            if t[k] < t[k - 1] + MIN_GAP {
                t[k] = t[k - 1] + MIN_GAP;
            }
        }
        let t_mean = t.iter().sum::<f64>() / 5.0;
        for tk in &mut t {
            *tk -= t_mean;
        }

        let x_mean = x.iter().sum::<f64>() / n as f64;
        for xi in &mut x {
            *xi -= x_mean;
        }
    }

    let mut info = vec![prior_prec_x; n];
    let mut games_played = vec![0usize; n];
    for g in &games {
        let delta = x[g.a_idx] - x[g.b_idx];
        let fisher = fisher_delta(delta, &t);
        info[g.a_idx] += fisher;
        info[g.b_idx] += fisher;
        games_played[g.a_idx] += 1;
        games_played[g.b_idx] += 1;
    }

    let mut out = Vec::with_capacity(n);
    for (idx, r) in current.iter().enumerate() {
        let sd_x = (1.0 / info[idx]).sqrt();
        let rd_elo = (sd_x / ELO_TO_LOGIT).clamp(5.0, 350.0);
        out.push(EngineRating {
            name: r.name.clone(),
            rating: 1500.0 + x[idx] / ELO_TO_LOGIT,
            rd: rd_elo,
            volatility: 0.06,
            games: games_played[idx],
        });
    }
    Ok(out)
}

fn load_ordinal_games(
    conn: &Connection,
    name_to_idx: &HashMap<String, usize>,
) -> Result<Vec<OrdinalGame>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT engine_a, engine_b, points_a, points_b, IFNULL(winner, ''), IFNULL(outcome, '') FROM ratings_games ORDER BY id ASC",
        )
        .map_err(|e| format!("prepare ratings_games query: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| format!("query ratings_games: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        let (a, b, points_a, points_b, winner, outcome) =
            row.map_err(|e| format!("ratings_games row: {e}"))?;
        let (Some(&a_idx), Some(&b_idx)) = (name_to_idx.get(&a), name_to_idx.get(&b)) else {
            continue;
        };
        let duel_row = DuelRow {
            points_a,
            points_b,
            winner,
            outcome,
        };
        let Some(category) = outcome_category_from_row(&duel_row) else {
            continue;
        };
        out.push(OrdinalGame {
            a_idx,
            b_idx,
            category,
        });
    }
    Ok(out)
}

fn refit_eval_candidate_ordinal(
    pool: &[EngineRating],
    games: &[EvalOrdinalGame],
) -> Result<EngineRating, String> {
    if games.is_empty() {
        return Ok(EngineRating {
            name: "candidate".to_string(),
            rating: 1500.0,
            rd: 350.0,
            volatility: 0.06,
            games: 0,
        });
    }

    const ELO_TO_LOGIT: f64 = std::f64::consts::LN_10 / 400.0;
    const SIGMA_CAND_ELO: f64 = 300.0;
    const SIGMA_OPP_LOCK_ELO: f64 = 80.0;
    const SIGMA_THRESH: f64 = 3.0;
    const MIN_GAP: f64 = 0.05;
    const LR: f64 = 0.03;
    const B1: f64 = 0.9;
    const B2: f64 = 0.999;
    const EPS: f64 = 1e-8;
    const ITERS: usize = 180;

    let opp_x: Vec<f64> = pool
        .iter()
        .map(|r| (r.rating - 1500.0) * ELO_TO_LOGIT)
        .collect();

    let mut x = 0.0;
    let mut t = vec![-1.3, -0.6, 0.0, 0.6, 1.3];
    let mut mx = 0.0;
    let mut vx = 0.0;
    let mut mt = vec![0.0; 5];
    let mut vt = vec![0.0; 5];

    let sigma_c = SIGMA_CAND_ELO * ELO_TO_LOGIT;
    let prior_prec_x = 1.0 / (sigma_c * sigma_c);
    let prior_prec_t = 1.0 / (SIGMA_THRESH * SIGMA_THRESH);
    let lock_prec = 1.0 / ((SIGMA_OPP_LOCK_ELO * ELO_TO_LOGIT).powi(2));

    for it in 1..=ITERS {
        let mut gx = 0.0;
        let mut gt = [0.0; 5];

        for g in games {
            let delta = x - opp_x[g.opp_idx];
            let (p, d_delta, d_tau) = category_model_terms(delta, &t);
            let y = g.category - 1;
            let py = p[y].max(1e-12);
            let inv_py = 1.0 / py;
            gx += d_delta[y] * inv_py;
            for k in 0..5 {
                gt[k] += d_tau[y][k] * inv_py;
            }
        }

        gx -= prior_prec_x * x;
        gx -= lock_prec * x;
        for k in 0..5 {
            gt[k] -= prior_prec_t * t[k];
        }

        mx = B1 * mx + (1.0 - B1) * gx;
        vx = B2 * vx + (1.0 - B2) * gx * gx;
        let mh = mx / (1.0 - B1.powi(it as i32));
        let vh = vx / (1.0 - B2.powi(it as i32));
        x += LR * mh / (vh.sqrt() + EPS);

        for k in 0..5 {
            mt[k] = B1 * mt[k] + (1.0 - B1) * gt[k];
            vt[k] = B2 * vt[k] + (1.0 - B2) * gt[k] * gt[k];
            let mh = mt[k] / (1.0 - B1.powi(it as i32));
            let vh = vt[k] / (1.0 - B2.powi(it as i32));
            t[k] += LR * mh / (vh.sqrt() + EPS);
        }

        for k in 1..5 {
            if t[k] < t[k - 1] + MIN_GAP {
                t[k] = t[k - 1] + MIN_GAP;
            }
        }
        let tm = t.iter().sum::<f64>() / 5.0;
        for tk in &mut t {
            *tk -= tm;
        }
    }

    let mut info = prior_prec_x + lock_prec;
    for g in games {
        let delta = x - opp_x[g.opp_idx];
        info += fisher_delta(delta, &t);
    }
    let sd_x = (1.0 / info).sqrt();
    let rd_elo = (sd_x / ELO_TO_LOGIT).clamp(5.0, 350.0);

    Ok(EngineRating {
        name: "candidate".to_string(),
        rating: 1500.0 + x / ELO_TO_LOGIT,
        rd: rd_elo,
        volatility: 0.06,
        games: games.len(),
    })
}

fn outcome_category_from_row(row: &DuelRow) -> Option<usize> {
    if score_from_row(row).is_none() {
        return None;
    }
    let d = row.points_a - row.points_b;
    let y = if d <= -2.5 {
        1
    } else if d <= -1.5 {
        2
    } else if d < 0.0 {
        3
    } else if d < 1.5 {
        4
    } else if d < 2.5 {
        5
    } else {
        6
    };
    Some(y)
}

fn category_model_terms(delta: f64, t: &[f64]) -> ([f64; 6], [f64; 6], [[f64; 5]; 6]) {
    let mut f = [0.0; 5];
    let mut ff = [0.0; 5];
    for k in 0..5 {
        f[k] = sigmoid(t[k] - delta);
        ff[k] = f[k] * (1.0 - f[k]);
    }

    let mut p = [0.0; 6];
    p[0] = f[0];
    p[1] = f[1] - f[0];
    p[2] = f[2] - f[1];
    p[3] = f[3] - f[2];
    p[4] = f[4] - f[3];
    p[5] = 1.0 - f[4];

    let mut d_delta = [0.0; 6];
    d_delta[0] = -ff[0];
    d_delta[1] = ff[0] - ff[1];
    d_delta[2] = ff[1] - ff[2];
    d_delta[3] = ff[2] - ff[3];
    d_delta[4] = ff[3] - ff[4];
    d_delta[5] = ff[4];

    let mut d_tau = [[0.0; 5]; 6];
    d_tau[0][0] = ff[0];
    d_tau[1][0] = -ff[0];
    d_tau[1][1] = ff[1];
    d_tau[2][1] = -ff[1];
    d_tau[2][2] = ff[2];
    d_tau[3][2] = -ff[2];
    d_tau[3][3] = ff[3];
    d_tau[4][3] = -ff[3];
    d_tau[4][4] = ff[4];
    d_tau[5][4] = -ff[4];

    for k in 0..6 {
        p[k] = p[k].max(1e-12);
    }

    (p, d_delta, d_tau)
}

fn fisher_delta(delta: f64, t: &[f64]) -> f64 {
    let (p, d_delta, _) = category_model_terms(delta, t);
    let mut out = 0.0;
    for k in 0..6 {
        out += (d_delta[k] * d_delta[k]) / p[k];
    }
    out.max(1e-9)
}

fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        let z = (-x).exp();
        1.0 / (1.0 + z)
    } else {
        let z = x.exp();
        z / (1.0 + z)
    }
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

fn engine_signature(cfg: &EngineConfig) -> String {
    let mut parts = Vec::new();
    parts.push(format!("cmd={}", cfg.command.join("\u{1f}")));
    let mut envs: Vec<(&String, &String)> = cfg.env.iter().collect();
    envs.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in envs {
        parts.push(format!("env:{k}={v}"));
    }
    let mut opts: Vec<(&String, &String)> = cfg.options.iter().collect();
    opts.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in opts {
        parts.push(format!("opt:{k}={v}"));
    }
    parts.join("\u{1e}")
}

fn mix_seed(base: u64, idx: usize) -> u64 {
    let mut z = base.wrapping_add((idx as u64).wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

fn is_resource_exhaustion_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("resource temporarily unavailable")
        || lower.contains("os error 35")
        || lower.contains("too many open files")
        || lower.contains("os error 24")
}
