# bgci

`bgci` runs backgammon engine duels over UBGI.

![bgci pubeval vs random](docs/pubeval-vs-random.gif)

## Install

Clone the repo and run:

```bash
cargo install --path crates/bgci-cli
```

Workspace layout:

- `crates/bgci-core`: engine protocol, duel runner, game execution primitives
- `crates/bgci-cli`: `bgci` command-line frontend (`duel`, `check`, `engine`)
- `crates/bgci-ratings`: ratings DB + ingest + leaderboard + pairing scheduler
- `crates/bgci-ratings`: rating runner/orchestrator (`run`)

## Quick Start

```bash
bgci duel --engine-a pubeval --engine-b random --games 1000
bgci check pubeval  # check UBGI compatibility
bgci engine --list
```

## Important: User Engine Aliases

bgci supports XDG config and reads aliases from
`XDG_CONFIG_HOME` (e.g `~/.config/bgci/config.toml`).

Example:

```toml
[engines.wildbg]
command = ["/path/to/wildbg", "--ubgi"]

[engines.gnubg]
command = ["/path/to/gnubg", "--ubgi", "--pkgdatadir", "/path/to/share", "--datadir", "/path/to/share"]
```

References:

- GNUbg fork with native UBGI support: <https://github.com/bungogood/gnubg-ubgi>
- wildbg by Carsten Wenderdel: <https://github.com/carsten-wenderdel/wildbg>

Then you can duel aliases directly:

```bash
bgci duel --engine-a gnubg --engine-b wildbg --games 1000
```

## Useful Commands

```bash
# duel from config
bgci duel --config examples/pubeval-vs-random.toml

# check both engines in a config
bgci check --config examples/pubeval-vs-random.toml

# check one side from config
bgci check --config examples/pubeval-vs-random.toml a
bgci check --config examples/pubeval-vs-random.toml b

# record duel results into sqlite
bgci duel --engine-a pubeval --engine-b random --games 200 --record --db data/eval.db

# default DB path follows XDG:
# $XDG_DATA_HOME/bgci/eval.db (or ~/.local/share/bgci/eval.db)

# run an Elo search directly from an engine pool
bgci ratings --engines gnubg wildbg tabula pubeval --budget-games 2000 --pair-games 40 --parallel 8

# periodically replay all raw ratings games from DB for global refit
bgci ratings --engines gnubg wildbg tabula pubeval --budget-games 20000 --pair-games 200 --refit-every-games 4000

# enforce minimum pair coverage before pure information-gain scheduling
bgci ratings --engines gnubg wildbg tabula pubeval --budget-games 20000 --pair-games 200 --min-pair-games 400

# resume ratings state from DB (default DB follows XDG data path)
bgci ratings --engines gnubg wildbg tabula pubeval --budget-games 10000

# reset ratings state before running
bgci ratings --engines gnubg wildbg tabula pubeval --budget-games 2000 --reset

# show persisted leaderboard without running new games
bgci ratings --show

# hard reset all ratings tables
bgci ratings --reset-all

# run ratings with per-engine options as part of identity
bgci ratings --engines "gnubg:ply=1" "gnubg:ply=2" "wildbg:ply=1,top_k=8" --budget-games 4000

# run with a simple live terminal dashboard
bgci ratings --engines gnubg wildbg tabula pubeval --budget-games 2000 --pair-games 40 --parallel 4 --tui

# estimate rating of a candidate engine against rated pool (does not modify pool ratings)
bgci eval --engine hawk1 --budget-games 2000 --pair-games 200

# restrict eval to specific opponents from the ratings DB
bgci eval --engine hawk1 --opponents wildbg hureval pubeval --budget-games 2000

# equivalent standalone binary
bgci-ratings run --engines gnubg wildbg tabula pubeval --budget-games 2000 --pair-games 40 --parallel 8
```

## UBGI Protocol

bgci speaks UBGI (Universal Backgammon Interface), a UCI-inspired protocol for
engine communication.

Primary reference for this project:

- `docs/ubgi-v0.2-spec.md`

## References

- UBGI early protocol work: <https://github.com/oysteijo/Universal-Backgammon-Interface>
- GNU Backgammon: <https://www.gnu.org/software/gnubg/>
