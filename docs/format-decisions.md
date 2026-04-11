# UBGI Format Decisions (v0.1)

## Goals

- Keep parsing dead simple for engine authors.
- Allow immediate implementation with existing open-source engines/libraries.
- Keep room for richer GUI workflows later.

## Community Input Summary

Key points from community discussion:

- Start with a thin protocol, UCI-like.
- Mandatory core should be "find best action".
- Single in-flight request is preferred for v0.x.
- Move format should be fixed/simple.
- Board format should be one canonical choice if possible, optional alternatives later.
- Error handling should use `error` (not `sorry`).

## Board Format Options

### Option A: GNUbg Position ID (14-char)

Pros:

- Already used in `bkgm` and many open tools.
- Compact and efficient.
- Easy for dueller workflows where match state is managed externally.

Cons:

- Less human-readable than XGID.
- Does not include full match/cube context by itself.

### Option B: XGID

Pros:

- Human-readable and commonly recognized by players.
- Carries more context in one string.

Cons:

- More parsing complexity and edge-case handling.
- Not currently native in our `bkgm` stack.

## v0.1 Decision

For **UBGI v0.1**:

- Canonical board command is `position gnubgid <id>`.
- Optional support for `position xgid <id>` is allowed but not required.
- Match/cube/session context is sent via explicit commands (`newsession`, `setscore`, `setcube`, `setturn`).

Rationale: this gives us a working dueller/protocol now, using current Rust tooling, while preserving a clear migration path to broader GUI interoperability.

## Move Format Decision

For **UBGI v0.1**:

- Core response is `bestmove` with simple text move tokens (e.g. `24/18 13/12`).
- Dueller extension is `bestmoveid <gnubgid>` for exact legality matching.
- Engines used in duellers should emit both when possible.

Rationale:

- Aligns with community preference for human-readable moves in GUI workflows.
- Keeps deterministic/fast benchmarking practical for harnesses.

## Compatibility Strategy

- v0.1 strict core for interoperability.
- Adapters may convert native engine formats to/from UBGI text.
- v0.2 can define equivalent canonicality for XGID if adoption demands it.
