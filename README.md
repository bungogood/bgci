# bgci

bgci is a lightweight runner for the Universal Backgammon Interface (UBGI), a work-in-progress protocol for backgammon engine communication and control. It provides duel orchestration, logging, and result collection so you can quickly configure and run matches between different engines.

![bgci pubeval vs random](docs/pubeval-vs-random.gif)

UBGI is inspired by the chess Universal Chess Interface (UCI) and defines a simple, engine-agnostic protocol for exchanging moves, diagnostics, and match metadata. bgci implements UBGI's duel-management features, making it easy to set up tournaments, capture per-game traces, and export results. Based on early UBGI protocol work by Øystein Schønning-Johansen [here](https://github.com/oysteijo/Universal-Backgammon-Interface)

## Quick Start

Run baseline example from a duel config:

```bash
cargo run -- duel --config examples/pubeval-vs-random.toml
```

Run GNUbg adapter vs random:

```bash
cargo run -- duel --config examples/gnubg-cli-vs-random.toml
```

Run a duel directly without a duel config file:

```bash
cargo run -- duel --engine-a gnubg --engine-b pubeval --games 1000
```

Run an engine protocol check directly from an alias:

```bash
cargo run -- check --engine pubeval
```

Built-in engines can now be referenced directly in config using `engine`:

- `engine = "pubeval"`
- `engine = "random"`
- `engine = "gnubg-cli"`

Use `command = ["..."]` only for ad-hoc direct process commands.

You can also define reusable engine shortcuts in user config (`$XDG_CONFIG_HOME/bgci/config.toml` or `~/.config/bgci/config.toml`):

```toml
[engines.xg]
command = ["xg", "--ubgi"]

[engines.gnubg-local]
command = "gnubg-cli"
env = { BGCI_GNUBG_BIN = "/opt/homebrew/bin/gnubg" }
```

Then duel configs can stay lightweight:

```toml
[engine_a]
name = "xg"
engine = "xg"

[engine_b]
name = "pubeval"
engine = "pubeval"
```

Set `BGCI_CONFIG=/path/to/config.toml` to override the default user config path.

List all available engine aliases (built-ins + user config):

```bash
cargo run -- engine --list

# include source/command/env for each alias
cargo run -- engine --list --verbose
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

- `examples/gnubg-cli-vs-random.toml` and `examples/gnubg-cli-vs-pubeval.toml` use the built-in `gnubg-cli` adapter, which adapts GNUbg's existing text CLI to UBGI.
- `https://www.gnu.org/software/gnubg/`
