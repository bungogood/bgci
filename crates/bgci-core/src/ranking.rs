//! Rating fitting and matchup selection for engine ranking pools.

use std::time::Duration;

const ELO_TO_LOG_ODDS: f64 = std::f64::consts::LN_10 / 400.0;
const PRIOR_RD: f64 = 300.0;
const MIN_RD: f64 = 30.0;
const MAX_RD: f64 = 350.0;
const MAX_ITERATIONS: usize = 1_000;
const DAMPING: f64 = 0.5;
const MAX_IDLE_BATCHES: usize = 20;

/// The result of one ranking game.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RankingGame {
    pub engine_a: usize,
    pub engine_b: usize,
    pub a_won: bool,
}

/// An engine's fitted Elo rating and approximate rating deviation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rating {
    pub index: usize,
    pub elo: f64,
    pub rd: f64,
    pub games: usize,
}

/// Fits Bradley-Terry ratings, centered at 1500 Elo.
///
/// Games with out-of-range or identical engine indices are ignored. The
/// Gaussian prior makes disconnected and undefeated pools finite and keeps
/// engines without games at the pool mean.
pub fn fit_ratings(engine_count: usize, games: &[RankingGame]) -> Vec<Rating> {
    if engine_count == 0 {
        return Vec::new();
    }

    let valid_games = games
        .iter()
        .filter(|game| {
            game.engine_a < engine_count
                && game.engine_b < engine_count
                && game.engine_a != game.engine_b
        })
        .copied()
        .collect::<Vec<_>>();
    let mut games_per_engine = vec![0_usize; engine_count];
    for game in &valid_games {
        games_per_engine[game.engine_a] += 1;
        games_per_engine[game.engine_b] += 1;
    }

    // Optimize in natural log-odds units; this avoids poorly scaled Elo
    // gradients while retaining a direct Bradley-Terry parameterization.
    let prior_variance = (PRIOR_RD * ELO_TO_LOG_ODDS).powi(2);
    let prior_precision = prior_variance.recip();
    let mut strengths = vec![0.0_f64; engine_count];

    for _ in 0..MAX_ITERATIONS {
        let mut gradient = strengths
            .iter()
            .map(|strength| -strength * prior_precision)
            .collect::<Vec<_>>();
        let mut information = vec![prior_precision; engine_count];

        for game in &valid_games {
            let probability = logistic(strengths[game.engine_a] - strengths[game.engine_b]);
            let result = f64::from(game.a_won);
            let residual = result - probability;
            let fisher = probability * (1.0 - probability);

            gradient[game.engine_a] += residual;
            gradient[game.engine_b] -= residual;
            information[game.engine_a] += fisher;
            information[game.engine_b] += fisher;
        }

        let mut largest_step = 0.0_f64;
        for index in 0..engine_count {
            let step = (DAMPING * gradient[index] / information[index]).clamp(-0.5, 0.5);
            strengths[index] += step;
            largest_step = largest_step.max(step.abs());
        }

        let mean = strengths.iter().sum::<f64>() / engine_count as f64;
        for strength in &mut strengths {
            *strength -= mean;
        }

        if largest_step < 1e-10 {
            break;
        }
    }

    let mut information = vec![prior_precision; engine_count];
    for game in &valid_games {
        let probability = logistic(strengths[game.engine_a] - strengths[game.engine_b]);
        let fisher = probability * (1.0 - probability);
        information[game.engine_a] += fisher;
        information[game.engine_b] += fisher;
    }

    strengths
        .into_iter()
        .enumerate()
        .map(|(index, strength)| Rating {
            index,
            elo: 1500.0 + strength / ELO_TO_LOG_ODDS,
            rd: (information[index].recip().sqrt() / ELO_TO_LOG_ODDS).clamp(MIN_RD, MAX_RD),
            games: games_per_engine[index],
        })
        .collect()
}

fn select_information_pair(
    ratings: &[Rating],
    pair_counts: &[Vec<usize>],
    average_decision_time: &[Option<Duration>],
    last_played_batch: &[Option<usize>],
    next_batch: usize,
) -> Option<(usize, usize)> {
    if ratings.len() < 2
        || ratings
            .iter()
            .any(|rating| !rating.elo.is_finite() || !rating.rd.is_finite() || rating.rd < 0.0)
    {
        return None;
    }

    let max_index = ratings.iter().map(|rating| rating.index).max()?;
    if pair_counts.len() <= max_index
        || pair_counts.iter().any(|row| row.len() <= max_index)
        || average_decision_time.len() <= max_index
        || last_played_batch.len() <= max_index
    {
        return None;
    }

    let mut ordered = ratings.iter().collect::<Vec<_>>();
    ordered.sort_unstable_by_key(|rating| rating.index);
    if ordered
        .windows(2)
        .any(|pair| pair[0].index == pair[1].index)
    {
        return None;
    }
    for (position, left) in ordered.iter().enumerate() {
        for right in &ordered[position + 1..] {
            if pair_counts[left.index][right.index] != pair_counts[right.index][left.index] {
                return None;
            }
        }
    }

    let mut known_costs = ordered
        .iter()
        .filter_map(|rating| average_decision_time[rating.index])
        .map(|cost| cost.as_secs_f64())
        .collect::<Vec<_>>();
    known_costs.sort_by(f64::total_cmp);
    let typical_cost = known_costs
        .get(known_costs.len() / 2)
        .copied()
        .unwrap_or(1.0)
        .max(1e-9);
    let engine_cost = |index: usize| {
        average_decision_time[index]
            .map(|cost| cost.as_secs_f64())
            .unwrap_or(typical_cost)
            .max(typical_cost * 0.01)
    };
    let forced_engine = ordered
        .iter()
        .filter_map(|rating| {
            let idle = last_played_batch[rating.index]
                .map_or(next_batch.saturating_add(1), |last| {
                    next_batch.saturating_sub(last)
                });
            (idle >= MAX_IDLE_BATCHES).then_some((idle, rating.index))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
        .map(|(_, index)| index);

    let mut information_choice: Option<(f64, usize, usize, usize)> = None;
    for (position, left) in ordered.iter().enumerate() {
        for right in &ordered[position + 1..] {
            if forced_engine.is_some_and(|index| index != left.index && index != right.index) {
                continue;
            }
            let probability = logistic((left.elo - right.elo) * ELO_TO_LOG_ODDS);
            let uncertainty = left.rd.mul_add(left.rd, right.rd * right.rd);
            let information = probability * (1.0 - probability) * uncertainty;
            let score = information / (engine_cost(left.index) + engine_cost(right.index));
            let count = pair_counts[left.index][right.index];
            let candidate = (score, count, left.index, right.index);
            if information_choice.is_none_or(|best| {
                candidate.0 > best.0
                    || (candidate.0 == best.0
                        && (candidate.1 < best.1
                            || (candidate.1 == best.1
                                && (candidate.2, candidate.3) < (best.2, best.3))))
            }) {
                information_choice = Some(candidate);
            }
        }
    }

    information_choice.map(|(_, _, left, right)| (left, right))
}

/// Selects placement games for provisional engines before information scheduling.
pub fn select_pair_for_pool(
    ratings: &[Rating],
    pair_counts: &[Vec<usize>],
    average_decision_time: &[Option<Duration>],
    last_played_batch: &[Option<usize>],
    next_batch: usize,
    placement_opponents: usize,
    placement_pairs: usize,
) -> Option<(usize, usize)> {
    let information_choice = select_information_pair(
        ratings,
        pair_counts,
        average_decision_time,
        last_played_batch,
        next_batch,
    )?;
    if placement_opponents == 0 || placement_pairs == 0 {
        return Some(information_choice);
    }
    let required_opponents = placement_opponents.min(ratings.len().saturating_sub(1));
    let qualified = ratings
        .iter()
        .map(|rating| {
            ratings
                .iter()
                .filter(|other| {
                    other.index != rating.index
                        && pair_counts
                            .get(rating.index)
                            .and_then(|row| row.get(other.index))
                            .is_some_and(|count| *count >= placement_pairs)
                })
                .count()
        })
        .collect::<Vec<_>>();

    let mut choice: Option<(usize, usize, f64, usize, usize)> = None;
    for (left_position, left) in ratings.iter().enumerate() {
        for (right_position, right) in ratings.iter().enumerate().skip(left_position + 1) {
            let count = *pair_counts.get(left.index)?.get(right.index)?;
            if count != *pair_counts.get(right.index)?.get(left.index)? || count >= placement_pairs
            {
                continue;
            }
            let left_needs = qualified[left_position] < required_opponents;
            let right_needs = qualified[right_position] < required_opponents;
            let needs = usize::from(left_needs) + usize::from(right_needs);
            if needs == 0 {
                continue;
            }
            let probability = logistic((left.elo - right.elo) * ELO_TO_LOG_ODDS);
            let uncertainty = left.rd.mul_add(left.rd, right.rd * right.rd);
            let information = probability * (1.0 - probability) * uncertainty;
            let left_cost = average_decision_time[left.index]
                .map_or(1.0, |cost| cost.as_secs_f64())
                .max(1e-9);
            let right_cost = average_decision_time[right.index]
                .map_or(1.0, |cost| cost.as_secs_f64())
                .max(1e-9);
            let utility = information / (left_cost + right_cost);
            let candidate = (needs, count, utility, left.index, right.index);
            if choice.is_none_or(|best| {
                candidate.0 > best.0
                    || (candidate.0 == best.0
                        && (candidate.1 < best.1
                            || (candidate.1 == best.1
                                && (candidate.2 > best.2
                                    || (candidate.2 == best.2
                                        && (candidate.3, candidate.4) < (best.3, best.4))))))
            }) {
                choice = Some(candidate);
            }
        }
    }
    choice
        .map(|(_, _, _, left, right)| (left, right))
        .or(Some(information_choice))
}

pub fn is_provisional(
    rating: &Rating,
    pair_counts: &[Vec<usize>],
    placement_opponents: usize,
    placement_pairs: usize,
    established_rd: f64,
) -> bool {
    let required_opponents = placement_opponents.min(pair_counts.len().saturating_sub(1));
    let opponents = pair_counts
        .get(rating.index)
        .map(|row| {
            row.iter()
                .enumerate()
                .filter(|(index, count)| *index != rating.index && **count >= placement_pairs)
                .count()
        })
        .unwrap_or(0);
    opponents < required_opponents || rating.rd > established_rd
}

fn logistic(log_odds: f64) -> f64 {
    if log_odds >= 0.0 {
        1.0 / (1.0 + (-log_odds).exp())
    } else {
        let odds = log_odds.exp();
        odds / (1.0 + odds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rating(index: usize, elo: f64, rd: f64) -> Rating {
        Rating {
            index,
            elo,
            rd,
            games: 0,
        }
    }

    fn counts(values: [[usize; 3]; 3]) -> Vec<Vec<usize>> {
        values.into_iter().map(Vec::from).collect()
    }

    #[test]
    fn stronger_engines_rank_above_weaker_engines() {
        let mut games = Vec::new();
        for _ in 0..40 {
            games.push(RankingGame {
                engine_a: 0,
                engine_b: 1,
                a_won: true,
            });
            games.push(RankingGame {
                engine_a: 0,
                engine_b: 2,
                a_won: true,
            });
            games.push(RankingGame {
                engine_a: 1,
                engine_b: 2,
                a_won: true,
            });
        }

        let ratings = fit_ratings(3, &games);

        assert!(ratings[0].elo > ratings[1].elo);
        assert!(ratings[1].elo > ratings[2].elo);
        assert!((ratings.iter().map(|rating| rating.elo).sum::<f64>() / 3.0 - 1500.0).abs() < 1e-9);
        assert_eq!(ratings[0].games, 80);
    }

    #[test]
    fn no_game_pool_is_centered_and_uncertain() {
        let ratings = fit_ratings(4, &[]);

        assert_eq!(ratings.len(), 4);
        for (index, rating) in ratings.iter().enumerate() {
            assert_eq!(rating.index, index);
            assert_eq!(rating.elo, 1500.0);
            assert!((rating.rd - PRIOR_RD).abs() < 1e-9);
            assert_eq!(rating.games, 0);
        }
    }

    #[test]
    fn disconnected_engines_stay_at_the_pool_mean() {
        let games = vec![RankingGame {
            engine_a: 0,
            engine_b: 1,
            a_won: true,
        }];
        let ratings = fit_ratings(4, &games);

        assert!(ratings[0].elo > 1500.0);
        assert!(ratings[1].elo < 1500.0);
        assert_eq!(ratings[2].elo, 1500.0);
        assert_eq!(ratings[3].elo, 1500.0);
        assert!((ratings[2].rd - PRIOR_RD).abs() < 1e-9);
        assert!((ratings[3].rd - PRIOR_RD).abs() < 1e-9);
    }

    #[test]
    fn invalid_games_are_ignored() {
        let games = [
            RankingGame {
                engine_a: 0,
                engine_b: 0,
                a_won: true,
            },
            RankingGame {
                engine_a: 0,
                engine_b: 9,
                a_won: true,
            },
        ];

        assert_eq!(fit_ratings(2, &games), fit_ratings(2, &[]));
        assert!(fit_ratings(0, &games).is_empty());
    }

    #[test]
    fn information_selection_avoids_a_gross_mismatch() {
        let ratings = [
            rating(0, 1100.0, 100.0),
            rating(1, 1500.0, 100.0),
            rating(2, 1550.0, 100.0),
        ];
        let pair_counts = counts([[0, 4, 4], [4, 0, 4], [4, 4, 0]]);

        assert_eq!(
            select_information_pair(&ratings, &pair_counts, &[None; 3], &[None; 3], 0),
            Some((1, 2))
        );
    }

    #[test]
    fn selection_ties_are_stable_by_engine_index() {
        let ratings = [
            rating(2, 1500.0, 100.0),
            rating(0, 1500.0, 100.0),
            rating(1, 1500.0, 100.0),
        ];
        let pair_counts = counts([[0, 3, 3], [3, 0, 3], [3, 3, 0]]);

        assert_eq!(
            select_information_pair(&ratings, &pair_counts, &[None; 3], &[None; 3], 0),
            Some((0, 1))
        );
    }

    #[test]
    fn information_selection_penalizes_slow_matchups() {
        let ratings = [
            rating(0, 1500.0, 100.0),
            rating(1, 1500.0, 100.0),
            rating(2, 1500.0, 100.0),
        ];
        let pair_counts = counts([[0, 3, 3], [3, 0, 3], [3, 3, 0]]);

        assert_eq!(
            select_information_pair(
                &ratings,
                &pair_counts,
                &[
                    Some(Duration::from_millis(10)),
                    Some(Duration::from_millis(10)),
                    Some(Duration::from_secs(1))
                ],
                &[Some(9); 3],
                10
            ),
            Some((0, 1))
        );
    }

    #[test]
    fn idle_engine_is_not_starved_by_runtime_cost() {
        let ratings = [
            rating(0, 1500.0, 100.0),
            rating(1, 1500.0, 100.0),
            rating(2, 1500.0, 100.0),
        ];
        let pair_counts = counts([[0, 3, 3], [3, 0, 3], [3, 3, 0]]);

        assert_eq!(
            select_information_pair(
                &ratings,
                &pair_counts,
                &[
                    Some(Duration::from_millis(10)),
                    Some(Duration::from_millis(10)),
                    Some(Duration::from_secs(1))
                ],
                &[Some(24), Some(24), Some(5)],
                25
            ),
            Some((0, 2))
        );
    }

    #[test]
    fn selection_rejects_malformed_inputs() {
        let duplicate = [rating(0, 1500.0, 100.0), rating(0, 1500.0, 100.0)];
        assert_eq!(
            select_information_pair(&duplicate, &[vec![0]], &[None], &[None], 0),
            None
        );

        let ratings = [rating(0, 1500.0, 100.0), rating(1, 1500.0, 100.0)];
        assert_eq!(
            select_information_pair(
                &ratings,
                &[vec![0, 1], vec![2, 0]],
                &[None; 2],
                &[None; 2],
                0
            ),
            None
        );
        assert_eq!(
            select_information_pair(&ratings[..1], &[vec![0]], &[None], &[None], 0),
            None
        );
    }

    #[test]
    fn placement_does_not_require_every_pair() {
        let ratings = [
            rating(0, 1500.0, 300.0),
            rating(1, 1500.0, 100.0),
            rating(2, 1500.0, 100.0),
            rating(3, 1500.0, 100.0),
        ];
        let pair_counts = vec![
            vec![0, 20, 20, 0],
            vec![20, 0, 0, 0],
            vec![20, 0, 0, 0],
            vec![0, 0, 0, 0],
        ];

        assert_eq!(
            select_pair_for_pool(&ratings, &pair_counts, &[None; 4], &[None; 4], 0, 2, 20),
            Some((1, 2))
        );
    }

    #[test]
    fn placement_spreads_games_before_deepening_a_matchup() {
        let ratings = [
            rating(0, 1500.0, 200.0),
            rating(1, 1500.0, 200.0),
            rating(2, 1500.0, 200.0),
        ];
        let pair_counts = counts([[0, 5, 0], [5, 0, 0], [0, 0, 0]]);

        assert_eq!(
            select_pair_for_pool(&ratings, &pair_counts, &[None; 3], &[None; 3], 0, 2, 10),
            Some((0, 2))
        );
    }

    #[test]
    fn provisional_status_uses_opponent_diversity_and_uncertainty() {
        let pair_counts = vec![vec![0, 20, 20], vec![20, 0, 0], vec![20, 0, 0]];
        assert!(!is_provisional(
            &rating(0, 1600.0, 70.0),
            &pair_counts,
            2,
            20,
            80.0
        ));
        assert!(is_provisional(
            &rating(1, 1500.0, 70.0),
            &pair_counts,
            2,
            20,
            80.0
        ));
        assert!(is_provisional(
            &rating(0, 1600.0, 100.0),
            &pair_counts,
            2,
            20,
            80.0
        ));
    }
}
