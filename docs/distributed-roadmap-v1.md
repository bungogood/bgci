# Distributed Dueling Roadmap v1

## Principles

- Keep `bgci duel` fast and simple by default (ephemeral mode).
- Add durable orchestration only in managed/server mode.
- Use strict engine compatibility for distributed runs.
- Preserve deterministic game scheduling and seed derivation.

## Strict Compatibility Policy

Distributed runs use **strict** policy only.

A run is accepted only if all selected workers can satisfy required engines with matching capability fingerprints.

Required checks:

- Engine alias available on worker.
- UBGI handshake succeeds.
- `id name` and `id version` captured.
- Command fingerprint matches coordinator-resolved snapshot.

No `best-effort` or `compatible` mode in v1.

## Iteration Plan

### Iteration 1: Execution Abstraction (In Progress)

- Introduce `MatchPlan` domain object.
- Introduce `DuelExecutor` trait.
- Keep `LocalThreadExecutor` as default implementation.
- Ensure current behavior/output remains unchanged.

### Iteration 2: Managed Local Runs (SQLite)

- Add `--managed` mode for `duel`.
- Persist runs/jobs/results/events in SQLite.
- Print `run_id` and support `resume`.
- Keep default ephemeral mode unchanged.

### Iteration 3: Coordinator Server

- Add `bgci serve` with Tokio.
- Durable queue + lease model.
- Heartbeats, lease expiry, and requeue.

### Iteration 4: Remote Workers

- Add `bgci worker serve`.
- Worker capability registration.
- Strict preflight engine compatibility checks.

### Iteration 5: Live Monitoring

- Add streaming run events (SSE/WebSocket).
- Add dashboard-ready status endpoints.
- Add UBGI log indexing for per-game deep dives.

## Master/Client Failure Handling

- Client submits run to coordinator and may disconnect.
- Coordinator persists state and continues scheduling.
- Worker leases expire and are requeued on worker loss.
- Duplicate result submissions are deduplicated by `(run_id, game_id)`.

This guarantees runs continue even if a laptop/client dies.

## Current Status

- `MatchPlan` and `DuelExecutor` abstraction started.
- Local threaded backend remains active.
- Next step: wire managed mode and SQLite state transitions.
