# Match File Spec v1

## Purpose

`match.toml` is a portable run specification for managed runs and server submission.

It captures benchmark intent (engines, games, variant, policy) independent of local shell commands.

## Example

```toml
version = 1
name = "pubeval-vs-wildbg-10k"

games = 10000
seed = 42
parallel = 16
max_plies = 512
swap_sides = true
variant = "backgammon"

log = "info"
ubgi_log = "off"

[engine_a]
name = "pubeval"
engine = "pubeval"

[engine_b]
name = "wildbg"
engine = "wildbg"

[policy]
managed = true
fail_fast = false
max_attempts = 2
```

## Top-Level Fields

- `version` (required, integer)
- `name` (optional, string)
- `games` (required, integer > 0)
- `seed` (required, integer)
- `parallel` (optional, default `1`)
- `max_plies` (optional, default `512`)
- `swap_sides` (optional, default `true`)
- `variant` (optional, default `backgammon`)
- `log` (optional, default `off`)
- `ubgi_log` (optional, default `off`; `off|full|errors` target)

## Engine Sections

Two required sections:

- `[engine_a]`
- `[engine_b]`

Each supports:

- `name` (required, display name)
- `engine` (recommended alias reference)
- `command` (explicit command array/string)
- `env` (optional map)

Constraint: exactly one of `engine` or `command` should be set.

## Policy Section

Optional `[policy]`:

- `managed` (bool, default `false`)
- `fail_fast` (bool, default `false`)
- `max_attempts` (int, default `1`)
- `timeout_sec_per_game` (optional)

## Engine Resolution

Recommended managed-mode behavior:

1. Resolve aliases at coordinator submission time.
2. Store a concrete command/env snapshot in run metadata.
3. Workers execute snapshot directly.

This avoids drift from worker-local alias registries.

## Compatibility Notes

- Existing duel config files can be accepted as `match.toml` if they contain required fields.
- `version = 1` allows future schema migration.
