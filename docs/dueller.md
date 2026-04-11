# UBGI Dueller (v0.1)

This project includes a minimal dueller runner that can pit two UBGI engines against each other.

Engine reply compatibility for chequer decisions:

- accepted: `bestmoveid <gnubgid>`
- rejected: `bestmove <payload>`

## Config

Edit `config/duel.toml` or use one of the public duel configs in `config/`.

Important fields:

- `games`: number of games.
- `seed`: RNG seed for dice stream.
- `swap_sides`: alternate X/O assignments each game.
- `variant`: currently `backgammon`.
- `log`: one of `off`, `error`, `warn`, `info`, `debug`, `trace`.
- `engine_a.command` / `engine_b.command`: command arrays used to spawn engines.

## Run

```bash
cargo run
```

This runs the default `bgci` binary (the dueller).

To run GNUbg CLI adapter against random:

```bash
cargo run -- --config config/gnubg-cli-vs-random.toml
```

To run GNUbg CLI adapter against pubeval:

```bash
cargo run -- --config config/gnubg-cli-vs-pubeval.toml
```

To run pubeval against random:

```bash
cargo run -- --config config/pubeval-vs-random.toml
```

## Output

- Per-game console summary.
- Timestamped CSV at configured `output_csv` base path.
- Timestamped duel log file (if `log != "off"`) near the CSV output.

CSV columns:

- `game`
- `engine_x`
- `engine_o`
- `winner`
- `outcome`
- `points_x`
- `points_o`
- `points_a`
- `points_b`
- `plies`

## Reference Engine

For testing, a minimal random UBGI engine is included:

```bash
cargo run --bin random_engine
```

The default duel config runs `random_engine` for both sides.

## Logging

- `info`: protocol-level tracing (`-> engine`, `<- engine`) plus run metadata.
- `debug`: includes additional engine stderr diagnostics.
- `error`: protocol errors only.
