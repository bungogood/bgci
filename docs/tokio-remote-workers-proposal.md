# Tokio + Remote Workers Proposal for bgci

## Goals

- Prepare bgci for live dashboards, streaming telemetry, and distributed execution.
- Keep current local duel workflow stable while introducing better abstractions.
- Preserve deterministic benchmark behavior (seeds, pairing, side swaps).
- Make remote execution first-class without rewriting game logic.

## Why Change

Current duel execution is local and thread-based. It works well for single-host throughput, but future goals (dashboards, remote workers, richer run control) need:

- async networking and long-lived connections,
- clearer execution boundaries,
- durable run state and retry semantics,
- protocol-level contracts between coordinator and workers.

Tokio is a strong fit for orchestration, networking, and concurrent I/O.

## Non-Goals (Initial Phases)

- No rewrite of backgammon rules/evaluation logic.
- No immediate replacement of current local threading in one step.
- No hard dependency on a specific cloud provider.

## Architecture Overview

### Roles

1. **Coordinator (`bgci duel`)**
   - Builds full run plan.
   - Schedules game jobs.
   - Aggregates results and computes stats.
   - Produces final CSV/log/summary.

2. **Worker (`bgci-worker`)**
   - Executes assigned jobs (engine pair, seed, limits).
   - Returns structured results and optional trace payload.
   - Reports health/capabilities.

### Key Principle

Coordinator is source of truth for run state and statistics; workers are stateless executors.

## Core Abstractions

### 1) MatchPlan

Immutable unit of work for one game:

- `run_id`
- `game_id`
- `seed`
- `variant`
- `max_plies`
- `a_is_x`
- `engine_a_ref`
- `engine_b_ref`

### 2) MatchResult

Canonical outcome contract:

- `run_id`, `game_id`
- winner/outcome/points/plies
- move/decision timing summaries
- error details if failed
- metadata (`worker_id`, engine command hash, version)
- optional trace content reference

### 3) WorkerCapability

Worker-advertised capability surface:

- supported engine aliases
- max local concurrency
- CPU/memory tags
- optional labels (arch, host group)

### 4) Scheduler

Pluggable scheduler interface:

- local queue + workers
- remote-capability-aware scheduling
- retries with idempotency keys

### 5) ResultSink

Single output boundary for:

- csv rows
- trace persistence
- status updates for CLI/dashboard

## Execution Backends

Define an execution backend trait to keep duel logic stable:

- `LocalThreadBackend` (current model, incremental upgrades)
- `LocalTokioBackend` (async process/io orchestration)
- `RemoteBackend` (HTTP/WebSocket workers)

This allows gradual migration and A/B parity testing.

## Tokio Adoption Strategy

Use Tokio primarily for orchestration:

- process I/O multiplexing,
- networking,
- heartbeat/status streams,
- dashboard/event fan-out.

Do **not** tie core evaluation/game semantics to async runtime.

## Remote Worker Protocol (v1)

Prefer HTTP+JSON first for simplicity and debuggability.

### Endpoints

- `GET /health`
- `GET /capabilities`
- `POST /jobs` (submit one game)
- `POST /heartbeat` (worker -> coordinator optional)

### Job Contract

`POST /jobs` request includes full `MatchPlan` and idempotency key:

- `idempotency_key = run_id + game_id`

Response includes:

- accepted/rejected reason,
- result payload if synchronous, or job handle if async mode later.

### Retry Safety

Coordinator may retry on timeout/network errors; workers must deduplicate by idempotency key.

## Determinism and Fairness

To keep benchmarking valid:

- Seed derivation happens at coordinator (`seed_for_game`).
- Pairing/side-swaps happen centrally.
- Worker does not decide scheduling-sensitive randomness.
- Result includes engine/version/command fingerprint for reproducibility.

## Observability

Introduce structured run events:

- `RunStarted`, `GameQueued`, `GameStarted`, `GameFinished`, `GameFailed`, `RunFinished`

Event stream consumers:

- CLI status renderer
- future dashboard (WebSocket/SSE)
- optional persistent event log for replay

## Security and Safety

For remote mode:

- token auth initially, mTLS later,
- per-worker process limits,
- max runtime per game,
- constrained allowed engine aliases/commands,
- optional sandboxing/containerization.

## Failure Model

Expected failures:

- worker unreachable,
- engine crash/hang,
- malformed result,
- partial run interruption.

Coordinator behavior:

- retry with bounded attempts,
- mark terminal failures per game,
- continue run unless policy says fail-fast,
- support resume by persisting completed game ids.

## Proposed Module Layout

Suggested incremental layout:

- `src/domain/` (run plan, result types, stats model)
- `src/executor/` (backend trait + local backend)
- `src/scheduler/` (queueing and assignment)
- `src/worker_api/` (HTTP contracts and client)
- `src/events/` (event bus + sinks)
- `src/dashboard/` (future live UI hooks)

## Migration Plan

### Phase 1: Internal Refactor (No Network)

- Introduce `MatchPlan`/`MatchResult`/`ExecutionBackend`.
- Keep current local behavior and output format unchanged.
- Add parity tests against existing duel results.

### Phase 2: Tokio Local Orchestration

- Add optional Tokio runtime for local execution path.
- Keep thread backend available as fallback.
- Validate throughput + deterministic parity.

### Phase 3: Remote Worker MVP

- Build `bgci-worker` with HTTP API (`health`, `capabilities`, `jobs`).
- Add coordinator remote backend.
- Run mixed local+remote workers in one duel.

### Phase 4: Live Dashboard + Streaming

- Add event streaming endpoint (SSE/WebSocket).
- Build dashboard view for in-flight runs.
- Expose per-worker and per-engine telemetry.

### Phase 5: Reliability + Scale

- Resume support and durable run state.
- Retry/idempotency hardening.
- Capability-aware scheduling and backpressure tuning.

## CLI Direction

Future-friendly command shape:

- `bgci duel ... --backend local|remote|auto`
- `bgci worker serve --bind 0.0.0.0:PORT`
- `bgci duel ... --worker http://host:port` (repeatable)
- `bgci duel ... --parallel N` remains valid in local mode

## Open Questions

1. Should remote workers write traces locally and upload references, or upload full trace text?
2. Do we require strict fail-fast mode for CI (abort on first failed game)?
3. Should engine aliases be resolved at coordinator only, or allow worker-local alias resolution?
4. What is the minimal auth model acceptable for first remote rollout?

## Recommendation

Start with Phase 1 immediately (abstractions + backend trait), then Phase 3 remote MVP using HTTP+JSON while retaining local path. This minimizes risk and keeps momentum toward dashboards and distributed benchmarking.
