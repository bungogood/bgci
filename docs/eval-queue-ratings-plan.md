# Eval Queue + Ratings Plan

This document defines how `bgci` evolves from a duel tool into a queue-driven evaluation and ratings system while keeping the existing duel UX simple.

## Goals

- Keep `bgci duel` fast and fun for manual engine-vs-engine play.
- Add a queue-backed evaluation mode that can run continuously.
- Support local workers first, remote workers later, without rewriting core logic.
- Produce stable Elo-style ratings with uncertainty from accumulated duel data.
- Enable future `bgci eval <engine>` style workflows.

## Guiding Principles

- One duel runner implementation, multiple orchestration modes.
- Queue and ratings are opt-in; default duel remains stateless.
- Persistent state in SQLite for resumability and auditability.
- Idempotent jobs and deterministic seeds to avoid duplicate accounting.
- Clear separation of concerns:
  - duel execution
  - job scheduling
  - rating post-processing

## Product Modes

1. Manual duel mode
   - `bgci duel ...`
   - No DB writes by default.
   - Optional `--record` to store run in SQLite.

2. Evaluation mode (queue-backed)
   - `bgci ratings run ...` or `bgci eval ...`
   - Planner enqueues jobs.
   - Workers execute jobs.
   - Post-processor updates ratings and uncertainty.

3. Worker mode
   - Local worker: process jobs from SQLite queue.
   - Remote worker (future): same lease/complete protocol over network.

## Target CLI Shape

Short term:

- `bgci duel --engine-a ... --engine-b ... --games ...`
- `bgci duel ... --record --db data/eval.db`
- `bgci ratings run --engines ... --budget-games ... --pair-games ...`

Medium term:

- `bgci queue worker --db data/eval.db`
- `bgci queue status --db data/eval.db`
- `bgci ratings daemon --db data/eval.db --tui`

Long term:

- `bgci eval <engine-alias> --db data/eval.db --target-ci 50 --max-games 5000`

## Architecture

Control plane:

- Planner: chooses next pairings based on ratings + uncertainty + coverage.
- Queue manager: inserts jobs, leases jobs, retries failures.
- Post-processor: ingests completed runs, updates ratings, emits events.

Data plane:

- Workers execute duel jobs using existing duel runner.
- Workers write artifacts and submit structured results.

Presentation:

- TUI subscribes to events/state snapshots.
- File logs via `tracing` for `tail -f` workflows.

## SQLite Schema (Initial)

Minimum tables:

- `engines`
  - `id`, `name`, `command_json`, `command_hash`, `created_at`

- `jobs`
  - `id`, `type`, `status` (`queued|leased|running|done|failed`)
  - `spec_json`, `priority`, `attempts`, `max_attempts`
  - `next_attempt_at`, `created_at`, `updated_at`

- `job_leases`
  - `job_id`, `worker_id`, `leased_at`, `lease_expires_at`, `heartbeat_at`

- `duel_runs`
  - `id`, `job_id`, `engine_a_id`, `engine_b_id`
  - `seed_base`, `games`, `pair_games`, `parallel`, `variant`
  - `output_csv_path`, `log_path`, `trace_dir`, `started_at`, `finished_at`

- `game_results`
  - `id`, `duel_run_id`, `game_idx`, `a_is_x`
  - `points_a`, `points_b`, `plies`, `winner`

- `rating_snapshots`
  - `id`, `run_group`, `engine_id`, `rating`, `uncertainty`, `games`, `created_at`

- `events` (optional but recommended)
  - `id`, `kind`, `payload_json`, `created_at`

## Job Contract

Job spec includes:

- engine aliases/ids
- game count and side-swap policy
- seed base/range
- variant
- resource hints (`parallel`, optional host tags)

Job lifecycle:

- `queued -> leased -> running -> done`
- failure path: `running -> failed` with retry metadata

Retry policy:

- exponential backoff
- max attempts
- classify retryable vs terminal failures

## Ratings Method (Current and Next)

Current method:

- Glicko-2 online updates from game outcomes.
- Information-guided pair scheduling (Bradley-Terry link + RD uncertainty mass).
- Mirrored paired games for variance reduction (same seed, side swap).

Near-term upgrade:

- Persist all raw ratings games in SQLite.
- Periodic global replay/refit from raw game history.
- Anchoring support:
  - fixed anchors (for absolute pool stability)
  - optional soft anchors (regularized)

Hybrid plan:

- Online updates drive live scheduling/TUI.
- Every N games, recompute from full raw history to reduce path dependence and batch-size sensitivity.
- Use refit snapshot as canonical leaderboard state.

## Scheduling Strategy

Planner priority score should combine:

- high uncertainty engines
- close rating pairs (most informative)
- under-sampled pairs/engines
- optional anchor coverage requirements

Constraints:

- min games per engine
- max repeats per pair in a window
- include anchors periodically

## Logging and TUI

Logging:

- Keep `tracing` as canonical runtime log output.
- Default filter for ratings runs should show orchestrator info without engine wire spam.
- Persist to rolling log file when running long jobs.

TUI (future):

- leaderboard panel (rating, uncertainty, games)
- active jobs panel (worker, pair, progress)
- queue health (queued/leased/running/failed)
- warnings panel (resource pressure, retries)
- recent rating deltas and suggested next pairings

## Remote Worker Readiness

Do now:

- introduce worker capability model (`host`, `arch`, optional tags)
- keep lease protocol storage-backed and transport-agnostic

Later:

- add HTTP/gRPC/NATS transport for remote workers
- keep job/result payload format unchanged

## Phased Implementation Plan

Phase 1: DB + recording

- Add SQLite schema + migrations.
- Add `bgci duel --record --db ...`.
- Record duel runs + game rows.

Phase 2: queue core (local)

- Implement queue CRUD and leasing in SQLite.
- Add local worker command.
- Add retry/backoff and stale lease recovery.

Phase 3: ratings integration

- Change `ratings run` to enqueue jobs, not execute inline.
- Add post-processing loop reading completed jobs.
- Persist periodic rating snapshots.

Phase 4: UX improvements

- Add TUI dashboard.
- Add rolling logs + status commands.
- Add resume/restart safety and checkpoints.

Phase 5: eval command + remote

- Add `bgci eval <engine>` workflow.
- Add remote workers with same lease/result semantics.

## Risks and Mitigations

- Resource pressure during spawn
  - cap worker parallelism
  - reuse workers where possible
  - retry with backoff

- Rating drift or instability
  - anchors
  - uncertainty-gated reporting
  - batch-based updates

- Queue corruption or duplicate processing
  - idempotent job IDs
  - transactional lease/complete operations
  - periodic integrity checks

## Success Criteria

- `bgci duel` remains simple and fast.
- Queue runner can run unattended for long durations.
- Restart does not lose progress or double-count games.
- Leaderboard includes uncertainty and converges with increased budget.
- Remote worker support can be added without redesigning core queue/rating APIs.
