# Benchmark V2 Design

## Purpose

`bgci` is a local-first backgammon engine testing tool. It supports two workflows
on one execution and storage model:

- A duel compares two engine builds and can optionally be saved.
- A league compares several engine builds and derives pool-scoped standings.
- A ranking pool repeatedly schedules informative matchups and can run until
  explicitly paused.

The system records reproducible experiments. It does not treat a mutable
leaderboard as primary data.

## Lessons From Chess Testing

OpenBench and fishtest separate an experiment definition, deterministic paired
work, game execution, and statistical analysis. Their distributed schedulers
scale that model but do not define it. Cutechess and fastchess make a repeated,
side-swapped game pair the basic unit of work.

For backgammon, games are grouped into mirror clusters: two games use the same
dice seed while the engines swap sides. An odd final game forms a singleton
cluster with a deterministically selected side. Statistical analysis treats
clusters, not individual games, as independent observations.

## Model

### Benchmark

An immutable resolved experiment manifest. Its kind is either `duel` or
`league`. It records the variant, rules, base seed, requested game count,
engine builds, UBGI settings, and software format versions.

### Engine Build

A resolved executable configuration, not merely a profile alias. Identity
includes the command, environment, UBGI settings, executable digest where
available, and model/data artifact digests in a future extension. Changing any
resolved input creates a different build identity.

### Matchup

A pairing of two engine builds within a benchmark. A saved duel has one matchup. A
league has one or more scheduled matchups.

### Cluster And Game

A cluster has a stable `pair_index` and deterministic seed. Full clusters have
two game rows with swapped sides; an odd final cluster has one explicitly
scheduled leg. Retries and failures must never overwrite a conflicting result.

## Storage

SQLite is the authoritative local application store. There is one current
schema, identified by SQLite `user_version = 2`. Older versions are rejected;
there are no in-place migrations.

Core tables:

- `engine_builds`
- `benchmarks`
- `benchmark_engines`
- `matchups`
- `games`

Raw benchmark definitions and game results are authoritative. Standings,
ratings, confidence intervals, and progress summaries are derived directly
from accepted game rows.

There is no CSV ingestion or export path in the core. CLI output is for humans;
future machine output uses a versioned JSON representation of the same typed
records.

## Statistics

The first release uses a fixed number of games, mirrored in two-game clusters when possible. It reports:

- completed games
- game wins and losses
- normal, gammon, and backgammon distributions
- points per game and paired point differential
- pair-aware uncertainty

League ratings are scoped to one benchmark and derived only from its games.
An Elo-style Bradley-Terry win model can be reported alongside points-based
results, but it must not hide gammon and backgammon equity.

SPRT is deferred until a backgammon-specific paired model has been validated by
simulation. Chess pentanomial SPRT cannot be copied without validating its
assumptions.

## CLI Direction

```text
bgci duel --engine-a A --engine-b B --games 20
bgci duel --name change-123 --engine-a NEW --engine-b BASE --games 2000 --save
bgci league --name engines-2026 --engines A B C --games-per-matchup 1000
bgci rank create main --engines A B C
bgci rank add main --engines D
bgci rank list
bgci rank run main
bgci rank show main
bgci history list
bgci history show ID
```

`duel` always has benchmark-grade mirrored semantics. It remains ephemeral
unless `--save` is supplied. `league` always records directly to SQLite.

## Delivery Status

Implemented:

1. CSV and legacy global ratings persistence are removed.
2. The matchup runner returns typed, mirrored game records.
3. SQLite storage ingests results directly into one current schema.
4. Saved duels and leagues share one cluster-oriented executor and schema.
5. Run manifests are atomic and completion requires every requested game.
6. Reported uncertainty is cluster-robust and includes singleton clusters.
7. Adaptive ranking pools support coverage-first scheduling, information-guided
   batches, continuous execution, and pause/resume from SQLite.
8. Ranking queries aggregate normalized point scores and mirrored-pair
   covariance moments directly from raw games.
9. Ranking RD uses centered full covariance with finite-sample shrinkage toward
   model information and mirrored pairs as robust score clusters; descriptive
   edge residuals expose possible non-transitivity.

Next:

1. Add executable/model fingerprints when engine identities stabilize.
2. Persist accepted games incrementally and support resume/verification.
3. Add bootstrap calibration and confirmation scheduling for transitivity
   diagnostics.
4. Extend rankings with explicitly compatible cross-pool history when needed.
5. Validate sequential stopping before introducing SPRT.
6. Add durable or remote workers only after ingestion is idempotent.
