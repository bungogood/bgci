# Scheduler Roadmap

## Goal

Support queued evaluation jobs that continue to completion even when the CLI process exits.

## Target Operating Modes

1. Local daemon mode: jobs run on this machine in a long-lived background service.
2. Remote backend mode: jobs run on remote workers managed by a scheduler service.

The `bgci` CLI should become a client that submits jobs and streams status, not the long-lived executor.

## Core Model

- Job: one duel/eval request (`run spec + metadata + status`).
- Task: one game shard within a job.
- Worker: process that executes tasks and reports progress.

Suggested states:

- `queued`
- `running`
- `cancelling`
- `completed`
- `failed`
- `cancelled`

## Minimal Interfaces

Keep interfaces transport-agnostic so local and remote implementations share one application flow.

```rust
pub struct EvalJobSpec {
    pub games: usize,
    pub parallel: usize,
    pub seed: u64,
    pub variant: String,
    pub max_plies: usize,
    pub engine_a: EngineConfig,
    pub engine_b: EngineConfig,
}

pub trait JobQueue {
    fn enqueue(&self, spec: EvalJobSpec) -> Result<String, String>;
    fn cancel(&self, job_id: &str) -> Result<(), String>;
    fn get(&self, job_id: &str) -> Result<JobStatus, String>;
}

pub trait EvaluationBackend {
    fn run_task(&self, task: TaskSpec) -> Result<TaskResult, String>;
}
```

## Incremental Plan

### Phase 1 (current codebase)

- Keep duel execution local.
- Keep `run_duel` as in-process orchestrator.
- Introduce clearer boundaries:
  - worker spawning/execution isolated (`src/duel_workers.rs`)
  - orchestrator remains in `src/duel_runner.rs`

### Phase 2 (local durable queue)

- Add a local daemon (`bgci serve`) with SQLite job store.
- Add CLI commands:
  - `bgci submit ...`
  - `bgci jobs list`
  - `bgci jobs show <id>`
  - `bgci jobs cancel <id>`
- CLI `duel` can optionally submit instead of running inline.

### Phase 3 (remote workers)

- Scheduler owns queue and shard assignment.
- Workers pull tasks and push heartbeats/results.
- Add retries, leases, and idempotent task result writes.

## Reliability Requirements

- Job metadata and progress persisted atomically.
- Task leasing with expiry to recover from worker death.
- Idempotent result ingestion by `(job_id, task_id)`.
- Graceful cancellation with bounded shutdown and force-kill fallback.

## Why this shape

This keeps the current fast subprocess execution path while evolving toward a scheduler architecture where your laptop process is optional.

## Near-Term Scope (Now)

What we should do now:

- Keep duel execution local and simple.
- Keep subprocess engines as the execution primitive.
- Improve code boundaries so scheduler work can be added without rewriting duel semantics.
- Add robust cancellation/timeout handling and better tests.

What we should not do now:

- Do not add remote transport code yet.
- Do not add SQLite job persistence yet.
- Do not introduce cluster orchestration concerns into the current CLI hot path.

## Commit Strategy

Prefer small commits grouped by concern:

1. runtime behavior changes (timeouts, cancellation semantics)
2. structural refactors (module splits, clearer ownership)
3. docs and roadmap notes

This keeps performance and correctness easy to validate at each step.
