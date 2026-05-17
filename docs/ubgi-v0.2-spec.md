# UBGI v0.2 Spec

UBGI (Universal Backgammon Interface) is a line-based text protocol for controller-to-engine communication.

This version is intentionally minimal:

- easy to implement in any language,
- flexible enough for rich UI configuration,
- strict enough for reliable engine-vs-engine support.

## 1. Transport

- UTF-8 text, one command per line.
- Keywords are case-sensitive.
- Engine must flush stdout after each response line.
- Unknown input must return an `error` line and continue.

## 2. Lifecycle

Typical sequence:

1. `ubgi`
2. `set ...` (zero or more)
3. `isready`
4. `newgame`
5. `position ...` + `dice ...` + `go ...`
6. `quit`

## 3. Commands (Controller -> Engine)

### Handshake

- `ubgi`
- `isready`
- `quit`

### Session

- `newgame`

### Position and Dice

- `position gnubgid <GNU_POSITION_ID>` (required)
- `position xgid <XGID_STRING>` (optional)
- `dice <d1> <d2>`

### Decision Request

- `go`
- `go chequer`
- `go cube`
- `go turn`

### Configuration

- `set <key> <value>`
- `get <key>`
- `keys`

Key naming convention:

- `game.*` for game/session context (example: `game.variant`)
- `engine.*` for engine behavior (example: `engine.ply`)

## 4. Responses (Engine -> Controller)

### Handshake and Readiness

- `id name <EngineName>`
- `id author <Author>`
- `proto 0.2`
- `ubgiok`
- `readyok`

### Key Discovery and Values

- `key <name> int [<min>..<max>] <default> [! ] [description...]`
- `key <name> bool <default> [! ] [description...]`
- `key <name> enum <a|b|...> <default> [! ] [description...]`
- `key <name> string * <default> [! ] [description...]`
- `value <name> <value>`

`!` is an optional minor-option marker declared by the engine.

- when `!` is present, the option is minor (controllers should usually exclude it from rating identity keys)
- when `!` is absent, the option is key-relevant by default

`!` is only valid as the standalone token immediately after `<default>`.

Type examples:

- `int 1..4 1`
- `int 42`
- `bool true`
- `enum backgammon|nackgammon|hypergammon backgammon`
- `string * default-value`

Examples:

- `key engine.ply int 1..4 1 search depth`
- `key engine.threads int 1..64 8 ! worker threads`
- `key engine.seed int * 42 ! rng seed`

### Optional Info

- `info <key> <value> ...`

Suggested keys: `nodes`, `time_ms`, `depth`, `pv`, `multipv`, `score`.

### Final Answers

- `bestmove <payload>`
- `bestcube <double|nodouble>`
- `bestturn cube=<double|nodouble> move=<payload>`

## 5. Minimal Error System

Error line format:

- `error <code> [detail...]`

Only four error codes are standardized:

- `bad_command` — unknown command or malformed syntax
- `bad_value` — parsed value is invalid for key/argument
- `bad_state` — command is valid but cannot be applied in current state
- `unsupported` — feature/key/format is not implemented by engine

Examples:

- `error bad_command expected: set <key> <value>`
- `error bad_value engine.ply must_be 1..4`
- `error bad_state missing position`
- `error unsupported go cube`

## 6. Move Representation

`bestmove` payload is space-separated `from/to` tokens.

- Example: `bestmove 24/18 13/12`
- `bar` represents bar entry.
- `off` represents bearing off.
- `pass` is used when no legal checker move exists.

## 7. Required v0.2 Playable Core

- `ubgi`, `isready`, `newgame`, `position gnubgid`, `dice`, `go` or `go chequer`, `bestmove`, `quit`
- `set`, `get`, `keys`
- minimal error system (`bad_command`, `bad_value`, `bad_state`, `unsupported`)

## 8. Compatibility Guidance

During migration, engines/controllers may support both:

- v0.1-style `setoption name <Name> value <Value>`
- v0.2-style `set <key> <value>`

When both are supported, v0.2 keys are preferred.
