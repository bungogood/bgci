# Ratings Design Review TODOs

This list converts the latest critical methodology review into concrete implementation tasks.

## P0 (implemented now)

- [x] Add game-based refit cadence (`--refit-every-games`) to reduce batch-size sensitivity.
- [x] Persist raw ratings games in SQLite (`ratings_games`) for cumulative replay.
- [x] Add information-guided pairing score (`p(1-p) * (RD_i^2 + RD_j^2)`).

## P0 (implemented in this pass)

- [x] Enforce minimum pair coverage before pure information-gain exploitation.
- [x] Make ratings persistence atomic per batch (state + pair counts + raw rows).
- [x] Persist engine provenance metadata for each ratings batch.
- [x] Skip timeout/incomplete rows in online update and refit scoring.
- [x] Report conservative score (`rating - 2*RD`) in leaderboard/final output.

## P1 (next)

- [ ] Replace scalar score mapping with categorical outcome likelihood (`normal/gammon/backgammon`).
- [ ] Add anchor policy (`--anchors`) and coverage constraints.
- [ ] Add calibration checks (predictive log loss / posterior predictive diagnostics).
- [ ] Add queue-backed ratings execution (local worker first, remote later).
