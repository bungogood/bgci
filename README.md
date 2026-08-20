# bgci

`bgci` runs reproducible backgammon engine tests over UBGI.

The project is local-first. A duel is a reproducible two-engine benchmark made
of mirrored game pairs. Duels are ephemeral by default and `--save` records
them directly to a versioned SQLite database. Multi-engine leagues use the same
pair and game records. There is no CSV result pipeline.

See `docs/benchmark-v2-design.md` for the model and roadmap.

## Install

```bash
cargo install --path crates/bgci-cli
```

The workspace contains:

- `crates/bgci-core`: UBGI engine execution, duel runner, and benchmark store
- `crates/bgci-cli`: the `bgci` command-line frontend

## Quick Start

```bash
# Ephemeral duel (100 mirror pairs / 200 games)
bgci duel --engine-a pubeval --engine-b random --pairs 100

# Save the same duel as a named benchmark
bgci duel \
  --name new-evaluator \
  --engine-a "gnubg:ply=2" \
  --engine-b "gnubg:ply=1" \
  --pairs 100 \
  --parallel 4 \
  --save

# Fixed round-robin league (100 mirror pairs per matchup)
bgci league \
  --name local-engines \
  --engines pubeval hureval pipcount random \
  --pairs-per-matchup 100 \
  --parallel 4

bgci history list
bgci history show 1
```

## Adaptive Rankings

Create the long-lived `main` ranking pool once:

```bash
bgci rank create main \
  --engines kestral gnubg pubeval random \
  --placement-opponents 3 \
  --placement-pairs 20
```

Run a bounded amount of additional work:

```bash
bgci rank run main --budget-pairs 1000 --batch-pairs 20 --parallel 8
```

Omit `--budget-pairs` to run continuously. `Ctrl-C` requests a clean pause after
the current mirrored batch finishes. Running the same name continues its saved
state automatically:

```bash
bgci rank run main --batch-pairs 20 --parallel 8
```

Add engines later as provisional members, without rebuilding the table:

```bash
bgci rank add main --engines kestral-dmp-best kestral-light
bgci rank list
bgci rank show main
bgci rank run main --parallel 8
```

The scheduler first places each provisional engine against a configurable
number of distinct opponents. It then selects matchups by expected information
using current rating uncertainty, predicted normalized score, and measured move
time. Engines averaging up to 50 ms per move have equal scheduling cost; above
that threshold, a square-root runtime penalty makes genuinely slow models play
less without letting cheap engines crowd out higher-uncertainty engines. An engine that
has sat out 20 batches is forced back into consideration, so very slow models
play less often after placement but are never permanently starved. An engine is
shown as established only after placement and after its approximate RD falls
below the pool's `--established-rd` threshold. Ratings are recomputed from
immutable saved games after every batch.

`--batch-pairs 20` runs 20 mirrored pairs (40 games) before refitting and
selecting again. `--placement-opponents 3 --placement-pairs 20` requires useful
connections to three opponents, not 20 pairs against every engine in the pool.

The model is Bradley-Terry Elo over normalized game points. A game's points are
mapped from `[-3, +3]` to a score in `[0, 1]`, so gammons and backgammons affect
the rating direction and negative PPG cannot be treated as a winning result.
Reported RD comes from full pool-relative covariance calibrated with mirrored
pairs as score clusters and finite-sample shrinkage toward model information.
The `tier` column groups engines from a common tier leader whose
rating-difference interval still overlaps at the working 95% level.

Raw games remain authoritative, while transactionally maintained per-matchup
and per-engine projections make routine fitting independent of total game
count. Existing schema-v1 databases are backfilled automatically. Inspect
descriptive model residuals and graph cycle coverage with:

```bash
bgci rank show main --diagnostics
```

These diagnostics are not significance tests; bootstrap calibration remains
required before claiming a matchup is genuinely non-transitive.

The default database is:

```text
$XDG_DATA_HOME/bgci/benchmarks.db
~/.local/share/bgci/benchmarks.db
```

Use `--db PATH` with `duel --save`, `league`, `rank`, or `history` to select
another database.

## Benchmark Semantics

A pair is the independent work unit. It contains two games with the same
deterministic dice seed and swapped engine sides. A saved duel has one matchup.
A league schedules every engine pairing and stores all results under one ID.

Current summaries report games, wins, points, points per game, and uncertainty
from completed mirror-pair scores. Elo-style ratings remain a future derived
projection; raw immutable games are the source of truth.

## Engine Aliases

Aliases are loaded from `$XDG_CONFIG_HOME/bgci/config.toml` or
`~/.config/bgci/config.toml`:

```toml
[engines.wildbg]
command = ["/path/to/wildbg", "--ubgi"]

[engines.gnubg]
command = ["/path/to/gnubg", "--ubgi"]
```

Related runnable configurations can share optional family metadata while
keeping independent commands and UBGI options:

```toml
[engines.kestral-dmp-best]
family = "kestral"
command = ["/path/to/kestral", "--model", "/models/dmp-best.bin"]

[engines.kestral-dmp-best.options]
"engine.ply" = "1"

[engines.kestral-light]
family = "kestral"
command = ["/path/to/kestral", "--model", "/models/dmp-light.bin"]

[engines.kestral-light.options]
"engine.ply" = "1"
```

Family is display and organization metadata. Every alias remains a separate
benchmark participant, and changing its family does not change launch identity.

Inspect configured and built-in engines with:

```bash
bgci engine --list
bgci check pubeval
```

## UBGI

The protocol reference used by this project is `docs/ubgi-v0.2-spec.md`.
