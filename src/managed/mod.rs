use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

use crate::domain::MatchPlan;
use crate::duel_runner::ProgressSnapshot;

pub struct ManagedRunStore {
    rt: tokio::runtime::Runtime,
    pool: SqlitePool,
}

pub struct RunRow {
    pub id: String,
    pub name: String,
    pub status: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub total_games: i64,
    pub completed_games: i64,
    pub failed_games: i64,
    pub output_csv: Option<String>,
    pub log_file: Option<String>,
}

pub struct RunSnapshot {
    pub line_engines: String,
    pub line_result: String,
    pub line_rate: String,
    pub line_decide: String,
    pub line_class: String,
    pub line_sides: String,
}

impl ManagedRunStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create db dir {}: {e}", parent.display()))?;
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to create runtime: {e}"))?;

        let db_url = sqlite_url(path);
        let connect_options = SqliteConnectOptions::from_str(&db_url)
            .map_err(|e| format!("failed to parse sqlite url {}: {e}", path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true);
        let pool = rt.block_on(async {
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(connect_options)
                .await
        })
        .map_err(|e| format!("failed to open sqlite db {}: {e}", path.display()))?;

        rt.block_on(async {

            sqlx::query(
                "CREATE TABLE IF NOT EXISTS runs (
                    id TEXT PRIMARY KEY,
                    name TEXT,
                    status TEXT NOT NULL,
                    spec_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    started_at TEXT,
                    finished_at TEXT,
                    total_games INTEGER NOT NULL,
                    completed_games INTEGER NOT NULL DEFAULT 0,
                    failed_games INTEGER NOT NULL DEFAULT 0,
                    output_csv TEXT,
                    log_file TEXT
                )",
            )
            .execute(&pool)
            .await?;

            sqlx::query(
                "CREATE TABLE IF NOT EXISTS jobs (
                    id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
                    game_id INTEGER NOT NULL,
                    seed INTEGER NOT NULL,
                    a_is_x INTEGER NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE(run_id, game_id)
                )",
            )
            .execute(&pool)
            .await?;

            sqlx::query(
                "CREATE TABLE IF NOT EXISTS run_snapshots (
                    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
                    done_games INTEGER NOT NULL,
                    line_engines TEXT NOT NULL,
                    line_result TEXT NOT NULL,
                    line_rate TEXT NOT NULL,
                    line_decide TEXT NOT NULL,
                    line_class TEXT NOT NULL,
                    line_sides TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                )",
            )
            .execute(&pool)
            .await?;

            Result::<(), sqlx::Error>::Ok(())
        })
        .map_err(|e| format!("failed to initialize sqlite schema: {e}"))?;

        Ok(Self { rt, pool })
    }

    pub fn create_run(&mut self, plan: &MatchPlan, run_name: Option<&str>) -> Result<String, String> {
        let run_id = generate_run_id();
        let created_at = now_utc_rfc3339();
        let spec_json = serialize_plan(plan);

        self.rt
            .block_on(async {
                let mut tx = self.pool.begin().await?;
                sqlx::query(
                    "INSERT INTO runs (id, name, status, spec_json, created_at, total_games, completed_games, failed_games)
                     VALUES (?1, ?2, 'queued', ?3, ?4, ?5, 0, 0)",
                )
                .bind(&run_id)
                .bind(run_name.unwrap_or(""))
                .bind(spec_json)
                .bind(&created_at)
                .bind(plan.config.games as i64)
                .execute(&mut *tx)
                .await?;

                for game_idx in 0..plan.config.games {
                    let job_id = format!("{}-{:05}", run_id, game_idx + 1);
                    let seed = seed_for_game(plan.config.seed, game_idx) as i64;
                    let a_is_x = if !(plan.config.swap_sides && game_idx % 2 == 1) {
                        1i64
                    } else {
                        0i64
                    };
                    sqlx::query(
                        "INSERT INTO jobs (id, run_id, game_id, seed, a_is_x, status, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?6)",
                    )
                    .bind(job_id)
                    .bind(&run_id)
                    .bind(game_idx as i64 + 1)
                    .bind(seed)
                    .bind(a_is_x)
                    .bind(&created_at)
                    .execute(&mut *tx)
                    .await?;
                }

                tx.commit().await?;
                Result::<(), sqlx::Error>::Ok(())
            })
            .map_err(|e| e.to_string())?;

        Ok(run_id)
    }

    #[allow(dead_code)]
    pub fn mark_run_started(
        &self,
        run_id: &str,
        output_csv: &Path,
        log_file: &Path,
    ) -> Result<(), String> {
        let now = now_utc_rfc3339();
        self.rt
            .block_on(async {
                sqlx::query(
                    "UPDATE runs SET status='running', started_at=?2, output_csv=?3, log_file=?4 WHERE id=?1",
                )
                    .bind(run_id)
                    .bind(&now)
                    .bind(output_csv.display().to_string())
                    .bind(log_file.display().to_string())
                    .execute(&self.pool)
                    .await?;
                sqlx::query("UPDATE jobs SET status='running', updated_at=?2 WHERE run_id=?1")
                    .bind(run_id)
                    .bind(now_utc_rfc3339())
                    .execute(&self.pool)
                    .await?;
                Result::<(), sqlx::Error>::Ok(())
            })
            .map_err(|e| e.to_string())
    }

    #[allow(dead_code)]
    pub fn update_run_progress(&self, run_id: &str, completed_games: usize) -> Result<(), String> {
        self.rt
            .block_on(async {
                sqlx::query("UPDATE runs SET completed_games=?2 WHERE id=?1")
                    .bind(run_id)
                    .bind(completed_games as i64)
                    .execute(&self.pool)
                    .await?;
                Result::<(), sqlx::Error>::Ok(())
            })
            .map_err(|e| e.to_string())
    }

    #[allow(dead_code)]
    pub fn mark_run_completed(
        &self,
        run_id: &str,
        output_csv: &Path,
        log_file: &Path,
        completed_games: usize,
    ) -> Result<(), String> {
        let now = now_utc_rfc3339();
        self.rt
            .block_on(async {
                sqlx::query("UPDATE runs SET status='completed', finished_at=?2, completed_games=?3, output_csv=?4, log_file=?5 WHERE id=?1")
                    .bind(run_id)
                    .bind(now)
                    .bind(completed_games as i64)
                    .bind(output_csv.display().to_string())
                    .bind(log_file.display().to_string())
                    .execute(&self.pool)
                    .await?;
                sqlx::query("UPDATE jobs SET status='succeeded', updated_at=?2 WHERE run_id=?1")
                    .bind(run_id)
                    .bind(now_utc_rfc3339())
                    .execute(&self.pool)
                    .await?;
                Result::<(), sqlx::Error>::Ok(())
            })
            .map_err(|e| e.to_string())
    }

    #[allow(dead_code)]
    pub fn mark_run_failed(&self, run_id: &str, failed_jobs: usize) -> Result<(), String> {
        self.rt
            .block_on(async {
                sqlx::query("UPDATE runs SET status='failed', finished_at=?2, failed_games=?3 WHERE id=?1")
                    .bind(run_id)
                    .bind(now_utc_rfc3339())
                    .bind(failed_jobs as i64)
                    .execute(&self.pool)
                    .await?;
                Result::<(), sqlx::Error>::Ok(())
            })
            .map_err(|e| e.to_string())
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<RunRow>, String> {
        self.rt
            .block_on(async {
                let rows = sqlx::query(
                    "SELECT id, COALESCE(name,''), status, created_at, started_at, finished_at, total_games, completed_games, failed_games, output_csv, log_file
                     FROM runs ORDER BY created_at DESC LIMIT ?1",
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?;

                let mut out = Vec::new();
                for row in rows {
                    out.push(RunRow {
                        id: row.get::<String, _>(0),
                        name: row.get::<String, _>(1),
                        status: row.get::<String, _>(2),
                        created_at: row.get::<String, _>(3),
                        started_at: row.get::<Option<String>, _>(4),
                        finished_at: row.get::<Option<String>, _>(5),
                        total_games: row.get::<i64, _>(6),
                        completed_games: row.get::<i64, _>(7),
                        failed_games: row.get::<i64, _>(8),
                        output_csv: row.get::<Option<String>, _>(9),
                        log_file: row.get::<Option<String>, _>(10),
                    });
                }
                Result::<Vec<RunRow>, sqlx::Error>::Ok(out)
            })
            .map_err(|e| e.to_string())
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<RunRow>, String> {
        self.rt
            .block_on(async {
                let row = sqlx::query(
                    "SELECT id, COALESCE(name,''), status, created_at, started_at, finished_at, total_games, completed_games, failed_games, output_csv, log_file
                     FROM runs WHERE id = ?1",
                )
                .bind(run_id)
                .fetch_optional(&self.pool)
                .await?;

                Result::<Option<RunRow>, sqlx::Error>::Ok(row.map(|r| RunRow {
                    id: r.get::<String, _>(0),
                    name: r.get::<String, _>(1),
                    status: r.get::<String, _>(2),
                    created_at: r.get::<String, _>(3),
                    started_at: r.get::<Option<String>, _>(4),
                    finished_at: r.get::<Option<String>, _>(5),
                    total_games: r.get::<i64, _>(6),
                    completed_games: r.get::<i64, _>(7),
                    failed_games: r.get::<i64, _>(8),
                    output_csv: r.get::<Option<String>, _>(9),
                    log_file: r.get::<Option<String>, _>(10),
                }))
            })
            .map_err(|e| e.to_string())
    }

    pub fn get_job_status_counts(&self, run_id: &str) -> Result<Vec<(String, i64)>, String> {
        self.rt
            .block_on(async {
                let rows = sqlx::query(
                    "SELECT status, COUNT(*) FROM jobs WHERE run_id = ?1 GROUP BY status ORDER BY status",
                )
                .bind(run_id)
                .fetch_all(&self.pool)
                .await?;
                let mut out = Vec::new();
                for row in rows {
                    out.push((row.get::<String, _>(0), row.get::<i64, _>(1)));
                }
                Result::<Vec<(String, i64)>, sqlx::Error>::Ok(out)
            })
            .map_err(|e| e.to_string())
    }

    pub fn cancel_run(&self, run_id: &str) -> Result<bool, String> {
        self.rt
            .block_on(async {
                let now = now_utc_rfc3339();
                let updated = sqlx::query(
                    "UPDATE runs SET status='cancelled', finished_at=?2 WHERE id=?1 AND status IN ('queued','running')",
                )
                .bind(run_id)
                .bind(now)
                .execute(&self.pool)
                .await?;

                sqlx::query(
                    "UPDATE jobs SET status='cancelled', updated_at=?2 WHERE run_id=?1 AND status IN ('queued','leased','running')",
                )
                .bind(run_id)
                .bind(now_utc_rfc3339())
                .execute(&self.pool)
                .await?;

                Result::<bool, sqlx::Error>::Ok(updated.rows_affected() > 0)
            })
            .map_err(|e| e.to_string())
    }

    #[allow(dead_code)]
    pub fn upsert_run_snapshot(
        &self,
        run_id: &str,
        snapshot: &ProgressSnapshot,
    ) -> Result<(), String> {
        self.rt
            .block_on(async {
                sqlx::query(
                    "INSERT INTO run_snapshots (run_id, done_games, line_engines, line_result, line_rate, line_decide, line_class, line_sides, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(run_id) DO UPDATE SET
                        done_games=excluded.done_games,
                        line_engines=excluded.line_engines,
                        line_result=excluded.line_result,
                        line_rate=excluded.line_rate,
                        line_decide=excluded.line_decide,
                        line_class=excluded.line_class,
                        line_sides=excluded.line_sides,
                        updated_at=excluded.updated_at",
                )
                .bind(run_id)
                .bind(snapshot.done_games as i64)
                .bind(&snapshot.lines.line_engines)
                .bind(&snapshot.lines.line_result)
                .bind(&snapshot.lines.line_rate)
                .bind(&snapshot.lines.line_decide)
                .bind(&snapshot.lines.line_class)
                .bind(&snapshot.lines.line_sides)
                .bind(now_utc_rfc3339())
                .execute(&self.pool)
                .await?;
                Result::<(), sqlx::Error>::Ok(())
            })
            .map_err(|e| e.to_string())
    }

    pub fn get_run_snapshot(&self, run_id: &str) -> Result<Option<RunSnapshot>, String> {
        self.rt
            .block_on(async {
                let row = sqlx::query(
                    "SELECT done_games, line_engines, line_result, line_rate, line_decide, line_class, line_sides
                     FROM run_snapshots WHERE run_id = ?1",
                )
                .bind(run_id)
                .fetch_optional(&self.pool)
                .await?;

                Result::<Option<RunSnapshot>, sqlx::Error>::Ok(row.map(|r| RunSnapshot {
                    line_engines: r.get::<String, _>(1),
                    line_result: r.get::<String, _>(2),
                    line_rate: r.get::<String, _>(3),
                    line_decide: r.get::<String, _>(4),
                    line_class: r.get::<String, _>(5),
                    line_sides: r.get::<String, _>(6),
                }))
            })
            .map_err(|e| e.to_string())
    }

    pub fn list_running_run_ids(&self) -> Result<Vec<String>, String> {
        self.rt
            .block_on(async {
                let rows = sqlx::query("SELECT id FROM runs WHERE status='running' ORDER BY created_at DESC")
                    .fetch_all(&self.pool)
                    .await?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row.get::<String, _>(0));
                }
                Result::<Vec<String>, sqlx::Error>::Ok(out)
            })
            .map_err(|e| e.to_string())
    }
}

pub fn default_db_path() -> PathBuf {
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(state_home).join("bgci/runs.db");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/state/bgci/runs.db");
    }
    PathBuf::from("runs.db")
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}", path.display())
}

fn generate_run_id() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "run-{:04}{:02}{:02}-{:02}{:02}{:02}-{:08x}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        fastrand::u32(..),
    )
}

fn now_utc_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn serialize_plan(plan: &MatchPlan) -> String {
    format!(
        "{{\"games\":{},\"parallel\":{},\"seed\":{},\"max_plies\":{},\"swap_sides\":{},\"variant\":\"{}\",\"engine_a\":\"{}\",\"engine_b\":\"{}\"}}",
        plan.config.games,
        plan.config.parallel,
        plan.config.seed,
        plan.config.max_plies,
        plan.config.swap_sides,
        plan.config.variant,
        plan.config.engine_a.name,
        plan.config.engine_b.name,
    )
}

fn seed_for_game(base_seed: u64, game_idx: usize) -> u64 {
    let mut z = base_seed.wrapping_add((game_idx as u64).wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}
