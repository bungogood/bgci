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

The default database is:

```text
$XDG_DATA_HOME/bgci/benchmarks.db
~/.local/share/bgci/benchmarks.db
```

Use `--db PATH` with `duel --save`, `league`, or `history` to select another
database.

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

Inspect configured and built-in engines with:

```bash
bgci engine --list
bgci check pubeval
```

## UBGI

The protocol reference used by this project is `docs/ubgi-v0.2-spec.md`.
