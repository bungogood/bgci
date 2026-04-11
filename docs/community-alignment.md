# Community Alignment Notes

This note compares the current `bgci` implementation direction with the public UBGI draft by Øystein and community feedback.

## Areas of Strong Alignment

- Protocol name is `UBGI`.
- UCI-like startup pattern (`ubgi`, `isready`, `readyok`).
- Thin protocol philosophy: UI/dueller owns orchestration.
- Single in-flight request model for v0.x.
- `error`-style failure replies.
- `newgame` and explicit `position` command.

## Deliberate v0.1 Compromises

- Keep `position gnubgid` mandatory for immediate Rust ecosystem integration.
- Allow `position xgid` as optional capability.
- Require `bestmoveid` in dueller mode for strict protocol framing.

## Migration Path

1. Keep v0.1 core stable and easy to implement.
2. Add canonical XGID profile in v0.2 if consensus solidifies.
3. Add analysis/rollout command families and stronger capability negotiation.

## Collaboration Guidance

- Treat Øystein's draft as upstream conceptual baseline.
- Keep this repo's implementation notes explicit when diverging.
- Prefer additive extensions over breaking command changes.
