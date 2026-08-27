use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::config::{EngineLaunch, EngineMetadata, ResolvedEngine};
use crate::duel_game::seed_for_game;
use crate::duel_runner::GameRecord;
use crate::ranking::RankingEdge;

const SCHEMA_VERSION: i64 = 2;
const BENCHMARK_SUMMARY_PROJECTION: &str =
    "SELECT b.id, b.name, b.kind, b.status, b.variant, b.requested_games,
            COUNT(g.id)
     FROM benchmarks b
     LEFT JOIN matchups m ON m.benchmark_id = b.id
     LEFT JOIN games g ON g.matchup_id = m.id";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BenchmarkKind {
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
    pub requested_games: usize,
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
    games: usize,
    seed: u64,
}

impl MatchupHandle {
    pub fn seed(self) -> u64 {
        self.seed
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScheduledMatchup {
    pub handle: MatchupHandle,
    pub engine_a: usize,
    pub engine_b: usize,
}

pub struct StartedBenchmark {
    pub id: i64,
    pub matchups: Vec<ScheduledMatchup>,
}

pub struct BenchmarkSpec<'a> {
    pub name: &'a str,
    pub variant: &'a str,
    pub seed: u64,
    pub max_plies: usize,
    pub games: usize,
}

pub struct RankingSpec<'a> {
    pub name: &'a str,
    pub variant: &'a str,
    pub seed: u64,
    pub max_plies: usize,
    pub placement_opponents: usize,
    pub placement_games: usize,
    pub established_rd: f64,
}

#[derive(Clone, Debug)]
pub struct RankingEngine {
    build_id: i64,
    pub config: ResolvedEngine,
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
    pub placement_games: usize,
    pub established_rd: f64,
    pub engines: Vec<RankingEngine>,
}

pub struct RankingData {
    pub edges: Vec<RankingEdge>,
    pub game_counts: Vec<Vec<usize>>,
    pub average_decision_time: Vec<Option<Duration>>,
    pub last_played_batch: Vec<Option<usize>>,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create database directory {}: {e}", parent.display()))?;
        }
        let conn =
            Connection::open(path).map_err(|e| format!("open database {}: {e}", path.display()))?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(|e| format!("configure database: {e}"))?;
        initialize_schema(&conn)?;
        Ok(Self { conn })
    }

    pub fn start_duel(
        &mut self,
        spec: BenchmarkSpec<'_>,
        engine_a: &ResolvedEngine,
        engine_b: &ResolvedEngine,
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
            spec.games,
        )?;
        let engine_a_id = add_engine(&tx, benchmark_id, "engine-a", engine_a)?;
        let engine_b_id = add_engine(&tx, benchmark_id, "engine-b", engine_b)?;
        let matchup = add_matchup(
            &tx,
            benchmark_id,
            engine_a_id,
            engine_b_id,
            spec.games,
            spec.seed,
            0,
        )?;
        tx.commit()
            .map_err(|e| format!("commit duel transaction: {e}"))?;
        Ok(StartedBenchmark {
            id: benchmark_id,
            matchups: vec![ScheduledMatchup {
                handle: matchup,
                engine_a: 0,
                engine_b: 1,
            }],
        })
    }

    pub fn start_league(
        &mut self,
        spec: BenchmarkSpec<'_>,
        engines: &[ResolvedEngine],
    ) -> Result<StartedBenchmark, String> {
        if engines.len() < 2 {
            return Err("league requires at least two engines".to_string());
        }
        let mut identities = HashSet::new();
        for engine in engines {
            if !identities.insert(engine_identity(engine)?) {
                return Err(format!("duplicate resolved engine: {}", engine.name));
            }
        }

        let matchup_count = engines.len() * (engines.len() - 1) / 2;
        let requested_games = spec
            .games
            .checked_mul(matchup_count)
            .ok_or_else(|| "league game count is too large".to_string())?;
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
            requested_games,
        )?;
        let mut engine_ids = Vec::with_capacity(engines.len());
        for engine in engines {
            engine_ids.push(add_engine(&tx, benchmark_id, "member", engine)?);
        }
        let mut matchups = Vec::with_capacity(matchup_count);
        for a in 0..engines.len() {
            for b in (a + 1)..engines.len() {
                let matchup_seed = seed_for_game(spec.seed, matchups.len());
                let handle = add_matchup(
                    &tx,
                    benchmark_id,
                    engine_ids[a],
                    engine_ids[b],
                    spec.games,
                    matchup_seed,
                    matchups.len(),
                )?;
                matchups.push(ScheduledMatchup {
                    handle,
                    engine_a: a,
                    engine_b: b,
                });
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
        engines: &[ResolvedEngine],
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
                  placement_opponents = ?, placement_games = ?, established_rd = ?
             WHERE id = ?",
            params![
                spec.placement_opponents as i64,
                spec.placement_games as i64,
                spec.established_rd,
                benchmark_id
            ],
        )
        .map_err(|e| format!("configure new ranking: {e}"))?;
        tx.commit()
            .map_err(|e| format!("commit ranking transaction: {e}"))?;
        self.load_ranking(benchmark_id)
    }

    fn load_ranking(&self, benchmark_id: i64) -> Result<RankingPool, String> {
        let (
            name,
            status,
            variant,
            seed,
            max_plies,
            placement_opponents,
            placement_games,
            established_rd,
        ): (String, String, String, String, i64, i64, i64, f64) = self
            .conn
            .query_row(
                "SELECT name, status, variant, seed, max_plies,
                        placement_opponents, placement_games, established_rd
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
                "SELECT eb.id, be.name, be.family, be.version, be.labels_json,
                         eb.command_json, eb.env_json, eb.ubgi_json
                 FROM benchmark_engines be
                 JOIN engine_builds eb ON eb.id = be.engine_build_id
                 WHERE be.benchmark_id = ?
                 ORDER BY be.name",
            )
            .map_err(|e| format!("prepare ranking engines: {e}"))?;
        let rows = stmt
            .query_map(params![benchmark_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(|e| format!("query ranking engines: {e}"))?;
        let mut engines = Vec::new();
        for row in rows {
            let (build_id, name, family, version, labels, command, env, ubgi) =
                row.map_err(|e| format!("read ranking engine: {e}"))?;
            let labels: BTreeMap<String, String> =
                serde_json::from_str(&labels).map_err(|e| format!("parse engine labels: {e}"))?;
            engines.push(RankingEngine {
                build_id,
                config: ResolvedEngine {
                    name,
                    launch: EngineLaunch::new(
                        serde_json::from_str(&command)
                            .map_err(|e| format!("parse engine command: {e}"))?,
                        serde_json::from_str(&env)
                            .map_err(|e| format!("parse engine environment: {e}"))?,
                        serde_json::from_str(&ubgi)
                            .map_err(|e| format!("parse engine UBGI settings: {e}"))?,
                    )?,
                    metadata: EngineMetadata {
                        family,
                        version,
                        labels,
                    },
                },
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
            placement_games: placement_games as usize,
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
        engines: &[ResolvedEngine],
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
            .map(|engine| engine_identity(&engine.config))
            .collect::<Result<HashSet<_>, _>>()?;
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
        engines: &[ResolvedEngine],
        apply_ubgi: bool,
    ) -> Result<RankingPool, String> {
        let pool = self.load_ranking_by_name(name)?;
        if pool.status != "paused" {
            return Err(format!(
                "ranking '{}' must be paused before refreshing metadata",
                pool.name
            ));
        }
        let configs_by_identity = engines
            .iter()
            .map(|engine| Ok((engine_identity(engine)?, engine)))
            .collect::<Result<HashMap<_, _>, String>>()?;
        let configs_by_name = engines
            .iter()
            .map(|engine| (engine.name.as_str(), engine))
            .collect::<HashMap<_, _>>();
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin refresh ranking metadata transaction: {e}"))?;
        for participant in &pool.engines {
            let config = if apply_ubgi {
                let config = configs_by_name
                    .get(participant.config.name.as_str())
                    .ok_or_else(|| {
                        format!(
                            "no current profile found for engine {}",
                            participant.config.name
                        )
                    })?;
                if config.launch.command() != participant.config.launch.command()
                    || config.launch.env() != participant.config.launch.env()
                {
                    return Err(format!(
                        "refusing to change command or environment for existing engine {}",
                        participant.config.name
                    ));
                }
                if participant
                    .config
                    .launch
                    .ubgi()
                    .iter()
                    .any(|(key, value)| config.launch.ubgi().get(key) != Some(value))
                {
                    return Err(format!(
                        "refusing to change an existing UBGI option for engine {}",
                        participant.config.name
                    ));
                }
                *config
            } else {
                let identity = engine_identity(&participant.config)?;
                *configs_by_identity.get(&identity).ok_or_else(|| {
                    format!(
                        "no current metadata found for engine {}",
                        participant.config.name
                    )
                })?
            };
            let labels = serde_json::to_string(&config.metadata.labels)
                .map_err(|e| format!("serialize engine labels: {e}"))?;
            let build_id = if apply_ubgi {
                let build_id = ensure_engine_build(&tx, config)?;
                if build_id != participant.build_id {
                    replace_ranking_build(&tx, pool.id, participant.build_id, build_id)?;
                }
                build_id
            } else {
                participant.build_id
            };
            tx.execute(
                "UPDATE benchmark_engines
                 SET name = ?, family = ?, version = ?, labels_json = ?
                 WHERE benchmark_id = ? AND engine_build_id = ? AND role = 'member'",
                params![
                    config.name,
                    config.metadata.family.as_deref(),
                    config.metadata.version.as_deref(),
                    labels,
                    pool.id,
                    build_id,
                ],
            )
            .map_err(|e| format!("refresh metadata for {}: {e}", participant.config.name))?;
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
        games: usize,
    ) -> Result<MatchupHandle, String> {
        if pool.status != "running" {
            return Err(format!("ranking {} is not running", pool.id));
        }
        if engine_a >= pool.engines.len() || engine_b >= pool.engines.len() || engine_a == engine_b
        {
            return Err("invalid ranking matchup engines".to_string());
        }
        if games == 0 {
            return Err("ranking batch must contain at least one game".to_string());
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
            games,
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
                    SELECT 1 FROM games WHERE matchup_id = matchups.id
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
        let mut game_counts = vec![vec![0; pool.engines.len()]; pool.engines.len()];
        let mut edge_stmt = self
            .conn
            .prepare(
                "WITH pair_stats AS (
                   SELECT MIN(m.engine_a_id, m.engine_b_id) AS engine_lo_id,
                          MAX(m.engine_a_id, m.engine_b_id) AS engine_hi_id,
                           m.id AS matchup_id, g.pair_index,
                           COUNT(*) AS physical_games,
                           SUM(g.points_a != 0) AS decisive_games,
                           SUM(CASE WHEN g.points_a != 0 THEN
                             0.5 + CASE WHEN m.engine_a_id < m.engine_b_id
                               THEN g.points_a ELSE -g.points_a END / 6.0
                             ELSE 0.0 END) AS score_sum_lo
                   FROM matchups m
                   JOIN games g ON g.matchup_id = m.id
                   WHERE m.benchmark_id = ?
                   GROUP BY engine_lo_id, engine_hi_id, m.id, g.pair_index
                 )
                 SELECT engine_lo_id, engine_hi_id, SUM(decisive_games > 0), SUM(physical_games),
                        SUM(decisive_games),
                         SUM(score_sum_lo), SUM(decisive_games * decisive_games),
                        SUM(decisive_games * score_sum_lo),
                        SUM(score_sum_lo * score_sum_lo)
                 FROM pair_stats
                 GROUP BY engine_lo_id, engine_hi_id
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
                    row.get::<_, i64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                    row.get::<_, f64>(7)?,
                    row.get::<_, f64>(8)?,
                ))
            })
            .map_err(|e| format!("query ranking edges: {e}"))?;
        let mut edges = Vec::new();
        for row in edge_rows {
            let (lo_id, hi_id, clusters, physical_games, rated_games, score, m2, m_score, score2) =
                row.map_err(|e| format!("read ranking edge: {e}"))?;
            if let (Some(&lo), Some(&hi)) = (index_by_build.get(&lo_id), index_by_build.get(&hi_id))
            {
                game_counts[lo][hi] = physical_games as usize;
                game_counts[hi][lo] = physical_games as usize;
                edges.push(RankingEdge {
                    engine_a: lo,
                    engine_b: hi,
                    completed_clusters: clusters as usize,
                    rated_games: rated_games as usize,
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
                "WITH engine_games AS (
                   SELECT m.engine_a_id AS engine_id, g.decisions_a AS decisions,
                          g.decision_seconds_a AS seconds, m.batch_index
                   FROM matchups m JOIN games g ON g.matchup_id = m.id
                   WHERE m.benchmark_id = ?
                   UNION ALL
                   SELECT m.engine_b_id, g.decisions_b, g.decision_seconds_b, m.batch_index
                   FROM matchups m JOIN games g ON g.matchup_id = m.id
                   WHERE m.benchmark_id = ?
                 )
                 SELECT engine_id, SUM(decisions), SUM(seconds), MAX(batch_index)
                 FROM engine_games GROUP BY engine_id",
            )
            .map_err(|e| format!("prepare ranking engine stats: {e}"))?;
        let engine_rows = engine_stmt
            .query_map(params![pool.id, pool.id], |row| {
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
            game_counts,
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
        for game in games {
            let (decisions_a, decisions_b, decision_seconds_a, decision_seconds_b) = (
                game.a_decisions,
                game.b_decisions,
                game.a_decision_time.as_secs_f64(),
                game.b_decision_time.as_secs_f64(),
            );
            tx.execute(
                "INSERT INTO games(
                        matchup_id, pair_index, leg, points_a, plies,
                        decisions_a, decisions_b, decision_seconds_a, decision_seconds_b
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    matchup.id,
                    game.pair_index as i64,
                    game.leg as i64,
                    game.points_a,
                    game.plies as i64,
                    decisions_a as i64,
                    decisions_b as i64,
                    decision_seconds_a,
                    decision_seconds_b
                ],
            )
            .map_err(|e| format!("insert game {}: {e}", game.game_idx))?;
        }
        tx.commit().map_err(|e| format!("commit results: {e}"))
    }

    pub fn finish_benchmark(&self, benchmark_id: i64) -> Result<(), String> {
        let changed = self
            .conn
            .execute(
                "UPDATE benchmarks SET status = 'completed', completed_at = CURRENT_TIMESTAMP
                 WHERE id = ? AND status = 'running'
                     AND requested_games = (
                      SELECT COUNT(*) FROM games g
                      JOIN matchups m ON m.id = g.matchup_id
                      WHERE m.benchmark_id = benchmarks.id
                    )",
                params![benchmark_id],
            )
            .map_err(|e| format!("finish benchmark: {e}"))?;
        if changed == 0 {
            return Err(format!(
                "benchmark {benchmark_id} cannot complete before all requested games are stored"
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
        let query = format!(
            "{BENCHMARK_SUMMARY_PROJECTION}
             GROUP BY b.id
             ORDER BY b.id DESC"
        );
        let mut stmt = self
            .conn
            .prepare(&query)
            .map_err(|e| format!("prepare benchmark list: {e}"))?;
        let rows = stmt
            .query_map([], decode_benchmark_summary)
            .map_err(|e| format!("query benchmark list: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read benchmark list: {e}"))
    }

    pub fn list_rankings(&self) -> Result<Vec<BenchmarkSummary>, String> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|summary| summary.kind == BenchmarkKind::Ranking.as_str())
            .collect())
    }

    pub fn get(&self, id: i64) -> Result<Option<BenchmarkSummary>, String> {
        let query = format!(
            "{BENCHMARK_SUMMARY_PROJECTION}
             WHERE b.id = ?
             GROUP BY b.id"
        );
        self.conn
            .query_row(&query, params![id], decode_benchmark_summary)
            .optional()
            .map_err(|e| format!("load benchmark {id}: {e}"))
    }

    pub fn engine_summaries(&self, benchmark_id: i64) -> Result<Vec<EngineSummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT be.family, be.name, be.role, COUNT(g.id),
                         COALESCE(SUM(CASE
                           WHEN m.engine_a_id = eb.id AND g.points_a > 0 THEN 1
                           WHEN m.engine_b_id = eb.id AND g.points_a < 0 THEN 1
                           ELSE 0 END), 0),
                         COALESCE(SUM(CASE
                            WHEN m.engine_a_id = eb.id THEN g.points_a
                            WHEN m.engine_b_id = eb.id THEN -g.points_a
                            ELSE 0 END), 0.0) AS points
                 FROM benchmark_engines be
                 JOIN engine_builds eb ON eb.id = be.engine_build_id
                 LEFT JOIN matchups m
                   ON m.benchmark_id = be.benchmark_id
                  AND (m.engine_a_id = eb.id OR m.engine_b_id = eb.id)
                   LEFT JOIN games g
                     ON g.matchup_id = m.id
                 WHERE be.benchmark_id = ?
                 GROUP BY eb.id, be.family, be.role
                  ORDER BY points * 1.0 / MAX(COUNT(g.id), 1) DESC, be.name",
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

fn decode_benchmark_summary(row: &Row<'_>) -> rusqlite::Result<BenchmarkSummary> {
    Ok(BenchmarkSummary {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        status: row.get(3)?,
        variant: row.get(4)?,
        requested_games: row.get::<_, i64>(5)? as usize,
        games: row.get::<_, i64>(6)? as usize,
    })
}

fn insert_benchmark(
    tx: &Transaction<'_>,
    name: &str,
    kind: BenchmarkKind,
    variant: &str,
    seed: u64,
    max_plies: usize,
    requested_games: usize,
) -> Result<i64, String> {
    if name.trim().is_empty() {
        return Err("benchmark name must not be empty".to_string());
    }
    if requested_games == 0 && kind != BenchmarkKind::Ranking {
        return Err("benchmark must request at least one game".to_string());
    }
    tx.execute(
        "INSERT INTO benchmarks(
            name, kind, status, variant, seed, max_plies, requested_games
         ) VALUES (?, ?, 'running', ?, ?, ?, ?)",
        params![
            name.trim(),
            kind.as_str(),
            variant,
            seed.to_string(),
            max_plies as i64,
            requested_games as i64
        ],
    )
    .map_err(|e| format!("create benchmark: {e}"))?;
    Ok(tx.last_insert_rowid())
}

fn add_engine(
    tx: &Transaction<'_>,
    benchmark_id: i64,
    role: &str,
    config: &ResolvedEngine,
) -> Result<i64, String> {
    let build_id = ensure_engine_build(tx, config)?;
    let labels = serde_json::to_string(&config.metadata.labels)
        .map_err(|e| format!("serialize engine labels: {e}"))?;
    tx.execute(
        "INSERT INTO benchmark_engines(
             benchmark_id, engine_build_id, role, name, family, version, labels_json
         ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![
            benchmark_id,
            build_id,
            role,
            config.name,
            config
                .metadata
                .family
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
            config
                .metadata
                .version
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
            labels
        ],
    )
    .map_err(|e| format!("add benchmark engine: {e}"))?;
    Ok(build_id)
}

fn ensure_engine_build(tx: &Transaction<'_>, config: &ResolvedEngine) -> Result<i64, String> {
    let identity = engine_identity(config)?;
    let command = serde_json::to_string(config.launch.command())
        .map_err(|e| format!("serialize engine command: {e}"))?;
    let env = serde_json::to_string(config.launch.env())
        .map_err(|e| format!("serialize engine environment: {e}"))?;
    let ubgi = serde_json::to_string(config.launch.ubgi())
        .map_err(|e| format!("serialize engine UBGI settings: {e}"))?;
    tx.execute(
        "INSERT INTO engine_builds(identity, command_json, env_json, ubgi_json)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(identity) DO NOTHING",
        params![identity, command, env, ubgi],
    )
    .map_err(|e| format!("insert engine build: {e}"))?;
    let build_id: i64 = tx
        .query_row(
            "SELECT id FROM engine_builds WHERE identity = ?",
            params![identity],
            |row| row.get(0),
        )
        .map_err(|e| format!("load engine build: {e}"))?;
    Ok(build_id)
}

fn replace_ranking_build(
    tx: &Transaction<'_>,
    benchmark_id: i64,
    old_build_id: i64,
    new_build_id: i64,
) -> Result<(), String> {
    let duplicate: bool = tx
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM benchmark_engines
               WHERE benchmark_id = ? AND engine_build_id = ? AND role = 'member'
             )",
            params![benchmark_id, new_build_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("check replacement ranking build: {e}"))?;
    if duplicate {
        return Err("refreshed UBGI settings duplicate another engine in the ranking".to_string());
    }

    for column in ["engine_a_id", "engine_b_id"] {
        tx.execute(
            &format!(
                "UPDATE matchups SET {column} = ?
                 WHERE benchmark_id = ? AND {column} = ?"
            ),
            params![new_build_id, benchmark_id, old_build_id],
        )
        .map_err(|e| format!("replace ranking matchup build: {e}"))?;
    }
    tx.execute(
        "UPDATE benchmark_engines SET engine_build_id = ?
         WHERE benchmark_id = ? AND engine_build_id = ? AND role = 'member'",
        params![new_build_id, benchmark_id, old_build_id],
    )
    .map_err(|e| format!("replace ranking member build: {e}"))?;
    Ok(())
}

fn engine_identity(config: &ResolvedEngine) -> Result<String, String> {
    serde_json::to_string(&config.launch)
        .map_err(|e| format!("serialize engine {} launch: {e}", config.name))
}

fn add_matchup(
    tx: &Transaction<'_>,
    benchmark_id: i64,
    engine_a_id: i64,
    engine_b_id: i64,
    games: usize,
    seed: u64,
    batch_index: usize,
) -> Result<MatchupHandle, String> {
    tx.execute(
        "INSERT INTO matchups(
            benchmark_id, engine_a_id, engine_b_id, requested_games, seed, batch_index
         ) VALUES (?, ?, ?, ?, ?, ?)",
        params![
            benchmark_id,
            engine_a_id,
            engine_b_id,
            games as i64,
            seed.to_string(),
            batch_index as i64
        ],
    )
    .map_err(|e| format!("create matchup: {e}"))?;
    Ok(MatchupHandle {
        id: tx.last_insert_rowid(),
        games,
        seed,
    })
}

fn validate_games(matchup: MatchupHandle, games: &[GameRecord]) -> Result<(), String> {
    let expected_games = matchup.games;
    if games.len() != expected_games {
        return Err(format!(
            "matchup {} produced {}/{} games",
            matchup.id,
            games.len(),
            expected_games
        ));
    }
    for (index, game) in games.iter().enumerate() {
        if game.game_idx != index {
            return Err(format!("invalid mirrored sequence at game {}", index + 1));
        }
        let expected_pair_index = index / 2;
        let expected_leg = if index + 1 == expected_games && expected_games % 2 == 1 {
            crate::duel_game::singleton_leg(matchup.seed, expected_pair_index)
        } else {
            index % 2
        };
        if game.pair_index != expected_pair_index || game.leg != expected_leg {
            return Err(format!(
                "invalid scheduled cluster/leg at game {}",
                index + 1
            ));
        }
        if !game.points_a.is_finite()
            || ![-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0].contains(&game.points_a)
        {
            return Err(format!("game {} has invalid points", index + 1));
        }
    }
    Ok(())
}

fn initialize_schema(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| format!("read schema version: {e}"))?;
    if version != 0 && version != SCHEMA_VERSION {
        return Err(format!("database schema {version} is unsupported"));
    }
    if version == 0 {
        conn.execute_batch(
            "BEGIN;
              CREATE TABLE engine_builds (
                id INTEGER PRIMARY KEY,
                identity TEXT NOT NULL UNIQUE,
                command_json TEXT NOT NULL,
               env_json TEXT NOT NULL,
               ubgi_json TEXT NOT NULL,
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
                requested_games INTEGER NOT NULL CHECK(requested_games >= 0),
               placement_opponents INTEGER,
                placement_games INTEGER,
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
                name TEXT NOT NULL,
                family TEXT,
               version TEXT,
               labels_json TEXT NOT NULL DEFAULT '{}',
               PRIMARY KEY(benchmark_id, role, engine_build_id)
             );
             CREATE TABLE matchups (
               id INTEGER PRIMARY KEY,
               benchmark_id INTEGER NOT NULL REFERENCES benchmarks(id),
               engine_a_id INTEGER NOT NULL REFERENCES engine_builds(id),
               engine_b_id INTEGER NOT NULL REFERENCES engine_builds(id),
                requested_games INTEGER NOT NULL CHECK(requested_games > 0),
               seed TEXT NOT NULL,
               batch_index INTEGER NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               UNIQUE(benchmark_id, batch_index)
             );
               CREATE TABLE games (
                 id INTEGER PRIMARY KEY,
                 matchup_id INTEGER NOT NULL REFERENCES matchups(id),
                 pair_index INTEGER NOT NULL CHECK(pair_index >= 0),
                 leg INTEGER NOT NULL CHECK(leg IN (0, 1)),
                points_a REAL NOT NULL CHECK(points_a IN (-3, -2, -1, 0, 1, 2, 3)),
                plies INTEGER NOT NULL,
                decisions_a INTEGER NOT NULL CHECK(decisions_a >= 0),
                decisions_b INTEGER NOT NULL CHECK(decisions_b >= 0),
                decision_seconds_a REAL NOT NULL CHECK(decision_seconds_a >= 0),
                 decision_seconds_b REAL NOT NULL CHECK(decision_seconds_b >= 0),
                 UNIQUE(matchup_id, pair_index, leg)
               );
               PRAGMA user_version = 2;
              COMMIT;",
        )
        .map_err(|e| format!("create database schema: {e}"))?;
    }
    Ok(())
}

pub fn default_db_path() -> PathBuf {
    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg_data_home).join("bgci").join("bgci.db");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("bgci")
            .join("bgci.db");
    }
    PathBuf::from("bgci.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ranking::{fit_rating_model, select_pair_for_model};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn config(name: &str) -> ResolvedEngine {
        ResolvedEngine {
            name: name.to_string(),
            launch: EngineLaunch::new(
                vec![name.to_string()],
                Default::default(),
                Default::default(),
            )
            .unwrap(),
            metadata: EngineMetadata::default(),
        }
    }

    fn spec(games: usize) -> BenchmarkSpec<'static> {
        BenchmarkSpec {
            name: "benchmark",
            variant: "backgammon",
            seed: 42,
            max_plies: 512,
            games,
        }
    }

    fn game(game_idx: usize) -> GameRecord {
        GameRecord {
            game_idx,
            pair_index: game_idx / 2,
            leg: game_idx % 2,
            points_a: 1.0,
            plies: 10,
            a_decisions: 5,
            b_decisions: 5,
            a_decision_time: Duration::from_millis(50),
            b_decision_time: Duration::from_millis(100),
            transcript: None,
        }
    }

    fn scheduled_games(count: usize, seed: u64) -> Vec<GameRecord> {
        (0..count)
            .map(|game_idx| {
                let mut game = game(game_idx);
                if game_idx + 1 == count && count % 2 == 1 {
                    game.leg = crate::duel_game::singleton_leg(seed, game.pair_index);
                }
                game
            })
            .collect()
    }

    fn database() -> Database {
        Database::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn ranking_spec() -> RankingSpec<'static> {
        RankingSpec {
            name: "ranking",
            variant: "backgammon",
            seed: 42,
            max_plies: 512,
            placement_opponents: 1,
            placement_games: 2,
            established_rd: 80.0,
        }
    }

    fn row_counts(store: &Database) -> [i64; 5] {
        [
            "benchmarks",
            "engine_builds",
            "benchmark_engines",
            "matchups",
            "games",
        ]
        .map(|table| {
            store
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        })
    }

    struct TempDatabase(PathBuf);

    impl TempDatabase {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(std::env::temp_dir().join(format!(
                "bgci-ranking-{}-{unique}.sqlite",
                std::process::id()
            )))
        }
    }

    impl Drop for TempDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            for suffix in ["-wal", "-shm"] {
                let _ = fs::remove_file(format!("{}{suffix}", self.0.display()));
            }
        }
    }

    #[test]
    fn saved_duel_lifecycle_preserves_metadata_legs_and_standings() {
        let mut store = database();
        let mut engine_a = config("a");
        engine_a.metadata.family = Some("kestral".to_string());
        engine_a.metadata.version = Some("v2".to_string());
        engine_a
            .metadata
            .labels
            .insert("model".to_string(), "large".to_string());
        let started = store.start_duel(spec(2), &engine_a, &config("b")).unwrap();

        assert!(store.finish_benchmark(started.id).is_err());
        let mut second = game(1);
        second.points_a = -2.0;
        store
            .record_games(started.matchups[0].handle, &[game(0), second])
            .unwrap();
        store.finish_benchmark(started.id).unwrap();

        let summary = store.get(started.id).unwrap().unwrap();
        assert_eq!(summary.name, "benchmark");
        assert_eq!(summary.kind, "duel");
        assert_eq!(summary.variant, "backgammon");
        assert_eq!(summary.status, "completed");
        assert_eq!((summary.requested_games, summary.games), (2, 2));
        let legs = store
            .conn
            .prepare("SELECT pair_index, leg, points_a FROM games ORDER BY pair_index, leg")
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(legs, [(0, 0, 1.0), (0, 1, -2.0)]);
        let summaries = store.engine_summaries(started.id).unwrap();
        let a = summaries
            .iter()
            .find(|summary| summary.name == "a")
            .unwrap();
        let b = summaries
            .iter()
            .find(|summary| summary.name == "b")
            .unwrap();
        assert_eq!(
            (a.family.as_deref(), a.role.as_str()),
            (Some("kestral"), "engine-a")
        );
        assert_eq!((a.games, a.wins, a.points), (2, 1, -1.0));
        assert_eq!((b.games, b.wins, b.points), (2, 1, 1.0));
        let metadata: (Option<String>, String) = store
            .conn
            .query_row(
                "SELECT version, labels_json FROM benchmark_engines WHERE name = 'a'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(metadata.0.as_deref(), Some("v2"));
        assert_eq!(
            serde_json::from_str::<BTreeMap<String, String>>(&metadata.1).unwrap()["model"],
            "large"
        );
    }

    #[test]
    fn game_validation_duplicate_ingestion_and_mid_write_failure_are_atomic() {
        let mut store = database();
        let first = store
            .start_duel(spec(2), &config("a"), &config("b"))
            .unwrap();
        let handle = first.matchups[0].handle;
        let mut wrong_index = game(0);
        wrong_index.game_idx = 1;
        assert!(store.record_games(handle, &[wrong_index, game(1)]).is_err());
        for points in [4.0, 0.5, f64::INFINITY, f64::NAN] {
            let mut invalid = game(0);
            invalid.points_a = points;
            assert!(store.record_games(handle, &[invalid, game(1)]).is_err());
        }
        assert_eq!(row_counts(&store)[4], 0);
        store.record_games(handle, &[game(0), game(1)]).unwrap();
        assert!(store.record_games(handle, &[game(0), game(1)]).is_err());

        let second = store
            .start_duel(spec(2), &config("a"), &config("b"))
            .unwrap();
        store
            .conn
            .execute_batch(&format!(
                "CREATE TRIGGER reject_second_leg BEFORE INSERT ON games
                 WHEN NEW.matchup_id = {} AND NEW.leg = 1
                 BEGIN SELECT RAISE(ABORT, 'second leg rejected'); END;",
                second.matchups[0].handle.id
            ))
            .unwrap();
        assert!(
            store
                .record_games(second.matchups[0].handle, &[game(0), game(1)])
                .is_err()
        );
        let second_games: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM games WHERE matchup_id = ?",
                params![second.matchups[0].handle.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(second_games, 0);
        assert_eq!(row_counts(&store)[4], 2);
    }

    #[test]
    fn odd_game_counts_persist_exact_scheduled_singletons() {
        for count in [1, 3] {
            let mut store = database();
            let started = store
                .start_duel(spec(count), &config("a"), &config("b"))
                .unwrap();
            let handle = started.matchups[0].handle;
            let games = scheduled_games(count, handle.seed());
            let mut wrong_leg = games.clone();
            wrong_leg[count - 1].leg ^= 1;
            assert!(store.record_games(handle, &wrong_leg).is_err());
            store.record_games(handle, &games).unwrap();
            store.finish_benchmark(started.id).unwrap();
            assert_eq!(store.get(started.id).unwrap().unwrap().games, count);
        }
    }

    #[test]
    fn invalid_duel_and_league_starts_leave_every_table_unchanged() {
        let mut store = database();
        let empty = row_counts(&store);
        let mut unnamed = spec(1);
        unnamed.name = "  ";
        assert!(
            store
                .start_duel(unnamed, &config("a"), &config("b"))
                .is_err()
        );
        assert_eq!(row_counts(&store), empty);
        assert!(
            store
                .start_duel(spec(1), &config("a"), &config("a"))
                .is_err()
        );
        assert_eq!(row_counts(&store), empty);
        assert!(store.start_league(spec(1), &[config("only")]).is_err());
        assert_eq!(row_counts(&store), empty);
        let first = config("engine");
        let mut renamed = first.clone();
        renamed.name = "renamed".to_string();
        assert!(store.start_league(spec(1), &[first, renamed]).is_err());
        assert_eq!(row_counts(&store), empty);
    }

    #[test]
    fn benchmark_start_rolls_back_after_a_mid_transaction_failure() {
        let mut store = database();
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER reject_matchup BEFORE INSERT ON matchups
                 BEGIN SELECT RAISE(ABORT, 'matchup rejected'); END;",
            )
            .unwrap();

        assert!(
            store
                .start_duel(spec(1), &config("a"), &config("b"))
                .is_err()
        );
        assert_eq!(row_counts(&store), [0; 5]);
    }

    #[test]
    fn league_schedule_has_deterministic_pairs_indices_and_seeds() {
        let mut store = database();
        let started = store
            .start_league(spec(2), &[config("a"), config("b"), config("c")])
            .unwrap();
        assert_eq!(
            started
                .matchups
                .iter()
                .enumerate()
                .map(|(index, matchup)| (
                    matchup.engine_a,
                    matchup.engine_b,
                    matchup.handle.seed(),
                    index,
                ))
                .collect::<Vec<_>>(),
            [
                (0, 1, seed_for_game(42, 0), 0),
                (0, 2, seed_for_game(42, 1), 1),
                (1, 2, seed_for_game(42, 2), 2),
            ]
        );
        let summary = store.get(started.id).unwrap().unwrap();
        assert_eq!(
            (summary.kind.as_str(), summary.requested_games),
            ("league", 6)
        );
    }

    #[test]
    fn file_backed_ranking_create_resume_pause_batch_and_empty_retry_lifecycle() {
        let path = TempDatabase::new();
        let pool_id = {
            let mut store = Database::open(&path.0).unwrap();
            let pool = store
                .start_ranking(ranking_spec(), &[config("a"), config("b")])
                .unwrap();
            assert_eq!(pool.status, "paused");
            assert_eq!((pool.seed, pool.next_batch, pool.engines.len()), (42, 0, 2));
            pool.id
        };

        let mut store = Database::open(&path.0).unwrap();
        assert_eq!(store.load_ranking_by_name("RANKING").unwrap().id, pool_id);
        store.resume_ranking(pool_id).unwrap();
        let pool = store.load_ranking(pool_id).unwrap();
        assert_eq!(pool.status, "running");
        let empty = store.start_ranking_batch(&pool, 0, 1, 2).unwrap();
        store.discard_empty_matchup(empty).unwrap();
        let retry = store.load_ranking(pool_id).unwrap();
        assert_eq!(retry.next_batch, 0);
        let matchup = store.start_ranking_batch(&retry, 0, 1, 2).unwrap();
        store.record_games(matchup, &[game(0), game(1)]).unwrap();
        store.pause_ranking(pool_id).unwrap();
        let expanded = store
            .add_ranking_engines("ranking", &[config("c")])
            .unwrap();
        assert_eq!(
            (expanded.status.as_str(), expanded.next_batch),
            ("paused", 1)
        );
        assert_eq!(expanded.engines.len(), 3);
    }

    #[test]
    fn ranking_persistence_feeds_canonical_pair_robust_model_selection() {
        let mut store = database();
        let created = store
            .start_ranking(ranking_spec(), &[config("a"), config("b")])
            .unwrap();
        store.resume_ranking(created.id).unwrap();
        let pool = store.load_ranking(created.id).unwrap();
        let matchup = store.start_ranking_batch(&pool, 1, 0, 3).unwrap();
        store
            .record_games(matchup, &scheduled_games(3, matchup.seed()))
            .unwrap();

        let data = store.ranking_data(&pool).unwrap();
        let edge = &data.edges[0];
        assert_eq!((edge.engine_a, edge.engine_b), (0, 1));
        assert_eq!(edge.completed_clusters, 2);
        assert_eq!(edge.rated_games, 3);
        assert!((edge.score_sum_a - 1.0).abs() < 1e-12);
        assert!((edge.sum_m_squared - 5.0).abs() < 1e-12);
        assert!((edge.sum_m_score - 5.0 / 3.0).abs() < 1e-12);
        assert!((edge.sum_score_squared - 5.0 / 9.0).abs() < 1e-12);
        assert_eq!(data.game_counts, vec![vec![0, 3], vec![3, 0]]);
        assert_eq!(
            data.average_decision_time,
            vec![
                Some(Duration::from_millis(20)),
                Some(Duration::from_millis(10))
            ]
        );
        assert_eq!(data.last_played_batch, vec![Some(0), Some(0)]);
        let model = fit_rating_model(pool.engines.len(), &data.edges);
        assert_eq!(
            select_pair_for_model(
                &model,
                &data.game_counts,
                &data.average_decision_time,
                &data.last_played_batch,
                pool.next_batch + 1,
                pool.placement_opponents,
                pool.placement_games,
            ),
            Some((0, 1))
        );
    }

    #[test]
    fn ranking_workload_counts_include_incomplete_singletons() {
        let mut store = database();
        let created = store
            .start_ranking(ranking_spec(), &[config("a"), config("b")])
            .unwrap();
        store.resume_ranking(created.id).unwrap();
        let pool = store.load_ranking(created.id).unwrap();
        let matchup = store.start_ranking_batch(&pool, 0, 1, 1).unwrap();
        let mut games = scheduled_games(1, matchup.seed());
        games[0].points_a = 0.0;
        store.record_games(matchup, &games).unwrap();

        let data = store.ranking_data(&pool).unwrap();
        assert_eq!(data.game_counts, vec![vec![0, 1], vec![1, 0]]);
        assert_eq!(data.edges[0].rated_games, 0);
        assert_eq!(data.edges[0].completed_clusters, 0);
    }

    #[test]
    fn ranking_refresh_replaces_only_its_immutable_build_snapshot() {
        let mut store = database();
        let created = store
            .start_ranking(ranking_spec(), &[config("a"), config("b")])
            .unwrap();
        store.resume_ranking(created.id).unwrap();
        let pool = store.load_ranking(created.id).unwrap();
        let old_a_id = pool.engines[0].build_id;
        let shared = store
            .start_duel(spec(1), &config("a"), &config("b"))
            .unwrap();
        let matchup = store.start_ranking_batch(&pool, 0, 1, 2).unwrap();
        store.record_games(matchup, &[game(0), game(1)]).unwrap();
        store.pause_ranking(pool.id).unwrap();
        let mut a = config("a");
        a.metadata.family = Some("family-a".to_string());
        a.metadata.version = Some("v2".to_string());
        a.metadata
            .labels
            .insert("model".to_string(), "large".to_string());
        a.launch
            .ubgi_mut()
            .insert("engine.ply".to_string(), "1".to_string());

        let refreshed = store
            .refresh_ranking_engine_metadata(&pool.name, &[a, config("b")], true)
            .unwrap();
        let refreshed_a = refreshed
            .engines
            .iter()
            .find(|engine| engine.config.name == "a")
            .unwrap();
        assert_ne!(refreshed_a.build_id, old_a_id);
        assert_eq!(
            refreshed_a.config.metadata.family.as_deref(),
            Some("family-a")
        );
        assert_eq!(refreshed_a.config.metadata.version.as_deref(), Some("v2"));
        assert_eq!(refreshed_a.config.metadata.labels["model"], "large");
        assert_eq!(refreshed_a.config.launch.ubgi()["engine.ply"], "1");
        let shared_a_id: i64 = store
            .conn
            .query_row(
                "SELECT engine_a_id FROM matchups WHERE benchmark_id = ?",
                params![shared.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(shared_a_id, old_a_id);
        let old_ubgi: String = store
            .conn
            .query_row(
                "SELECT ubgi_json FROM engine_builds WHERE id = ?",
                params![old_a_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_ubgi, "{}");
        let data = store.ranking_data(&refreshed).unwrap();
        assert_eq!((data.edges.len(), data.edges[0].rated_games), (1, 2));
        assert_eq!(data.game_counts[0][1], 2);
    }

    #[test]
    fn rejects_an_unsupported_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();

        let error = Database::from_connection(conn).err().unwrap();
        assert_eq!(error, "database schema 1 is unsupported");
    }

    #[test]
    fn creates_only_version_two_profile_vocabulary_columns() {
        let store = database();
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);

        for (table, required, rejected) in [
            (
                "benchmarks",
                "requested_games",
                ["requested_pairs", "placement_pairs"],
            ),
            (
                "matchups",
                "requested_games",
                ["requested_pairs", "placement_pairs"],
            ),
            (
                "engine_builds",
                "ubgi_json",
                ["options_json", "configuration_json"],
            ),
            (
                "benchmark_engines",
                "labels_json",
                ["options_json", "configuration_json"],
            ),
        ] {
            let columns = store
                .conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(columns.iter().any(|column| column == required));
            assert!(
                rejected
                    .iter()
                    .all(|name| !columns.iter().any(|column| column == name))
            );
        }
    }
}
