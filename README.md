# bgci

`bgci` runs reproducible backgammon engine tests over UBGI.

The project is local-first. A duel is a reproducible two-engine benchmark made
of games arranged in mirrored two-game clusters when possible. Duels are ephemeral by default and `--save` records
them directly to a versioned SQLite database. Multi-engine leagues use the same
pair and game records. There is no CSV result pipeline.

See `docs/benchmark-v2-design.md` for the benchmark model and roadmap, and
`docs/selective-search-design.md` for the recommended engine search design.

## Install

```bash
cargo install --path crates/bgci-cli
```

The workspace contains:

- `crates/bgci-core`: UBGI engine execution, duel runner, and benchmark store
- `crates/bgci-cli`: the `bgci` command-line frontend

## Quick Start

```bash
# Ephemeral duel (200 games)
bgci duel --engine-a pubeval --engine-b random --games 200

# Export an ephemeral duel as a Jellyfish MAT money session
bgci duel --engine-a pubeval --engine-b random --games 20 --mat duel.mat

# Save the same duel as a named benchmark
bgci duel \
  --name new-evaluator \
  --engine-a "gnubg:ply=2" \
  --engine-b "gnubg:ply=1" \
  --games 200 \
  --parallel 4 \
  --save

# Fixed round-robin league (200 games per matchup)
bgci league \
  --name local-engines \
  --engines pubeval hureval pipcount random \
  --games-per-matchup 200 \
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
  --placement-games 40
```

Run a bounded amount of additional work:

```bash
bgci rank run main --budget-games 2000 --batch-games 40 --parallel 8
```

Omit `--budget-games` to run continuously. `Ctrl-C` requests a clean pause after
the current mirrored batch finishes. Running the same name continues its saved
state automatically:

```bash
bgci rank run main --batch-games 40 --parallel 8
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
marked with `*` until placement is complete and its approximate RD falls below
the pool's `--established-rd` threshold. Ratings are recomputed from immutable
saved games after every batch.

`--batch-games 40` runs 40 games before refitting and selecting again.
`--placement-opponents 3 --placement-games 40` requires useful connections to
three opponents, not 40 games against every engine in the pool.

The model is Bradley-Terry Elo over normalized game points. A game's points are
mapped from `[-3, +3]` to a score in `[0, 1]`, so gammons and backgammons affect
the rating direction and negative PPG cannot be treated as a winning result.
Reported RD comes from full pool-relative covariance calibrated with mirrored
mirrored groups as score clusters and finite-sample shrinkage toward model information.

Raw games are authoritative. Ranking statistics are derived directly from
accepted mirrored game rows. Inspect descriptive model residuals and graph
cycle coverage with:

```bash
bgci rank show main --diagnostics
```

These diagnostics are not significance tests; bootstrap calibration remains
required before claiming a matchup is genuinely non-transitive.

Local application state is stored in:

```text
$XDG_DATA_HOME/bgci/bgci.db
~/.local/share/bgci/bgci.db
```

Use `--db PATH` with `duel --save`, `league`, `rank`, or `history` to select
another database.

`duel --mat PATH` exports all duel games as one 0-point Jellyfish MAT money
session after a successful run. MAT export is independent of `--save`, records
moves only for that duel, and currently supports standard Backgammon only.
Money-session rules are controlled by the MAT reader; disable Jacoby there when
benchmark gammons and backgammons must retain their recorded values.

## Benchmark Semantics

Games are arranged into mirrored clusters. A full cluster contains two games
with the same deterministic dice seed and swapped engine sides. An odd request
ends with one deterministic side-balanced singleton. A saved duel has one
matchup. A league schedules every engine pairing and stores all results under
one ID.

Current summaries report games, wins, points, points per game, and uncertainty
from mirrored score clusters, including odd singletons. Rankings derive
Elo-style ratings from those scores; raw immutable games remain the source of
truth.

## Profiles

Profiles are loaded from `$XDG_CONFIG_HOME/bgci/config.toml` or
`~/.config/bgci/config.toml`:

```toml
[profiles.wildbg]
command = ["/path/to/wildbg", "--ubgi"]

[profiles.gnubg]
command = ["/path/to/gnubg", "--ubgi"]
```

Related profiles can share informational metadata while keeping independent
commands and transmitted UBGI settings:

```toml
[profiles.kestral-dmp-best]
family = "kestral"
version = "2026.08"
labels = { model = "dmp-best" }
command = ["/path/to/kestral", "--model", "/models/dmp-best.bin"]

[profiles.kestral-dmp-best.ubgi]
"engine.ply" = "1"

[profiles.kestral-light]
family = "kestral"
command = ["/path/to/kestral", "--model", "/models/dmp-light.bin"]

[profiles.kestral-light.ubgi]
"engine.ply" = "1"
```

`family`, `version`, and `labels` are informational metadata and never select a
profile or enter its canonical name. Profile lookup is an exact,
case-insensitive alias match. Values under `.ubgi` are sent to the engine.
Canonical names use the configured alias plus effective UBGI settings, such as
`kestral-dmp-best:ply=1`. Refresh metadata snapshots for a paused pool without
changing games or launch identity with:

```bash
bgci rank refresh main
```

After auditing engine-reported defaults, `--apply-ubgi` can make those
settings explicit in a paused pool. It refuses command/environment changes and
refuses changing any UBGI setting that was already explicit:

```bash
bgci rank refresh main --apply-ubgi
```

Canonical names are executable specifications, not display-only names. They
can be copied from ranking output into `duel`, `rank add`, or other engine-aware
commands. Inline suffixes override that exact profile's UBGI settings:

```bash
bgci duel \
  -a "hedgehog-aureus:ply=2,search=star2" \
  -b "hedgehog-star2:ply=2,search=star2" \
  --games 200
```

Inspect configured and built-in profiles with:

```bash
bgci engine --list
bgci check pubeval
```

## UBGI

The protocol reference used by this project is `docs/ubgi-v0.2-spec.md`.
`engine.ply` is root-inclusive: `1` evaluates the legal children of the supplied
known-dice position, while `2` also considers every opponent roll and best
reply. Chance nodes do not add plies. GNU Backgammon adapters translate these
values to GNU's native evaluation depth, where GNU 0-ply equals UBGI 1-ply.
