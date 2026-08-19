use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::config::EngineConfig;
use crate::duel_game::seed_for_game;
use crate::duel_runner::GameRecord;

const SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BenchmarkKind {
    Duel,
    League,
}

impl BenchmarkKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Duel => "duel",
            Self::League => "league",
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
                tx.execute(
                    "INSERT INTO games(
                        pair_id, leg, game_index, engine_x_id, engine_o_id, winner_id,
                        outcome, points_x, points_o, points_a, points_b, plies
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                        game.plies as i64
                    ],
                )
                .map_err(|e| format!("insert game {}: {e}", game.game_idx))?;
            }
            tx.execute(
                "UPDATE pairs SET status = 'completed' WHERE id = ?",
                params![pair_id],
            )
            .map_err(|e| format!("complete pair: {e}"))?;
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
                "SELECT eb.name, be.role, COUNT(g.id),
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
                 GROUP BY eb.id, be.role
                 ORDER BY points * 1.0 / MAX(COUNT(g.id), 1) DESC, eb.name",
            )
            .map_err(|e| format!("prepare benchmark standings: {e}"))?;
        let rows = stmt
            .query_map(params![benchmark_id], |row| {
                Ok(EngineSummary {
                    name: row.get(0)?,
                    role: row.get(1)?,
                    games: row.get::<_, i64>(2)? as usize,
                    wins: row.get::<_, i64>(3)? as usize,
                    points: row.get(4)?,
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
    if requested_pairs == 0 {
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
        "INSERT INTO benchmark_engines(benchmark_id, engine_build_id, role)
         VALUES (?, ?, ?)",
        params![benchmark_id, build_id, role],
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
) -> Result<MatchupHandle, String> {
    tx.execute(
        "INSERT INTO matchups(
            benchmark_id, engine_a_id, engine_b_id, requested_pairs, seed
         ) VALUES (?, ?, ?, ?, ?)",
        params![
            benchmark_id,
            engine_a_id,
            engine_b_id,
            pairs as i64,
            seed.to_string()
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
        if game.game_idx != index || game.a_is_x != index.is_multiple_of(2) {
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
               kind TEXT NOT NULL CHECK(kind IN ('duel', 'league')),
               status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'failed')),
               variant TEXT NOT NULL,
               seed TEXT NOT NULL,
               max_plies INTEGER NOT NULL CHECK(max_plies > 0),
               requested_pairs INTEGER NOT NULL CHECK(requested_pairs > 0),
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               completed_at TEXT
             );
             CREATE TABLE benchmark_engines (
               benchmark_id INTEGER NOT NULL REFERENCES benchmarks(id),
               engine_build_id INTEGER NOT NULL REFERENCES engine_builds(id),
               role TEXT NOT NULL,
               PRIMARY KEY(benchmark_id, role, engine_build_id)
             );
             CREATE TABLE matchups (
               id INTEGER PRIMARY KEY,
               benchmark_id INTEGER NOT NULL REFERENCES benchmarks(id),
               engine_a_id INTEGER NOT NULL REFERENCES engine_builds(id),
               engine_b_id INTEGER NOT NULL REFERENCES engine_builds(id),
               requested_pairs INTEGER NOT NULL CHECK(requested_pairs > 0),
               seed TEXT NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               UNIQUE(benchmark_id, engine_a_id, engine_b_id)
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
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               UNIQUE(pair_id, leg),
               UNIQUE(pair_id, game_index)
             );
             PRAGMA user_version = 1;
             COMMIT;",
        )
        .map_err(|e| format!("apply benchmark schema v1: {e}"))?;
    }
    Ok(())
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
}
