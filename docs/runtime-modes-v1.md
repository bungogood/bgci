# Runtime Modes v1

## Problem

`bgci duel` currently means "run a benchmark now." For many local runs, users do not want durable run metadata, run ids, leases, or queue state.

At the same time, distributed execution needs durable storage and resumable orchestration.

## Decision

Define two explicit modes:

1. **Ephemeral mode (default)**
   - Command: `bgci duel ...`
   - No DB-backed run/job state.
   - Produces output artifacts (csv/log/traces) only.
   - Fast path for local experimentation.

2. **Managed mode (opt-in)**
   - Command: `bgci duel ... --managed`
   - Writes run/jobs/results/events to DB.
   - Prints `run_id` on start.
   - Enables resume, retries, and remote worker orchestration.

## CLI Shape (Proposed)

### Ephemeral local run

```bash
bgci duel --engine-a hureval --engine-b wildbg --games 1000 --parallel 8
```

### Managed local run

```bash
bgci duel --engine-a hureval --engine-b wildbg --games 1000 --parallel 8 --managed
```

### Managed run with explicit DB

```bash
bgci duel --config match.toml --managed --db ~/.local/state/bgci/runs.db
```

### Submit to detached server

```bash
bgci submit match.toml --server http://coordinator:8080
```

## Flags (Proposed)

- `--managed`
  - Enables DB-backed run lifecycle.
  - If absent, no durable run records are written.

- `--db <path>`
  - SQLite path when managed mode is used.
  - Default: `~/.local/state/bgci/runs.db`

- `--run-name <name>`
  - Optional name for managed runs.

## Behavior Guarantees

### Ephemeral

- No durable queue state.
- No run id required.
- Failures reported directly to terminal.

### Managed

- Unique `run_id` generated and printed.
- Job states persisted (`queued`, `running`, `succeeded`, `failed`, etc).
- Safe resume via `bgci runs resume <run_id>`.

## Why Not Always Persist?

- Local one-off duels should stay frictionless.
- DB writes and lifecycle bookkeeping add overhead and conceptual complexity.
- Explicit mode separation prevents accidental persistence and keeps UX predictable.

## Future Extensions

- `--managed=local|remote` (if needed)
- `bgci runs watch <run_id>`
- `bgci runs export <run_id>`
