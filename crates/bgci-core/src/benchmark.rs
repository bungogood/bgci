use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::config::EngineConfig;
use crate::duel_game::seed_for_game;
use crate::duel_runner::GameRecord;
use crate::ranking::RankingEdge;

const SCHEMA_VERSION: i64 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BenchmarkKind {
    Duel,
    League,
    Ranking,
}

impl BenchmarkKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Duel => "duel",
            Self::League => "league",
            Self::Ranking => "ranking",
        }
    }
}

#[derive(Debug)]
pub struct BenchmarkSummary {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub variant: String,
    pub requested_pairs: usize,
    pub completed_pairs: usize,
    pub games: usize,
}

#[derive(Debug)]
pub struct EngineSummary {
    pub family: Option<String>,
    pub name: String,
    pub role: String,
    pub games: usize,
    pub wins: usize,
    pub points: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct MatchupHandle {
    id: i64,
    engine_a_id: i64,
    engine_b_id: i64,
    pairs: usize,
    seed: u64,
}

impl MatchupHandle {
    pub fn seed(self) -> u64 {
        self.seed
    }
}

pub struct StartedBenchmark {
    pub id: i64,
    pub matchups: Vec<MatchupHandle>,
}

pub struct BenchmarkSpec<'a> {
    pub name: &'a str,
    pub variant: &'a str,
    pub seed: u64,
    pub max_plies: usize,
    pub pairs: usize,
}

pub struct RankingSpec<'a> {
    pub name: &'a str,
    pub variant: &'a str,
    pub seed: u64,
    pub max_plies: usize,
    pub placement_opponents: usize,
    pub placement_pairs: usize,
    pub established_rd: f64,
}

#[derive(Clone, Debug)]
pub struct RankingEngine {
    build_id: i64,
    identity: String,
    pub name: String,
    pub family: Option<String>,
    pub version: Option<String>,
    pub configuration: BTreeMap<String, String>,
    pub config: EngineConfig,
}

pub struct RankingPool {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub variant: String,
    pub seed: u64,
    pub max_plies: usize,
    pub next_batch: usize,
    pub placement_opponents: usize,
    pub placement_pairs: usize,
    pub established_rd: f64,
    pub engines: Vec<RankingEngine>,
}

pub struct RankingData {
    pub edges: Vec<RankingEdge>,
    pub pair_counts: Vec<Vec<usize>>,
    pub average_decision_time: Vec<Option<Duration>>,
    pub last_played_batch: Vec<Option<usize>>,
}

pub struct BenchmarkStore {
    conn: Connection,
}

impl BenchmarkStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create database directory {}: {e}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .map_err(|e| format!("open benchmark database {}: {e}", path.display()))?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(|e| format!("configure benchmark database: {e}"))?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn start_duel(
        &mut self,
        spec: BenchmarkSpec<'_>,
        engine_a: &EngineConfig,
        engine_b: &EngineConfig,
    ) -> Result<StartedBenchmark, String> {
        if engine_identity(engine_a)? == engine_identity(engine_b)? {
            return Err("saved duel requires two distinct resolved engines".to_string());
        }
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin duel transaction: {e}"))?;
        let benchmark_id = insert_benchmark(
            &tx,
            spec.name,
            BenchmarkKind::Duel,
            spec.variant,
            spec.seed,
            spec.max_plies,
            spec.pairs,
        )?;
        let engine_a_id = add_engine(&tx, benchmark_id, "engine-a", engine_a)?;
        let engine_b_id = add_engine(&tx, benchmark_id, "engine-b", engine_b)?;
        let matchup = add_matchup(
            &tx,
            benchmark_id,
            engine_a_id,
            engine_b_id,
            spec.pairs,
            spec.seed,
            0,
        )?;
        tx.commit()
            .map_err(|e| format!("commit duel transaction: {e}"))?;
        Ok(StartedBenchmark {
            id: benchmark_id,
            matchups: vec![matchup],
        })
    }

    pub fn start_league(
        &mut self,
        spec: BenchmarkSpec<'_>,
        engines: &[EngineConfig],
    ) -> Result<StartedBenchmark, String> {
        let matchup_count = engines.len() * (engines.len() - 1) / 2;
        let requested_pairs = spec
            .pairs
            .checked_mul(matchup_count)
            .ok_or_else(|| "league pair count is too large".to_string())?;
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin league transaction: {e}"))?;
        let benchmark_id = insert_benchmark(
            &tx,
            spec.name,
            BenchmarkKind::League,
            spec.variant,
            spec.seed,
            spec.max_plies,
            requested_pairs,
        )?;
        let mut engine_ids = Vec::with_capacity(engines.len());
        for engine in engines {
            engine_ids.push(add_engine(&tx, benchmark_id, "member", engine)?);
        }
        let mut matchups = Vec::with_capacity(matchup_count);
        for a in 0..engines.len() {
            for b in (a + 1)..engines.len() {
                let matchup_seed = seed_for_game(spec.seed, matchups.len());
                matchups.push(add_matchup(
                    &tx,
                    benchmark_id,
                    engine_ids[a],
                    engine_ids[b],
                    spec.pairs,
                    matchup_seed,
                    matchups.len(),
                )?);
            }
        }
        tx.commit()
            .map_err(|e| format!("commit league transaction: {e}"))?;
        Ok(StartedBenchmark {
            id: benchmark_id,
            matchups,
        })
    }

    pub fn start_ranking(
        &mut self,
        spec: RankingSpec<'_>,
        engines: &[EngineConfig],
    ) -> Result<RankingPool, String> {
        if engines.len() < 2 {
            return Err("ranking requires at least two engines".to_string());
        }
        let mut identities = HashSet::new();
        for engine in engines {
            if !identities.insert(engine_identity(engine)?) {
                return Err(format!("duplicate resolved engine: {}", engine.name));
            }
        }

        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin ranking transaction: {e}"))?;
        let benchmark_id = insert_benchmark(
            &tx,
            spec.name,
            BenchmarkKind::Ranking,
            spec.variant,
            spec.seed,
            spec.max_plies,
            0,
        )?;
        for engine in engines {
            add_engine(&tx, benchmark_id, "member", engine)?;
        }
        tx.execute(
            "UPDATE benchmarks
             SET status = 'paused', completed_at = CURRENT_TIMESTAMP,
                 placement_opponents = ?, placement_pairs = ?, established_rd = ?
             WHERE id = ?",
            params![
                spec.placement_opponents as i64,
                spec.placement_pairs as i64,
                spec.established_rd,
                benchmark_id
            ],
        )
        .map_err(|e| format!("configure new ranking: {e}"))?;
        tx.commit()
            .map_err(|e| format!("commit ranking transaction: {e}"))?;
        self.load_ranking(benchmark_id)
    }

    pub fn load_ranking(&self, benchmark_id: i64) -> Result<RankingPool, String> {
        let (
            name,
            status,
            variant,
            seed,
            max_plies,
            placement_opponents,
            placement_pairs,
            established_rd,
        ): (String, String, String, String, i64, i64, i64, f64) = self
            .conn
            .query_row(
                "SELECT name, status, variant, seed, max_plies,
                        placement_opponents, placement_pairs, established_rd
                 FROM benchmarks WHERE id = ? AND kind = 'ranking'",
                params![benchmark_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .map_err(|e| format!("load ranking {benchmark_id}: {e}"))?;
        let seed = seed
            .parse::<u64>()
            .map_err(|e| format!("parse ranking seed: {e}"))?;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT eb.id, eb.identity, eb.name, be.family, be.version, be.configuration_json,
                        eb.command_json, eb.env_json, eb.options_json
                 FROM benchmark_engines be
                 JOIN engine_builds eb ON eb.id = be.engine_build_id
                 WHERE be.benchmark_id = ?
                 ORDER BY eb.name",
            )
            .map_err(|e| format!("prepare ranking engines: {e}"))?;
        let rows = stmt
            .query_map(params![benchmark_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(|e| format!("query ranking engines: {e}"))?;
        let mut engines = Vec::new();
        for row in rows {
            let (build_id, identity, name, family, version, configuration, command, env, options) =
                row.map_err(|e| format!("read ranking engine: {e}"))?;
            let configuration: BTreeMap<String, String> = serde_json::from_str(&configuration)
                .map_err(|e| format!("parse engine display configuration: {e}"))?;
            engines.push(RankingEngine {
                build_id,
                identity,
                config: EngineConfig {
                    name: name.clone(),
                    family: family.clone(),
                    version: version.clone(),
                    configuration: configuration.clone(),
                    engine: None,
                    command: serde_json::from_str(&command)
                        .map_err(|e| format!("parse engine command: {e}"))?,
                    env: serde_json::from_str(&env)
                        .map_err(|e| format!("parse engine environment: {e}"))?,
                    options: serde_json::from_str(&options)
                        .map_err(|e| format!("parse engine options: {e}"))?,
                },
                name,
                family,
                version,
                configuration,
            });
        }
        let next_batch: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(batch_index) + 1, 0) FROM matchups WHERE benchmark_id = ?",
                params![benchmark_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("load ranking batch index: {e}"))?;
        Ok(RankingPool {
            id: benchmark_id,
            name,
            status,
            variant,
            seed,
            max_plies: max_plies as usize,
            next_batch: next_batch as usize,
            placement_opponents: placement_opponents as usize,
            placement_pairs: placement_pairs as usize,
            established_rd,
            engines,
        })
    }

    pub fn load_ranking_by_name(&self, name: &str) -> Result<RankingPool, String> {
        let benchmark_id: i64 = self
            .conn
            .query_row(
                "SELECT id FROM benchmarks
                 WHERE kind = 'ranking' AND name = ? COLLATE NOCASE",
                params![name],
                |row| row.get(0),
            )
            .map_err(|e| format!("load ranking '{name}': {e}"))?;
        self.load_ranking(benchmark_id)
    }

    pub fn add_ranking_engines(
        &mut self,
        name: &str,
        engines: &[EngineConfig],
    ) -> Result<RankingPool, String> {
        if engines.is_empty() {
            return Err("no engines supplied".to_string());
        }
        let pool = self.load_ranking_by_name(name)?;
        if pool.status != "paused" {
            return Err(format!(
                "ranking '{}' must be paused before adding engines",
                pool.name
            ));
        }
        let mut identities = pool
            .engines
            .iter()
            .map(|engine| engine.identity.clone())
            .collect::<HashSet<_>>();
        for engine in engines {
            if !identities.insert(engine_identity(engine)?) {
                return Err(format!(
                    "engine already belongs to ranking: {}",
                    engine.name
                ));
            }
        }
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin add ranking engines transaction: {e}"))?;
        let status: String = tx
            .query_row(
                "SELECT status FROM benchmarks WHERE id = ? AND kind = 'ranking'",
                params![pool.id],
                |row| row.get(0),
            )
            .map_err(|e| format!("recheck ranking status: {e}"))?;
        if status != "paused" {
            return Err(format!(
                "ranking '{}' must remain paused while adding engines",
                pool.name
            ));
        }
        for engine in engines {
            add_engine(&tx, pool.id, "member", engine)?;
        }
        tx.commit()
            .map_err(|e| format!("commit ranking engines: {e}"))?;
        self.load_ranking(pool.id)
    }

    pub fn refresh_ranking_engine_metadata(
        &mut self,
        name: &str,
        engines: &[EngineConfig],
    ) -> Result<RankingPool, String> {
        let pool = self.load_ranking_by_name(name)?;
        if pool.status != "paused" {
            return Err(format!(
                "ranking '{}' must be paused before refreshing metadata",
                pool.name
            ));
        }
        let configs = engines
            .iter()
            .map(|engine| Ok((engine_identity(engine)?, engine)))
            .collect::<Result<HashMap<_, _>, String>>()?;
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin refresh ranking metadata transaction: {e}"))?;
        for participant in &pool.engines {
            let config = configs.get(&participant.identity).ok_or_else(|| {
                format!("no current metadata found for engine {}", participant.name)
            })?;
            let configuration = serde_json::to_string(&config.configuration)
                .map_err(|e| format!("serialize engine display configuration: {e}"))?;
            tx.execute(
                "UPDATE benchmark_engines
                 SET family = ?, version = ?, configuration_json = ?
                 WHERE benchmark_id = ? AND engine_build_id = ? AND role = 'member'",
                params![
                    config.family.as_deref(),
                    config.version.as_deref(),
                    configuration,
                    pool.id,
                    participant.build_id,
                ],
            )
            .map_err(|e| format!("refresh metadata for {}: {e}", participant.name))?;
        }
        tx.commit()
            .map_err(|e| format!("commit ranking metadata refresh: {e}"))?;
        self.load_ranking(pool.id)
    }

    pub fn resume_ranking(&self, benchmark_id: i64) -> Result<(), String> {
        let changed = self
            .conn
            .execute(
                "UPDATE benchmarks SET status = 'running', completed_at = NULL
                 WHERE id = ? AND kind = 'ranking' AND status IN ('running', 'paused')",
                params![benchmark_id],
            )
            .map_err(|e| format!("resume ranking: {e}"))?;
        if changed == 0 {
            return Err(format!("paused ranking {benchmark_id} not found"));
        }
        Ok(())
    }

    pub fn pause_ranking(&self, benchmark_id: i64) -> Result<(), String> {
        let changed = self
            .conn
            .execute(
                "UPDATE benchmarks SET status = 'paused', completed_at = CURRENT_TIMESTAMP
                 WHERE id = ? AND kind = 'ranking' AND status = 'running'",
                params![benchmark_id],
            )
            .map_err(|e| format!("pause ranking: {e}"))?;
        if changed == 0 {
            return Err(format!("running ranking {benchmark_id} not found"));
        }
        Ok(())
    }

    pub fn start_ranking_batch(
        &mut self,
        pool: &RankingPool,
        engine_a: usize,
        engine_b: usize,
        pairs: usize,
    ) -> Result<MatchupHandle, String> {
        if pool.status != "running" {
            return Err(format!("ranking {} is not running", pool.id));
        }
        if engine_a >= pool.engines.len() || engine_b >= pool.engines.len() || engine_a == engine_b
        {
            return Err("invalid ranking matchup engines".to_string());
        }
        if pairs == 0 {
            return Err("ranking batch must contain at least one pair".to_string());
        }
        let seed = seed_for_game(pool.seed, pool.next_batch);
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin ranking batch transaction: {e}"))?;
        let matchup = add_matchup(
            &tx,
            pool.id,
            pool.engines[engine_a].build_id,
            pool.engines[engine_b].build_id,
            pairs,
            seed,
            pool.next_batch,
        )?;
        tx.commit()
            .map_err(|e| format!("commit ranking batch: {e}"))?;
        Ok(matchup)
    }

    pub fn discard_empty_matchup(&self, matchup: MatchupHandle) -> Result<(), String> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM matchups
                 WHERE id = ? AND NOT EXISTS (
                   SELECT 1 FROM pairs WHERE matchup_id = matchups.id
                 )",
                params![matchup.id],
            )
            .map_err(|e| format!("discard empty matchup {}: {e}", matchup.id))?;
        if changed == 0 {
            return Err(format!(
                "matchup {} cannot be discarded after results were recorded",
                matchup.id
            ));
        }
        Ok(())
    }

    pub fn ranking_data(&self, pool: &RankingPool) -> Result<RankingData, String> {
        let index_by_build = pool
            .engines
            .iter()
            .enumerate()
            .map(|(index, engine)| (engine.build_id, index))
            .collect::<HashMap<_, _>>();
        let mut pair_counts = vec![vec![0; pool.engines.len()]; pool.engines.len()];
        let mut edge_stmt = self
            .conn
            .prepare(
                "SELECT engine_lo_id, engine_hi_id, completed_pairs, rated_games,
                        score_sum_lo, sum_m_squared, sum_m_score, sum_score_squared
                 FROM ranking_edge_stats
                 WHERE benchmark_id = ?
                 ORDER BY engine_lo_id, engine_hi_id",
            )
            .map_err(|e| format!("prepare ranking edges: {e}"))?;
        let edge_rows = edge_stmt
            .query_map(params![pool.id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                    row.get::<_, f64>(7)?,
                ))
            })
            .map_err(|e| format!("query ranking edges: {e}"))?;
        let mut edges = Vec::new();
        for row in edge_rows {
            let (lo_id, hi_id, pairs, games, score, m2, m_score, score2) =
                row.map_err(|e| format!("read ranking edge: {e}"))?;
            if let (Some(&lo), Some(&hi)) = (index_by_build.get(&lo_id), index_by_build.get(&hi_id))
            {
                pair_counts[lo][hi] = pairs as usize;
                pair_counts[hi][lo] = pairs as usize;
                edges.push(RankingEdge {
                    engine_a: lo,
                    engine_b: hi,
                    completed_pairs: pairs as usize,
                    rated_games: games as usize,
                    score_sum_a: score,
                    sum_m_squared: m2,
                    sum_m_score: m_score,
                    sum_score_squared: score2,
                });
            }
        }

        let mut engine_stmt = self
            .conn
            .prepare(
                "SELECT engine_id, decision_count, decision_seconds, last_played_batch
                 FROM ranking_engine_stats
                 WHERE benchmark_id = ?",
            )
            .map_err(|e| format!("prepare ranking engine stats: {e}"))?;
        let engine_rows = engine_stmt
            .query_map(params![pool.id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })
            .map_err(|e| format!("query ranking engine stats: {e}"))?;
        let mut decision_counts = vec![0_usize; pool.engines.len()];
        let mut decision_seconds = vec![0.0_f64; pool.engines.len()];
        let mut last_played_batch = vec![None; pool.engines.len()];
        for row in engine_rows {
            let (build_id, decisions, seconds, batch) =
                row.map_err(|e| format!("read ranking engine stats: {e}"))?;
            if let Some(&index) = index_by_build.get(&build_id) {
                decision_counts[index] = decisions as usize;
                decision_seconds[index] = seconds;
                last_played_batch[index] = batch.map(|value| value as usize);
            }
        }
        let average_decision_time = decision_seconds
            .into_iter()
            .zip(decision_counts)
            .map(|(seconds, decisions)| {
                (decisions > 0).then(|| Duration::from_secs_f64(seconds / decisions as f64))
            })
            .collect();
        Ok(RankingData {
            edges,
            pair_counts,
            average_decision_time,
            last_played_batch,
        })
    }

    pub fn record_games(
        &mut self,
        matchup: MatchupHandle,
        games: &[GameRecord],
    ) -> Result<(), String> {
        validate_games(matchup, games)?;
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin result transaction: {e}"))?;
        for pair_index in 0..matchup.pairs {
            let first_game = pair_index * 2;
            let pair_games = &games[first_game..first_game + 2];
            tx.execute(
                "INSERT INTO pairs(matchup_id, pair_index, seed, status)
                 VALUES (?, ?, ?, 'running')",
                params![
                    matchup.id,
                    pair_index as i64,
                    seed_for_game(matchup.seed, pair_index).to_string()
                ],
            )
            .map_err(|e| format!("insert pair: {e}"))?;
            let pair_id = tx.last_insert_rowid();
            for game in pair_games {
                let (engine_x_id, engine_o_id) = if game.a_is_x {
                    (matchup.engine_a_id, matchup.engine_b_id)
                } else {
                    (matchup.engine_b_id, matchup.engine_a_id)
                };
                let winner_id = game.winner_a.map(|a_won| {
                    if a_won {
                        matchup.engine_a_id
                    } else {
                        matchup.engine_b_id
                    }
                });
                let (decisions_a, decisions_b, decision_seconds_a, decision_seconds_b) = (
                    game.a_decisions,
                    game.b_decisions,
                    game.a_decision_time.as_secs_f64(),
                    game.b_decision_time.as_secs_f64(),
                );
                tx.execute(
                    "INSERT INTO games(
                        pair_id, leg, game_index, engine_x_id, engine_o_id, winner_id,
                        outcome, points_x, points_o, points_a, points_b, plies,
                        decisions_a, decisions_b, decision_seconds_a, decision_seconds_b
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        pair_id,
                        (game.game_idx % 2) as i64,
                        game.game_idx as i64,
                        engine_x_id,
                        engine_o_id,
                        winner_id,
                        game.outcome.as_ref().map(|outcome| outcome.as_str()),
                        game.points_x,
                        game.points_o,
                        game.points_a,
                        game.points_b,
                        game.plies as i64,
                        decisions_a as i64,
                        decisions_b as i64,
                        decision_seconds_a,
                        decision_seconds_b
                    ],
                )
                .map_err(|e| format!("insert game {}: {e}", game.game_idx))?;
            }
            tx.execute(
                "UPDATE pairs SET status = 'completed' WHERE id = ?",
                params![pair_id],
            )
            .map_err(|e| format!("complete pair: {e}"))?;

            update_ranking_pair_projection(&tx, matchup, pair_games)?;
        }
        tx.commit().map_err(|e| format!("commit results: {e}"))
    }

    pub fn finish_benchmark(&self, benchmark_id: i64) -> Result<(), String> {
        let changed = self
            .conn
            .execute(
                "UPDATE benchmarks SET status = 'completed', completed_at = CURRENT_TIMESTAMP
                 WHERE id = ? AND status = 'running'
                   AND requested_pairs = (
                     SELECT COUNT(*) FROM pairs p
                     JOIN matchups m ON m.id = p.matchup_id
                     WHERE m.benchmark_id = benchmarks.id AND p.status = 'completed'
                   )",
                params![benchmark_id],
            )
            .map_err(|e| format!("finish benchmark: {e}"))?;
        if changed == 0 {
            return Err(format!(
                "benchmark {benchmark_id} cannot complete before all requested pairs are stored"
            ));
        }
        Ok(())
    }

    pub fn fail_benchmark(&self, benchmark_id: i64) -> Result<(), String> {
        let changed = self
            .conn
            .execute(
                "UPDATE benchmarks SET status = 'failed', completed_at = CURRENT_TIMESTAMP
                 WHERE id = ? AND status = 'running'",
                params![benchmark_id],
            )
            .map_err(|e| format!("fail benchmark: {e}"))?;
        if changed == 0 {
            return Err(format!("running benchmark {benchmark_id} not found"));
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<BenchmarkSummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT b.id, b.name, b.kind, b.status, b.variant, b.requested_pairs,
                        COUNT(DISTINCT CASE WHEN p.status = 'completed' THEN p.id END),
                        COUNT(DISTINCT g.id)
                 FROM benchmarks b
                 LEFT JOIN matchups m ON m.benchmark_id = b.id
                 LEFT JOIN pairs p ON p.matchup_id = m.id
                 LEFT JOIN games g ON g.pair_id = p.id
                 GROUP BY b.id
                 ORDER BY b.id DESC",
            )
            .map_err(|e| format!("prepare benchmark list: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(BenchmarkSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    status: row.get(3)?,
                    variant: row.get(4)?,
                    requested_pairs: row.get::<_, i64>(5)? as usize,
                    completed_pairs: row.get::<_, i64>(6)? as usize,
                    games: row.get::<_, i64>(7)? as usize,
                })
            })
            .map_err(|e| format!("query benchmark list: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read benchmark list: {e}"))
    }

    pub fn list_rankings(&self) -> Result<Vec<BenchmarkSummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT b.id, b.name, b.kind, b.status, b.variant, b.requested_pairs,
                        COALESCE(SUM(e.completed_pairs), 0),
                        COALESCE(SUM(e.rated_games), 0)
                 FROM benchmarks b
                 LEFT JOIN ranking_edge_stats e ON e.benchmark_id = b.id
                 WHERE b.kind = 'ranking'
                 GROUP BY b.id
                 ORDER BY b.id DESC",
            )
            .map_err(|e| format!("prepare ranking list: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(BenchmarkSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    status: row.get(3)?,
                    variant: row.get(4)?,
                    requested_pairs: row.get::<_, i64>(5)? as usize,
                    completed_pairs: row.get::<_, i64>(6)? as usize,
                    games: row.get::<_, i64>(7)? as usize,
                })
            })
            .map_err(|e| format!("query ranking list: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read ranking list: {e}"))
    }

    pub fn get(&self, id: i64) -> Result<Option<BenchmarkSummary>, String> {
        self.conn
            .query_row(
                "SELECT b.id, b.name, b.kind, b.status, b.variant, b.requested_pairs,
                        COUNT(DISTINCT CASE WHEN p.status = 'completed' THEN p.id END),
                        COUNT(DISTINCT g.id)
                 FROM benchmarks b
                 LEFT JOIN matchups m ON m.benchmark_id = b.id
                 LEFT JOIN pairs p ON p.matchup_id = m.id
                 LEFT JOIN games g ON g.pair_id = p.id
                 WHERE b.id = ?
                 GROUP BY b.id",
                params![id],
                |row| {
                    Ok(BenchmarkSummary {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        kind: row.get(2)?,
                        status: row.get(3)?,
                        variant: row.get(4)?,
                        requested_pairs: row.get::<_, i64>(5)? as usize,
                        completed_pairs: row.get::<_, i64>(6)? as usize,
                        games: row.get::<_, i64>(7)? as usize,
                    })
                },
            )
            .optional()
            .map_err(|e| format!("load benchmark {id}: {e}"))
    }

    pub fn engine_summaries(&self, benchmark_id: i64) -> Result<Vec<EngineSummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT be.family, eb.name, be.role, COUNT(g.id),
                        COALESCE(SUM(CASE WHEN g.winner_id = eb.id THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE
                            WHEN g.engine_x_id = eb.id THEN g.points_x
                            WHEN g.engine_o_id = eb.id THEN g.points_o
                            ELSE 0 END), 0.0) AS points
                 FROM benchmark_engines be
                 JOIN engine_builds eb ON eb.id = be.engine_build_id
                 LEFT JOIN matchups m
                   ON m.benchmark_id = be.benchmark_id
                  AND (m.engine_a_id = eb.id OR m.engine_b_id = eb.id)
                 LEFT JOIN pairs p ON p.matchup_id = m.id
                 LEFT JOIN games g
                   ON g.pair_id = p.id
                  AND (g.engine_x_id = eb.id OR g.engine_o_id = eb.id)
                 WHERE be.benchmark_id = ?
                 GROUP BY eb.id, be.family, be.role
                 ORDER BY points * 1.0 / MAX(COUNT(g.id), 1) DESC, eb.name",
            )
            .map_err(|e| format!("prepare benchmark standings: {e}"))?;
        let rows = stmt
            .query_map(params![benchmark_id], |row| {
                Ok(EngineSummary {
                    family: row.get(0)?,
                    name: row.get(1)?,
                    role: row.get(2)?,
                    games: row.get::<_, i64>(3)? as usize,
                    wins: row.get::<_, i64>(4)? as usize,
                    points: row.get(5)?,
                })
            })
            .map_err(|e| format!("query benchmark standings: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read benchmark standings: {e}"))
    }
}

fn insert_benchmark(
    tx: &Transaction<'_>,
    name: &str,
    kind: BenchmarkKind,
    variant: &str,
    seed: u64,
    max_plies: usize,
    requested_pairs: usize,
) -> Result<i64, String> {
    if name.trim().is_empty() {
        return Err("benchmark name must not be empty".to_string());
    }
    if requested_pairs == 0 && kind != BenchmarkKind::Ranking {
        return Err("benchmark must request at least one pair".to_string());
    }
    tx.execute(
        "INSERT INTO benchmarks(
            name, kind, status, variant, seed, max_plies, requested_pairs
         ) VALUES (?, ?, 'running', ?, ?, ?, ?)",
        params![
            name.trim(),
            kind.as_str(),
            variant,
            seed.to_string(),
            max_plies as i64,
            requested_pairs as i64
        ],
    )
    .map_err(|e| format!("create benchmark: {e}"))?;
    Ok(tx.last_insert_rowid())
}

fn add_engine(
    tx: &Transaction<'_>,
    benchmark_id: i64,
    role: &str,
    config: &EngineConfig,
) -> Result<i64, String> {
    let identity = engine_identity(config)?;
    let command = serde_json::to_string(&config.command)
        .map_err(|e| format!("serialize engine command: {e}"))?;
    let env = serde_json::to_string(&config.env)
        .map_err(|e| format!("serialize engine environment: {e}"))?;
    let options = serde_json::to_string(&config.options)
        .map_err(|e| format!("serialize engine options: {e}"))?;
    let configuration = serde_json::to_string(&config.configuration)
        .map_err(|e| format!("serialize engine display configuration: {e}"))?;
    tx.execute(
        "INSERT INTO engine_builds(identity, name, command_json, env_json, options_json)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(identity) DO NOTHING",
        params![identity, config.name, command, env, options],
    )
    .map_err(|e| format!("insert engine build: {e}"))?;
    let build_id: i64 = tx
        .query_row(
            "SELECT id FROM engine_builds WHERE identity = ?",
            params![identity],
            |row| row.get(0),
        )
        .map_err(|e| format!("load engine build: {e}"))?;
    tx.execute(
        "INSERT INTO benchmark_engines(
             benchmark_id, engine_build_id, role, family, version, configuration_json
         ) VALUES (?, ?, ?, ?, ?, ?)",
        params![
            benchmark_id,
            build_id,
            role,
            config
                .family
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
            config
                .version
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
            configuration
        ],
    )
    .map_err(|e| format!("add benchmark engine: {e}"))?;
    Ok(build_id)
}

fn engine_identity(config: &EngineConfig) -> Result<String, String> {
    serde_json::to_string(config).map_err(|e| format!("serialize engine {}: {e}", config.name))
}

fn add_matchup(
    tx: &Transaction<'_>,
    benchmark_id: i64,
    engine_a_id: i64,
    engine_b_id: i64,
    pairs: usize,
    seed: u64,
    batch_index: usize,
) -> Result<MatchupHandle, String> {
    tx.execute(
        "INSERT INTO matchups(
            benchmark_id, engine_a_id, engine_b_id, requested_pairs, seed, batch_index
         ) VALUES (?, ?, ?, ?, ?, ?)",
        params![
            benchmark_id,
            engine_a_id,
            engine_b_id,
            pairs as i64,
            seed.to_string(),
            batch_index as i64
        ],
    )
    .map_err(|e| format!("create matchup: {e}"))?;
    Ok(MatchupHandle {
        id: tx.last_insert_rowid(),
        engine_a_id,
        engine_b_id,
        pairs,
        seed,
    })
}

fn validate_games(matchup: MatchupHandle, games: &[GameRecord]) -> Result<(), String> {
    let expected_games = matchup
        .pairs
        .checked_mul(2)
        .ok_or_else(|| "matchup pair count is too large".to_string())?;
    if games.len() != expected_games {
        return Err(format!(
            "matchup {} produced {}/{} games",
            matchup.id,
            games.len(),
            expected_games
        ));
    }
    for (index, game) in games.iter().enumerate() {
        if game.game_idx != index || game.a_is_x != (index % 2 == 0) {
            return Err(format!("invalid mirrored sequence at game {}", index + 1));
        }
        if (game.points_x + game.points_o).abs() > f64::EPSILON
            || (game.points_a + game.points_b).abs() > f64::EPSILON
        {
            return Err(format!("game {} has inconsistent points", index + 1));
        }
        match (game.winner_a, game.outcome.as_ref()) {
            (Some(true), Some(_)) if game.points_a > 0.0 => {}
            (Some(false), Some(_)) if game.points_b > 0.0 => {}
            (None, None) if game.points_a == 0.0 => {}
            _ => return Err(format!("game {} has inconsistent outcome", index + 1)),
        }
    }
    Ok(())
}

fn update_ranking_pair_projection(
    tx: &Transaction<'_>,
    matchup: MatchupHandle,
    games: &[GameRecord],
) -> Result<(), String> {
    let (engine_lo_id, engine_hi_id, lo_is_a) = if matchup.engine_a_id < matchup.engine_b_id {
        (matchup.engine_a_id, matchup.engine_b_id, true)
    } else {
        (matchup.engine_b_id, matchup.engine_a_id, false)
    };
    let mut decisive_games = 0_i64;
    let mut score_sum_lo = 0.0;
    let mut decisions_a = 0_i64;
    let mut decisions_b = 0_i64;
    let mut seconds_a = 0.0;
    let mut seconds_b = 0.0;
    for game in games {
        if game.winner_a.is_some() {
            decisive_games += 1;
            let lo_points = if lo_is_a {
                game.points_a
            } else {
                game.points_b
            };
            score_sum_lo += 0.5 + lo_points / 6.0;
        }
        decisions_a += game.a_decisions as i64;
        decisions_b += game.b_decisions as i64;
        seconds_a += game.a_decision_time.as_secs_f64();
        seconds_b += game.b_decision_time.as_secs_f64();
    }
    let m = decisive_games as f64;

    tx.execute(
        "INSERT INTO ranking_edge_stats(
             benchmark_id, engine_lo_id, engine_hi_id, completed_pairs, rated_games,
             score_sum_lo, sum_m_squared, sum_m_score, sum_score_squared
         )
         SELECT m.benchmark_id, ?, ?, 1, ?, ?, ?, ?, ?
         FROM matchups m
         JOIN benchmarks b ON b.id = m.benchmark_id
         WHERE m.id = ? AND b.kind = 'ranking'
         ON CONFLICT(benchmark_id, engine_lo_id, engine_hi_id) DO UPDATE SET
           completed_pairs = completed_pairs + excluded.completed_pairs,
           rated_games = rated_games + excluded.rated_games,
           score_sum_lo = score_sum_lo + excluded.score_sum_lo,
           sum_m_squared = sum_m_squared + excluded.sum_m_squared,
           sum_m_score = sum_m_score + excluded.sum_m_score,
           sum_score_squared = sum_score_squared + excluded.sum_score_squared",
        params![
            engine_lo_id,
            engine_hi_id,
            decisive_games,
            score_sum_lo,
            m * m,
            m * score_sum_lo,
            score_sum_lo * score_sum_lo,
            matchup.id,
        ],
    )
    .map_err(|e| format!("update ranking edge projection: {e}"))?;

    for (engine_id, decisions, seconds) in [
        (matchup.engine_a_id, decisions_a, seconds_a),
        (matchup.engine_b_id, decisions_b, seconds_b),
    ] {
        tx.execute(
            "INSERT INTO ranking_engine_stats(
                 benchmark_id, engine_id, decision_count, decision_seconds, last_played_batch
             )
             SELECT m.benchmark_id, ?, ?, ?, m.batch_index
             FROM matchups m
             JOIN benchmarks b ON b.id = m.benchmark_id
             WHERE m.id = ? AND b.kind = 'ranking'
             ON CONFLICT(benchmark_id, engine_id) DO UPDATE SET
               decision_count = decision_count + excluded.decision_count,
               decision_seconds = decision_seconds + excluded.decision_seconds,
               last_played_batch = MAX(last_played_batch, excluded.last_played_batch)",
            params![engine_id, decisions, seconds, matchup.id],
        )
        .map_err(|e| format!("update ranking engine projection: {e}"))?;
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| format!("read schema version: {e}"))?;
    if version > SCHEMA_VERSION {
        return Err(format!(
            "benchmark database schema {version} is newer than supported schema {SCHEMA_VERSION}"
        ));
    }
    if version == 0 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE engine_builds (
               id INTEGER PRIMARY KEY,
               identity TEXT NOT NULL UNIQUE,
               name TEXT NOT NULL,
               command_json TEXT NOT NULL,
               env_json TEXT NOT NULL,
               options_json TEXT NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE benchmarks (
               id INTEGER PRIMARY KEY,
               name TEXT NOT NULL,
               kind TEXT NOT NULL CHECK(kind IN ('duel', 'league', 'ranking')),
               status TEXT NOT NULL CHECK(status IN ('running', 'paused', 'completed', 'failed')),
               variant TEXT NOT NULL,
               seed TEXT NOT NULL,
               max_plies INTEGER NOT NULL CHECK(max_plies > 0),
               requested_pairs INTEGER NOT NULL CHECK(requested_pairs >= 0),
               placement_opponents INTEGER,
               placement_pairs INTEGER,
               established_rd REAL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               completed_at TEXT
             );
             CREATE UNIQUE INDEX ranking_name_unique
               ON benchmarks(name COLLATE NOCASE) WHERE kind = 'ranking';
             CREATE TABLE benchmark_engines (
               benchmark_id INTEGER NOT NULL REFERENCES benchmarks(id),
               engine_build_id INTEGER NOT NULL REFERENCES engine_builds(id),
               role TEXT NOT NULL,
               family TEXT,
               version TEXT,
               configuration_json TEXT NOT NULL DEFAULT '{}',
               PRIMARY KEY(benchmark_id, role, engine_build_id)
             );
             CREATE TABLE matchups (
               id INTEGER PRIMARY KEY,
               benchmark_id INTEGER NOT NULL REFERENCES benchmarks(id),
               engine_a_id INTEGER NOT NULL REFERENCES engine_builds(id),
               engine_b_id INTEGER NOT NULL REFERENCES engine_builds(id),
               requested_pairs INTEGER NOT NULL CHECK(requested_pairs > 0),
               seed TEXT NOT NULL,
               batch_index INTEGER NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               UNIQUE(benchmark_id, batch_index)
             );
             CREATE TABLE pairs (
               id INTEGER PRIMARY KEY,
               matchup_id INTEGER NOT NULL REFERENCES matchups(id),
               pair_index INTEGER NOT NULL,
               seed TEXT NOT NULL,
               status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'incomplete')),
               UNIQUE(matchup_id, pair_index)
             );
              CREATE TABLE games (
               id INTEGER PRIMARY KEY,
               pair_id INTEGER NOT NULL REFERENCES pairs(id),
               leg INTEGER NOT NULL CHECK(leg IN (0, 1)),
               game_index INTEGER NOT NULL,
               engine_x_id INTEGER NOT NULL REFERENCES engine_builds(id),
               engine_o_id INTEGER NOT NULL REFERENCES engine_builds(id),
               winner_id INTEGER REFERENCES engine_builds(id),
               outcome TEXT CHECK(outcome IN ('normal', 'gammon', 'backgammon', 'unknown')),
               points_x REAL NOT NULL,
               points_o REAL NOT NULL,
               points_a REAL NOT NULL,
               points_b REAL NOT NULL,
               plies INTEGER NOT NULL,
               decisions_a INTEGER NOT NULL CHECK(decisions_a >= 0),
               decisions_b INTEGER NOT NULL CHECK(decisions_b >= 0),
               decision_seconds_a REAL NOT NULL CHECK(decision_seconds_a >= 0),
               decision_seconds_b REAL NOT NULL CHECK(decision_seconds_b >= 0),
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               UNIQUE(pair_id, leg),
                UNIQUE(pair_id, game_index)
              );
              CREATE TABLE ranking_edge_stats (
                benchmark_id INTEGER NOT NULL REFERENCES benchmarks(id),
                engine_lo_id INTEGER NOT NULL REFERENCES engine_builds(id),
                engine_hi_id INTEGER NOT NULL REFERENCES engine_builds(id),
                completed_pairs INTEGER NOT NULL CHECK(completed_pairs >= 0),
                rated_games INTEGER NOT NULL CHECK(rated_games >= 0),
                score_sum_lo REAL NOT NULL,
                sum_m_squared REAL NOT NULL,
                sum_m_score REAL NOT NULL,
                sum_score_squared REAL NOT NULL,
                PRIMARY KEY(benchmark_id, engine_lo_id, engine_hi_id),
                CHECK(engine_lo_id < engine_hi_id)
              );
              CREATE TABLE ranking_engine_stats (
                benchmark_id INTEGER NOT NULL REFERENCES benchmarks(id),
                engine_id INTEGER NOT NULL REFERENCES engine_builds(id),
                decision_count INTEGER NOT NULL CHECK(decision_count >= 0),
                decision_seconds REAL NOT NULL CHECK(decision_seconds >= 0),
                last_played_batch INTEGER,
                PRIMARY KEY(benchmark_id, engine_id)
              );
               PRAGMA user_version = 3;
              COMMIT;",
        )
        .map_err(|e| format!("apply benchmark schema v3: {e}"))?;
    } else if version == 1 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin benchmark schema v2 migration: {e}"))?;
        tx.execute_batch(
            "CREATE TABLE ranking_edge_stats (
               benchmark_id INTEGER NOT NULL REFERENCES benchmarks(id),
               engine_lo_id INTEGER NOT NULL REFERENCES engine_builds(id),
               engine_hi_id INTEGER NOT NULL REFERENCES engine_builds(id),
               completed_pairs INTEGER NOT NULL CHECK(completed_pairs >= 0),
               rated_games INTEGER NOT NULL CHECK(rated_games >= 0),
               score_sum_lo REAL NOT NULL,
               sum_m_squared REAL NOT NULL,
               sum_m_score REAL NOT NULL,
               sum_score_squared REAL NOT NULL,
               PRIMARY KEY(benchmark_id, engine_lo_id, engine_hi_id),
               CHECK(engine_lo_id < engine_hi_id)
             );
             CREATE TABLE ranking_engine_stats (
               benchmark_id INTEGER NOT NULL REFERENCES benchmarks(id),
               engine_id INTEGER NOT NULL REFERENCES engine_builds(id),
               decision_count INTEGER NOT NULL CHECK(decision_count >= 0),
               decision_seconds REAL NOT NULL CHECK(decision_seconds >= 0),
               last_played_batch INTEGER,
               PRIMARY KEY(benchmark_id, engine_id)
             );
             ALTER TABLE benchmark_engines ADD COLUMN version TEXT;
             ALTER TABLE benchmark_engines ADD COLUMN configuration_json TEXT NOT NULL DEFAULT '{}';",
        )
        .map_err(|e| format!("create benchmark schema v2 projections: {e}"))?;
        rebuild_ranking_projections(&tx)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| format!("set benchmark schema version: {e}"))?;
        tx.commit()
            .map_err(|e| format!("commit benchmark schema v2 migration: {e}"))?;
    } else if version == 2 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin benchmark schema v3 migration: {e}"))?;
        tx.execute_batch(
            "ALTER TABLE benchmark_engines ADD COLUMN version TEXT;
             ALTER TABLE benchmark_engines ADD COLUMN configuration_json TEXT NOT NULL DEFAULT '{}';",
        )
        .map_err(|e| format!("add benchmark engine metadata: {e}"))?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| format!("set benchmark schema version: {e}"))?;
        tx.commit()
            .map_err(|e| format!("commit benchmark schema v3 migration: {e}"))?;
    }
    Ok(())
}

fn rebuild_ranking_projections(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute_batch(
        "DELETE FROM ranking_edge_stats;
         DELETE FROM ranking_engine_stats;
         WITH pair_stats AS (
           SELECT m.benchmark_id,
                  MIN(m.engine_a_id, m.engine_b_id) AS engine_lo_id,
                  MAX(m.engine_a_id, m.engine_b_id) AS engine_hi_id,
                  p.id AS pair_id,
                  SUM(CASE WHEN g.winner_id IS NOT NULL THEN 1 ELSE 0 END) AS decisive_games,
                  SUM(CASE WHEN g.winner_id IS NOT NULL THEN
                    0.5 + CASE WHEN m.engine_a_id < m.engine_b_id
                      THEN g.points_a ELSE g.points_b END / 6.0
                    ELSE 0.0 END) AS score_sum_lo
           FROM benchmarks b
           JOIN matchups m ON m.benchmark_id = b.id
           JOIN pairs p ON p.matchup_id = m.id AND p.status = 'completed'
           LEFT JOIN games g ON g.pair_id = p.id
           WHERE b.kind = 'ranking'
           GROUP BY m.benchmark_id, engine_lo_id, engine_hi_id, p.id
         )
         INSERT INTO ranking_edge_stats(
           benchmark_id, engine_lo_id, engine_hi_id, completed_pairs, rated_games,
           score_sum_lo, sum_m_squared, sum_m_score, sum_score_squared
         )
         SELECT benchmark_id, engine_lo_id, engine_hi_id, COUNT(*),
                SUM(decisive_games), SUM(score_sum_lo),
                SUM(decisive_games * decisive_games),
                SUM(decisive_games * score_sum_lo),
                SUM(score_sum_lo * score_sum_lo)
         FROM pair_stats
         GROUP BY benchmark_id, engine_lo_id, engine_hi_id;

         WITH engine_games AS (
           SELECT m.benchmark_id, m.engine_a_id AS engine_id,
                  g.decisions_a AS decisions, g.decision_seconds_a AS seconds,
                  m.batch_index
           FROM benchmarks b
           JOIN matchups m ON m.benchmark_id = b.id
           JOIN pairs p ON p.matchup_id = m.id AND p.status = 'completed'
           JOIN games g ON g.pair_id = p.id
           WHERE b.kind = 'ranking'
           UNION ALL
           SELECT m.benchmark_id, m.engine_b_id,
                  g.decisions_b, g.decision_seconds_b, m.batch_index
           FROM benchmarks b
           JOIN matchups m ON m.benchmark_id = b.id
           JOIN pairs p ON p.matchup_id = m.id AND p.status = 'completed'
           JOIN games g ON g.pair_id = p.id
           WHERE b.kind = 'ranking'
         )
         INSERT INTO ranking_engine_stats(
           benchmark_id, engine_id, decision_count, decision_seconds, last_played_batch
         )
         SELECT benchmark_id, engine_id, SUM(decisions), SUM(seconds), MAX(batch_index)
         FROM engine_games
         GROUP BY benchmark_id, engine_id;",
    )
    .map_err(|e| format!("rebuild ranking projections: {e}"))
}

pub fn default_benchmark_db_path() -> PathBuf {
    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg_data_home)
            .join("bgci")
            .join("benchmarks.db");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("bgci")
            .join("benchmarks.db");
    }
    PathBuf::from("data/benchmarks.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(name: &str) -> EngineConfig {
        EngineConfig {
            name: name.to_string(),
            family: None,
            version: None,
            configuration: BTreeMap::new(),
            engine: Some(name.to_string()),
            command: vec![name.to_string()],
            env: Default::default(),
            options: Default::default(),
        }
    }

    fn spec(pairs: usize) -> BenchmarkSpec<'static> {
        BenchmarkSpec {
            name: "benchmark",
            variant: "backgammon",
            seed: 42,
            max_plies: 512,
            pairs,
        }
    }

    fn game(game_idx: usize, a_is_x: bool) -> GameRecord {
        GameRecord {
            game_idx,
            a_is_x,
            winner_a: Some(true),
            outcome: Some(crate::duel_runner::GameOutcome::Normal),
            points_x: if a_is_x { 1.0 } else { -1.0 },
            points_o: if a_is_x { -1.0 } else { 1.0 },
            points_a: 1.0,
            points_b: -1.0,
            plies: 10,
            a_decisions: 5,
            b_decisions: 5,
            a_decision_time: Duration::from_millis(50),
            b_decision_time: Duration::from_millis(100),
        }
    }

    #[test]
    fn stores_a_complete_mirror_pair() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let started = store
            .start_duel(spec(1), &config("a"), &config("b"))
            .unwrap();
        store
            .record_games(started.matchups[0], &[game(0, true), game(1, false)])
            .unwrap();
        store.finish_benchmark(started.id).unwrap();

        let summary = store.get(started.id).unwrap().unwrap();
        assert_eq!(summary.completed_pairs, 1);
        assert_eq!(summary.games, 2);
        assert_eq!(summary.status, "completed");
        let summaries = store.engine_summaries(started.id).unwrap();
        assert_eq!(summaries[0].name, "a");
        assert_eq!(summaries[0].wins, 2);
        assert_eq!(summaries[0].points, 2.0);
    }

    #[test]
    fn rejects_duplicate_pair_ingestion() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let started = store
            .start_duel(spec(1), &config("a"), &config("b"))
            .unwrap();
        store
            .record_games(started.matchups[0], &[game(0, true), game(1, false)])
            .unwrap();

        assert!(
            store
                .record_games(started.matchups[0], &[game(0, true), game(1, false)])
                .is_err()
        );
    }

    #[test]
    fn refuses_to_complete_without_all_pairs() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let started = store
            .start_duel(spec(2), &config("a"), &config("b"))
            .unwrap();

        assert!(store.finish_benchmark(started.id).is_err());
    }

    #[test]
    fn rejects_saved_self_play_without_creating_a_benchmark() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let result = store.start_duel(spec(1), &config("a"), &config("a"));

        assert!(result.is_err());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn league_matchups_have_distinct_deterministic_seeds() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let engines = [config("a"), config("b"), config("c")];
        let started = store.start_league(spec(1), &engines).unwrap();

        assert_eq!(started.matchups.len(), 3);
        assert_ne!(started.matchups[0].seed(), started.matchups[1].seed());
        assert_ne!(started.matchups[1].seed(), started.matchups[2].seed());
    }

    #[test]
    fn benchmark_start_is_atomic() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let mut invalid = spec(1);
        invalid.name = "  ";

        assert!(
            store
                .start_duel(invalid, &config("a"), &config("b"))
                .is_err()
        );
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn rejects_non_mirrored_results() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let started = store
            .start_duel(spec(1), &config("a"), &config("b"))
            .unwrap();
        let mut first = game(0, true);
        first.a_is_x = false;

        assert!(
            store
                .record_games(started.matchups[0], &[first, game(1, false)])
                .is_err()
        );
    }

    #[test]
    fn family_is_metadata_not_launch_identity() {
        let first = config("engine");
        let mut second = first.clone();
        second.family = Some("new-family".to_string());

        assert_eq!(
            engine_identity(&first).unwrap(),
            engine_identity(&second).unwrap()
        );
    }

    #[test]
    fn stores_family_with_benchmark_participant() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let mut a = config("a");
        a.family = Some("kestral".to_string());
        let started = store.start_duel(spec(1), &a, &config("b")).unwrap();
        store
            .record_games(started.matchups[0], &[game(0, true), game(1, false)])
            .unwrap();
        let summaries = store.engine_summaries(started.id).unwrap();

        assert_eq!(summaries[0].family.as_deref(), Some("kestral"));
    }

    #[test]
    fn ranking_pool_persists_batches_and_resumes() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let mut pool = store
            .start_ranking(
                RankingSpec {
                    name: "ranking",
                    variant: "backgammon",
                    seed: 42,
                    max_plies: 512,
                    placement_opponents: 1,
                    placement_pairs: 1,
                    established_rd: 80.0,
                },
                &[config("a"), config("b")],
            )
            .unwrap();
        store.resume_ranking(pool.id).unwrap();
        pool = store.load_ranking(pool.id).unwrap();
        let matchup = store.start_ranking_batch(&pool, 0, 1, 1).unwrap();
        store
            .record_games(matchup, &[game(0, true), game(1, false)])
            .unwrap();
        pool.next_batch += 1;
        let data = store.ranking_data(&pool).unwrap();

        assert_eq!(data.edges.len(), 1);
        assert_eq!(data.edges[0].rated_games, 2);
        assert!((data.edges[0].score_sum_a - 4.0 / 3.0).abs() < 1e-12);
        assert_eq!(data.pair_counts[0][1], 1);
        assert_eq!(
            data.average_decision_time,
            vec![
                Some(Duration::from_millis(10)),
                Some(Duration::from_millis(20))
            ]
        );
        assert_eq!(data.last_played_batch, vec![Some(0), Some(0)]);
        store.pause_ranking(pool.id).unwrap();
        assert_eq!(store.load_ranking(pool.id).unwrap().status, "paused");
        store.resume_ranking(pool.id).unwrap();
        let resumed = store.load_ranking(pool.id).unwrap();
        assert_eq!(resumed.status, "running");
        assert_eq!(resumed.next_batch, 1);
        store.pause_ranking(pool.id).unwrap();
        let expanded = store
            .add_ranking_engines("ranking", &[config("c")])
            .unwrap();
        assert_eq!(expanded.engines.len(), 3);
    }

    #[test]
    fn ranking_metadata_can_be_refreshed_without_changing_identity() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let pool = ranking_pool(&mut store);
        store.pause_ranking(pool.id).unwrap();
        let mut a = config("a");
        a.family = Some("family-a".to_string());
        a.version = Some("v2".to_string());
        a.configuration
            .insert("model".to_string(), "large".to_string());

        let refreshed = store
            .refresh_ranking_engine_metadata(&pool.name, &[a, config("b")])
            .unwrap();
        let a = refreshed
            .engines
            .iter()
            .find(|engine| engine.name == "a")
            .unwrap();

        assert_eq!(a.family.as_deref(), Some("family-a"));
        assert_eq!(a.version.as_deref(), Some("v2"));
        assert_eq!(a.configuration["model"], "large");
    }

    #[test]
    fn empty_ranking_batch_can_be_retried() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let mut pool = store
            .start_ranking(
                RankingSpec {
                    name: "ranking",
                    variant: "backgammon",
                    seed: 42,
                    max_plies: 512,
                    placement_opponents: 1,
                    placement_pairs: 1,
                    established_rd: 80.0,
                },
                &[config("a"), config("b")],
            )
            .unwrap();
        store.resume_ranking(pool.id).unwrap();
        pool = store.load_ranking(pool.id).unwrap();
        let failed = store.start_ranking_batch(&pool, 0, 1, 1).unwrap();

        store.discard_empty_matchup(failed).unwrap();

        let retry_pool = store.load_ranking(pool.id).unwrap();
        assert_eq!(retry_pool.next_batch, 0);
        assert!(store.start_ranking_batch(&retry_pool, 0, 1, 1).is_ok());
    }

    fn ranking_pool(store: &mut BenchmarkStore) -> RankingPool {
        let pool = store
            .start_ranking(
                RankingSpec {
                    name: "projection-ranking",
                    variant: "backgammon",
                    seed: 42,
                    max_plies: 512,
                    placement_opponents: 1,
                    placement_pairs: 1,
                    established_rd: 80.0,
                },
                &[config("a"), config("b")],
            )
            .unwrap();
        store.resume_ranking(pool.id).unwrap();
        store.load_ranking(pool.id).unwrap()
    }

    #[test]
    fn ranking_projections_are_canonical_and_pair_robust() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let pool = ranking_pool(&mut store);
        let matchup = store.start_ranking_batch(&pool, 1, 0, 2).unwrap();
        store
            .record_games(
                matchup,
                &[game(0, true), game(1, false), game(2, true), game(3, false)],
            )
            .unwrap();

        let data = store.ranking_data(&pool).unwrap();
        let edge = &data.edges[0];
        assert_eq!((edge.engine_a, edge.engine_b), (0, 1));
        assert_eq!(edge.completed_pairs, 2);
        assert_eq!(edge.rated_games, 4);
        assert!((edge.score_sum_a - 4.0 / 3.0).abs() < 1e-12);
        assert!((edge.sum_m_squared - 8.0).abs() < 1e-12);
        assert!((edge.sum_m_score - 8.0 / 3.0).abs() < 1e-12);
        assert!((edge.sum_score_squared - 8.0 / 9.0).abs() < 1e-12);
        assert_eq!(data.pair_counts, vec![vec![0, 2], vec![2, 0]]);
        assert_eq!(
            data.average_decision_time,
            vec![
                Some(Duration::from_millis(20)),
                Some(Duration::from_millis(10))
            ]
        );
        assert_eq!(data.last_played_batch, vec![Some(0), Some(0)]);
    }

    #[test]
    fn projection_failure_rolls_back_raw_results() {
        let mut store =
            BenchmarkStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let pool = ranking_pool(&mut store);
        let matchup = store.start_ranking_batch(&pool, 0, 1, 1).unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER reject_ranking_projection
                 BEFORE INSERT ON ranking_edge_stats
                 BEGIN SELECT RAISE(ABORT, 'projection rejected'); END;",
            )
            .unwrap();

        assert!(
            store
                .record_games(matchup, &[game(0, true), game(1, false)])
                .is_err()
        );
        let pair_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM pairs", [], |row| row.get(0))
            .unwrap();
        let game_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM games", [], |row| row.get(0))
            .unwrap();
        assert_eq!((pair_count, game_count), (0, 0));
    }

    #[test]
    fn schema_v1_migration_backfills_ranking_projections() {
        let conn = Connection::open_in_memory().unwrap();
        let mut store = BenchmarkStore::from_connection(conn).unwrap();
        let pool = ranking_pool(&mut store);
        let matchup = store.start_ranking_batch(&pool, 1, 0, 1).unwrap();
        store
            .record_games(matchup, &[game(0, true), game(1, false)])
            .unwrap();
        store
            .conn
            .execute_batch(
                "DROP TABLE ranking_edge_stats;
                  DROP TABLE ranking_engine_stats;
                  ALTER TABLE benchmark_engines DROP COLUMN configuration_json;
                  ALTER TABLE benchmark_engines DROP COLUMN version;
                  PRAGMA user_version = 1;",
            )
            .unwrap();

        let store = BenchmarkStore::from_connection(store.conn).unwrap();
        let data = store.ranking_data(&pool).unwrap();
        assert_eq!(data.edges.len(), 1);
        assert_eq!(data.edges[0].completed_pairs, 1);
        assert_eq!(data.edges[0].rated_games, 2);
        assert!((data.edges[0].score_sum_a - 2.0 / 3.0).abs() < 1e-12);
        assert!((data.edges[0].sum_m_squared - 4.0).abs() < 1e-12);
        assert_eq!(
            data.average_decision_time,
            vec![
                Some(Duration::from_millis(20)),
                Some(Duration::from_millis(10))
            ]
        );
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }
}
