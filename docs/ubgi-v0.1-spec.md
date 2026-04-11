# UBGI v0.1 Draft Spec

UBGI (Universal Backgammon Interface) is a line-based text protocol for GUI/dueller-to-engine communication.

This draft focuses on the mandatory core: **best action selection**.

## 1. Design Principles

- UCI-like handshake and readiness model.
- Thin engine protocol: GUI/dueller owns match management and orchestration.
- One command in flight at a time (v0.1).
- Text over stdin/stdout for maximum portability.
- Determinism should be controllable via options.

## 2. Transport

- UTF-8 text, one command per line.
- Commands and response keywords are case-sensitive.
- Engine must flush stdout after each response line.
- Unknown command must return an `error` line and continue.

## 3. Lifecycle

Typical sequence:

1. `ubgi`
2. `setoption ...` (zero or more)
3. `isready`
4. `newgame` and context commands
5. `position` + `dice` + `go`
6. `quit`

## 4. Commands (GUI/Dueller -> Engine)

### Handshake

- `ubgi`
- `isready`
- `quit`

### Configuration

- `setoption name <Name> value <Value>`

Recommended common options:

- `Threads` (int)
- `Seed` (int)
- `Deterministic` (bool)
- `EvalMode` (`cubeless|cubeful`)

### Session Context

- `newgame`
- `newsession type <money|match> [length <N>] [jacoby <on|off>] [crawford <on|off>]`
- `setscore <p0> <p1>`
- `setcube value <N> owner <center|p0|p1>`
- `setturn <p0|p1>`

### Position and Dice

- `position gnubgid <GNU_POSITION_ID>` (mandatory)
- `position xgid <XGID_STRING>` (optional)
- `dice <d1> <d2>`

### Decision Request

- `go role <chequer|cube|turn> [movetime <ms>] [depth <n>] [nodes <n>] [trials <n>]`

Notes:

- `chequer`: choose checker play using current board + dice.
- `cube`: choose `double` or `nodouble`.
- `turn`: full turn decision policy.

### Optional Interrupt

- `stop` (optional in v0.1, recommended for engines that support long analysis)

## 5. Responses (Engine -> GUI/Dueller)

### Handshake and State

- `id name <EngineName>`
- `id author <Author>`
- `option name <Name> type <spin|check|string|combo> ...`
- `ubgiok`
- `readyok`

### Info (optional streaming)

- `info <key> <value> ...`

Suggested keys: `role`, `time_ms`, `nodes`, `nps`, `depth`, `eq`, `mwc`, `pwin`, `pgammon`, `pbg`, `pv`.

### Final Answers

- `bestmove <payload>`
- `bestmoveid <GNU_POSITION_ID>` (recommended extension for duellers)
- `bestcube <double|nodouble>`
- `bestturn cube=<double|nodouble> moveid=<GNU_POSITION_ID>`
- optional: `eval eq <f> mwc <f> pwin <f> pgammon <f> pbg <f>`

### Errors

- `error <code> <message>`

Suggested codes:

- `unknown_command`
- `bad_argument`
- `missing_context`
- `unsupported_feature`
- `search_in_progress`

## 6. Move Representation

UBGI v0.1 supports two payload styles to keep both GUI and dueller workflows practical.

### 6.1 `bestmove` (core)

- For GUI-facing compatibility, engines should provide `bestmove`.
- Preferred text shape is space-separated `from/to` tokens (example: `24/18 13/12`).

### 6.2 `bestmoveid` (dueller extension)

- For deterministic harness validation, engines may provide `bestmoveid <gnubgid>`.
- Duellers can match this directly against legal child positions.

If both are present, they must represent the same action.

## 7. Determinism and Benchmarking

For reproducible duels, GUI/dueller should:

- Set deterministic option and fixed seed.
- Provide explicit dice.
- Use fixed compute budget (`movetime`, `nodes`, or `trials`).

## 8. Compliance Levels

### Level 0 (playable core)

- `ubgi`, `isready`, `newgame`, `position gnubgid`, `dice`, `go role chequer`, `bestmove`, `quit`

### Level 1 (dueller-ready)

- Level 0 plus cube actions, deterministic options, and `bestmoveid`

### Level 2 (analysis-friendly)

- Level 1 plus `info` streaming and optional `stop`

## 9. Open Items for v0.2

- Canonical XGID profile and exact parsing rules.
- Standardized analysis/rollout request and result schemas.
- Capability negotiation command for optional feature discovery.
- Conformance test corpus and protocol test harness.
