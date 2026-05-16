# Ratings Scheduling Method

This note documents how pair selection and rating estimation are done in `bgci ratings`.

## Goals

- Choose engine pairings with a paper-grounded criterion.
- Use online updates for fast live behavior.
- Periodically re-derive ratings from raw game history to reduce path dependence.

## Pair Scheduler (Information-Guided)

The scheduler uses a one-step expected information proxy from paired-comparison models
(Bradley-Terry / Elo logistic link) and adaptive paired-comparison design ideas.

For engines `i` and `j`:

- Expected win probability:

  `p_ij = 1 / (1 + 10^((R_j - R_i)/400))`

- Bernoulli information term (maximized near `p=0.5`):

  `I_outcome = p_ij * (1 - p_ij)`

- Posterior uncertainty mass (from Glicko-2 RD):

  `U_pair = RD_i^2 + RD_j^2`

- Pair score used by scheduler:

  `Score(i,j) = I_outcome * U_pair`

Interpretation:

- prefers close-strength pairings (`p` near 0.5)
- prefers uncertain engines (larger RD)
- naturally shifts to exploitation as RD shrinks

## Selection Rule

At each batch:

1. Compute `Score(i,j)` for all pairs.
2. If `--min-pair-games > 0`, first restrict to undercovered pairs (`games(i,j) < min_pair_games`).
3. Choose the pair with maximum score.
4. If scores tie, choose the pair with fewer prior games in this run.

This is deterministic and reproducible given the same run state.

## Persistence and Refit Hygiene

- Raw ratings games are persisted in `ratings_games`.
- Per-batch engine provenance signature is persisted in `ratings_batches`.
- Batch persistence is atomic (state + pair counts + raw rows in one transaction).
- Timeout/incomplete outcomes are excluded from score updates and global replay refit.

## Rating Estimation

`bgci ratings` uses Glicko-2 (`rating`, `RD`, `volatility`) for online updates during play.

In addition, raw game rows are persisted in SQLite (`ratings_games`) and a periodic
global refit can replay all raw results from scratch.

Refit cadence is game-count based (`--refit-every-games`), not batch-count based.

This gives a hybrid workflow:

- fast online updates for scheduling and live display
- periodic cumulative re-derivation from full raw history

This reduces drift and sensitivity to batch ordering while preserving responsiveness.

## Raw Outcomes

Each recorded game stores at least:

- `engine_a`, `engine_b`
- `points_a`, `points_b`
- winner/outcome labels from duel CSV
- `batch_idx` and insertion order

The cumulative replay uses all stored rows in deterministic order.

## Why This Improves Stability

- Online-only updates can be path-dependent (especially with adaptive scheduling).
- Periodic replay from full raw history re-anchors estimates to all evidence.
- With enough games, uncertainty contracts and rank flips between close engines become rarer.

## References

- Bradley, R.A., Terry, M.E. (1952), *Rank Analysis of Incomplete Block Designs: I. The Method of Paired Comparisons*.
- Zermelo, E. (1929), tournament strength estimation via maximum likelihood.
- Glickman, M.E., Jensen, S.T. (2005), *Adaptive paired comparison design*.
