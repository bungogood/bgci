# bgci

`bgci` is a lightweight UBGI duel runner for backgammon engines.

## Quick Start

Run the default duel config:

```bash
cargo run
```

Run a public matchup config:

```bash
cargo run -- --config config/gnubg-cli-vs-random.toml
```

## Public Duel Config Files

- `config/gnubg-cli-vs-random.toml`
- `config/gnubg-cli-vs-pubeval.toml`
- `config/pubeval-vs-random.toml`

Use `config/duel.toml` as a local default.

## Logging

Set `log` in a duel config:

- `off`: no duel log file
- `info`: protocol traffic and run metadata
- `debug`: includes engine stderr diagnostics

Results and logs are timestamped and written under `artifacts/duels/...`.

## Local/Private Configs

Put private configs under `config/local/` (gitignored).

## Docs

See `docs/dueller.md` and `docs/ubgi-v0.1-spec.md`.
