# Selective Backgammon Search

This document describes a practical path from naive full-width expectimax to
stronger checker-play analysis at usable latency.

## Ply Convention

UBGI uses root-inclusive decision depth. A ply is one complete player turn. A
dice roll is a chance node and does not add a ply.

- 1-ply generates every legal move for the supplied position and dice, then
  evaluates each resulting position.
- 2-ply additionally considers every opponent roll and best reply.
- 3-ply additionally considers the original player's following roll and best
  move.

GNU Backgammon counts evaluation depth after applying the candidate root move,
so GNU 0-ply equals UBGI 1-ply, GNU 1-ply equals UBGI 2-ply, and so on.

## Why Naive Search Fails

There are approximately 20 legal moves per roll and 21 unordered dice outcomes.
A full-width search therefore grows roughly as follows:

```text
1-ply:  20 evaluations
2-ply:  20 x 21 x 20 = 8,400 evaluations
3-ply:  20 x 21 x 20 x 21 x 20 = 3,528,000 evaluations
```

Batching improves inference throughput but does not solve this branching
problem. Useful deeper analysis requires selective search.

## Recommended Search

Use a GNUbg-style selective expectimax:

1. Generate and statically evaluate every root move.
2. Retain only credible root candidates.
3. Enumerate all 21 opponent rolls for each retained candidate.
4. Use a cheap evaluator or policy to shortlist legal replies.
5. Select the likely best reply with the normal evaluator.
6. Recurse only from that selected reply.
7. Average every roll using its exact probability.
8. Reduce the root candidates again before searching another level.

A selective 3-ply search can progressively narrow the root:

```text
all root moves
  -> 1-ply evaluation
  -> retain about 12
  -> 2-ply evaluation
  -> retain about 4
  -> 3-ply evaluation
  -> choose the best move
```

At internal decision nodes:

```text
all legal replies
  -> cheap pruning or policy score
  -> retain about 6-10
  -> normal static evaluation
  -> follow the best reply
```

This is an approximation: the internal best reply is selected using a shallower
evaluation rather than backing up every reply through the complete remaining
subtree. The approximation makes deeper analysis practical.

## Root Move Filters

A filter should combine a candidate limit with an equity threshold:

- `accept`: number of moves retained unconditionally.
- `extra`: maximum number of additional moves considered.
- `threshold`: retain extra moves whose equity is within this distance of the
  current best move.

Initial values can be based on GNUbg's Normal filters:

| Target depth | Filtering depth | Maximum candidates | Equity threshold |
| ---: | ---: | ---: | ---: |
| 2 | 1 | 8-12 | 0.15-0.25 |
| 3 | 1 | 12-16 | 0.25-0.35 |
| 3 | 2 | 4-6 | 0.05-0.10 |

These values require calibration for each evaluator. Threshold filtering is
preferable to fixed top-K alone: obvious positions can retain one move, while
close decisions preserve more alternatives.

## Internal Move Pruning

Internal replies can be reduced with increasingly sophisticated methods:

1. Evaluate all replies statically and recurse through only the best few.
2. Use a smaller pruning network to shortlist replies before normal evaluation.
3. Add a policy head that predicts promising legal moves.
4. Use a dynamic shortlist such as `base + floor(log2(legal_moves))`.
5. Retain more candidates when evaluations are close or volatility is high.

A pruning model should optimize shortlist recall rather than exact equity:

> Does the shortlist contain the move selected by the expensive evaluator?

GNUbg uses dedicated pruning networks and normally retains approximately
`5 + floor(log2(legal_moves))` internal candidates. It then applies its normal
evaluator to that shortlist.

## Chance Nodes

All 21 unordered rolls should be included unless a sound bounded chance-search
algorithm is used. Their exact weights are:

- Doubles: `1/36` each.
- Non-doubles: `2/36` each.

GNUbg evaluates every roll. Its pruning consists of root move filters and
internal neural-network shortlists; it does not use alpha-beta, Star1, Star2, or
chance-outcome pruning.

Star1 or Star2 can be added when the engine has:

- Exact terminal values.
- Correct perspective inversion.
- Exact dice probabilities.
- Valid lower and upper bounds for every evaluator output.
- Strong move ordering.

Star pruning complements move filtering. It does not replace it.

## Inference Improvements

After fixing the search shape:

- Batch the legal moves for each roll into one model call.
- Batch independent roll branches where practical.
- Cache position evaluations within a search.
- Deduplicate transpositions before inference.
- Use exact bearoff databases and terminal values.
- Use phase-specific race, contact, or crashed evaluators when available.
- Keep model sessions and tensors reusable across searches.
- Avoid recursive allocation of board representations.

Selective branching generally provides a larger gain than these implementation
optimizations, but both are needed for competitive latency.

## Rollout Analysis

For analysis stronger than selective 3-ply, prefer rollouts over another naive
depth increase:

1. Use selective 2-ply or 3-ply to shortlist two to four root moves.
2. Run candidates with paired, identical dice streams.
3. Use a fast policy for rollout play.
4. Stop at a fixed horizon and evaluate the resulting positions.
5. Allocate more trials while candidates' confidence intervals overlap.

Paired dice reduce the variance of differences between candidate moves.

## Migration Plan

1. Adopt root-inclusive `engine.ply` semantics.
2. Correct terminal evaluation, perspective changes, and dice weighting.
3. Add progressive root move filters.
4. Replace full recursive internal expansion with shallow best-reply selection.
5. Batch model evaluations.
6. Add a pruning model or policy head.
7. Add Star bounds where evaluator bounds are valid.
8. Add paired adaptive rollouts for high-quality analysis.

## Engine Priorities

| Engine | Recommended next step |
| --- | --- |
| Hedgehog | Fix exhaustive terminal handling, use Star2 as the main selective search, and add progressive root filters. |
| WildBG | Add root filtering and shallow internal best-reply selection before increasing depth. |
| Hawk | Add root filtering and batch reply evaluation around its existing model. |
| Camlbot | Add progressive filtering and avoid full recursive expansion at 3-ply. |
| Kestral | Keep root-inclusive numbering and add GNU-style filters around the evaluator. |
| ai-backgammon | Replace full recursive expectimax with progressive selective expansion. |
| Tabula | Fix the `newgame` ply reset and chance weighting before optimizing search. |
| BGSage | Audit and tune its existing filters rather than replacing them. |

## Evaluation

Do not tune search only through engine-versus-engine results. Maintain a
checker-play corpus with strong GNUbg analysis or rollout references and report:

- Best-move agreement.
- Average equity loss.
- Worst-case equity loss.
- Best-move shortlist recall.
- Evaluated positions per decision.
- Median and p95 move latency.
- Search strength at fixed time budgets.

The target architecture is:

```text
strong static evaluator
+ progressive root filtering
+ cheap internal move pruning
+ exact 21-roll averaging
+ optional Star bounds
+ paired rollout analysis for close root moves
```

This structure should make useful 3-ply analysis possible without naive
multi-million-position searches.
