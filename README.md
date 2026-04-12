# bgci

bgci is a lightweight runner for the Universal Backgammon Interface (UBGI), a work-in-progress protocol for backgammon engine communication and control. It provides duel orchestration, logging, and result collection so you can quickly configure and run matches between different engines.

![bgci pubeval vs random](docs/pubeval-vs-random.gif)

UBGI is inspired by the chess Universal Chess Interface (UCI) and defines a simple, engine-agnostic protocol for exchanging moves, diagnostics, and match metadata. bgci implements UBGI's duel-management features, making it easy to set up tournaments, capture per-game traces, and export results. Based on early UBGI protocol work by Øystein Schønning-Johansen [here](https://github.com/oysteijo/Universal-Backgammon-Interface)

## Quick Start

`--config` is required. Run baseline example:

```bash
cargo run -- --config examples/pubeval-vs-random.toml
```

Run GNUbg adapter vs random:

```bash
cargo run -- --config examples/gnubg-cli-vs-random.toml
```

## Public Duel Configs

- `examples/pubeval-vs-random.toml` (baseline example)
- `examples/gnubg-cli-vs-random.toml`
- `examples/gnubg-cli-vs-pubeval.toml`

## Logging

Set `log` in a duel config:

- `off`: no duel log file
- `info`: protocol traffic and run metadata
- `debug`: includes engine stderr diagnostics

Results and logs are derived from datetime and engine names.

- output root: `data/<engine-a>-vs-<engine-b>/`
- files: `results-<timestamp>.csv`, `duel-<timestamp>.log`
- per-game traces: `games-<timestamp>/`

## Local/Private Configs

`config/` is gitignored. Copy any file from `examples/` into `config/` and edit locally.

## Docs

See `docs/ubgi-v0.1-spec.md`.

GNU Backgammon (GNUbg) adapter reference:

- `examples/gnubg-cli-vs-random.toml` and `examples/gnubg-cli-vs-pubeval.toml` use `gnubg_engine`, which adapts GNUbg's existing text CLI to UBGI.
- `https://www.gnu.org/software/gnubg/`
