# bgci

`bgci` runs backgammon engine duels over UBGI.

![bgci pubeval vs random](docs/pubeval-vs-random.gif)

## Install

Clone the repo and run:

```bash
cargo install --path .
```

## Quick Start

```bash
bgci duel --engine-a pubeval --engine-b random --games 1000
bgci check pubeval  # check UBGI compatibility
bgci engine --list
```

By default, duel runs do not write CSV, MAT, or trace/log files.
Enable outputs explicitly with CLI flags or in config.

```bash
bgci duel --engine-a pubeval --engine-b random --games 1000 --output-csv results.csv
bgci duel --engine-a pubeval --engine-b random --games 1000 --output-mat results.mat
bgci duel --engine-a pubeval --engine-b random --games 1000 --output-traces traces/
bgci duel --engine-a pubeval --engine-b random --games 1000 --log info --output-log duel.log
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
```

## UBGI Protocol

bgci speaks UBGI (Universal Backgammon Interface), a UCI-inspired protocol for
engine communication.

Primary reference for this project:

- `docs/ubgi-v0.1-spec.md`

## References

- UBGI early protocol work: <https://github.com/oysteijo/Universal-Backgammon-Interface>
- GNU Backgammon: <https://www.gnu.org/software/gnubg/>
