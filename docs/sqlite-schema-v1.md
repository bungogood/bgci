# SQLite Schema v1 (Managed Runs)

## Goals

- Durable run and job lifecycle
- Safe retry/idempotency
- Resume support after client/coordinator interruption
- Efficient status queries for CLI/dashboard

## Pragmas

Recommended on open:

- `PRAGMA journal_mode=WAL;`
- `PRAGMA synchronous=NORMAL;`
- `PRAGMA foreign_keys=ON;`

## Tables

### runs

- `id TEXT PRIMARY KEY` (ULID/UUID)
- `name TEXT`
- `status TEXT NOT NULL` (`queued|running|completed|failed|cancelled`)
- `spec_json TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `started_at TEXT`
- `finished_at TEXT`
- `total_games INTEGER NOT NULL`
- `completed_games INTEGER NOT NULL DEFAULT 0`
- `failed_games INTEGER NOT NULL DEFAULT 0`

### jobs

- `id TEXT PRIMARY KEY`
- `run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE`
- `game_id INTEGER NOT NULL`
- `seed INTEGER NOT NULL`
- `a_is_x INTEGER NOT NULL`
- `status TEXT NOT NULL` (`queued|leased|running|succeeded|failed|cancelled`)
- `lease_owner TEXT`
- `lease_expires_at TEXT`
- `attempt_count INTEGER NOT NULL DEFAULT 0`
- `last_error TEXT`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

Unique index:

- `UNIQUE(run_id, game_id)`

### job_attempts

- `id TEXT PRIMARY KEY`
- `job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE`
- `worker_id TEXT`
- `status TEXT NOT NULL` (`running|succeeded|failed|timed_out|cancelled`)
- `started_at TEXT NOT NULL`
- `ended_at TEXT`
- `error_text TEXT`

### results

- `job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE`
- `winner_x INTEGER`
- `points_x REAL NOT NULL`
- `points_o REAL NOT NULL`
- `plies INTEGER NOT NULL`
- `a_decisions INTEGER NOT NULL`
- `b_decisions INTEGER NOT NULL`
- `a_decision_sec REAL NOT NULL`
- `b_decision_sec REAL NOT NULL`
- `trace_path TEXT`
- `ubgi_path_a TEXT`
- `ubgi_path_b TEXT`
- `created_at TEXT NOT NULL`

### events

- `id INTEGER PRIMARY KEY AUTOINCREMENT`
- `run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE`
- `ts TEXT NOT NULL`
- `kind TEXT NOT NULL`
- `payload_json TEXT NOT NULL`

Index:

- `(run_id, id)` for ordered replay.

### workers

- `id TEXT PRIMARY KEY`
- `status TEXT NOT NULL` (`online|offline|draining`)
- `last_seen_at TEXT NOT NULL`
- `capabilities_json TEXT NOT NULL`

## State Transitions

Job lifecycle:

- `queued -> leased -> running -> succeeded`
- `queued -> leased -> running -> failed`
- `leased -> queued` (lease expiry)
- `running -> queued` (worker lost + lease expiry)

Run lifecycle:

- `queued -> running -> completed`
- `queued|running -> failed`
- `queued|running -> cancelled`

## Idempotency Rules

- A game is uniquely identified by `(run_id, game_id)`.
- `results.job_id` primary key guarantees single accepted final result.
- Duplicate late submissions should be acknowledged but ignored.

## Query Patterns

- Run status summary by `run_id`
- Pending jobs count by status
- Recent failures with error text
- Progress timeline from `events`

## Migration Strategy

- Use `schema_version` table for migrations.
- Keep additive migrations for v1-v2 compatibility.
- Add periodic `VACUUM`/archive command for long-term storage control.
