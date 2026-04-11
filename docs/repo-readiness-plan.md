# BGCI Repo Readiness Plan

## Goals

- Keep the repo portable and public-ready.
- Reduce complexity in the duel CLI.
- Keep engine debugging easy for contributors.

## Phase 1: Structure and Complexity

- Split orchestration from business logic.
- Move duel loop and game execution out of `src/main.rs`.
- Move stats accumulation into a dedicated module.
- Move timestamp/path generation into a dedicated module.

## Phase 2: Logging Clarity

- Keep `log = "info"` as protocol-level tracing:
  - commands sent to engine
  - responses from engine
  - protocol errors
- Keep `log = "debug"` for additional engine stderr diagnostics.
- Keep logs file-based with timestamped filenames near duel outputs.

## Phase 3: Portability Cleanup

- Remove explicit machine-local defaults (for example `/home/<user>/...`).
- Use portable defaults and env overrides for local tools.

## Phase 4: Preset Curation (next pass)

- Keep only public duel config files:
  - `config/gnubg-cli-vs-random.toml`
  - `config/gnubg-cli-vs-pubeval.toml`
  - `config/pubeval-vs-random.toml`
- Keep personal configs in `config/local/` (gitignored).

## Acceptance Criteria

- `src/main.rs` is small and readable.
- No explicit personal filesystem paths in tracked code.
- `cargo check` succeeds.
- Duel output and logs remain easy to find and understand.
