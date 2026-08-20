//! Rating fitting and matchup selection for engine ranking pools.

use std::{collections::BTreeMap, time::Duration};

const ELO_TO_LOG_ODDS: f64 = std::f64::consts::LN_10 / 400.0;
const PRIOR_RD: f64 = 300.0;
const MAX_RD: f64 = 350.0;
const MAX_ITERATIONS: usize = 1_000;
const MAX_IDLE_BATCHES: usize = 20;
const ACCEPTABLE_MOVE_SECONDS: f64 = 0.05;
const ROBUST_COVARIANCE_PRIOR_PAIRS: f64 = 30.0;

/// Aggregated ranking observations for one pair of engines.
///
/// Each completed pair is a score cluster containing `m` rated games and a
/// total score for engine A. The final three fields are sums over those
/// clusters and allow cluster-robust covariance estimation without retaining
/// individual results.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RankingEdge {
    pub engine_a: usize,
    pub engine_b: usize,
    pub rated_games: usize,
    pub score_sum_a: f64,
    pub completed_pairs: usize,
    pub sum_m_squared: f64,
    pub sum_m_score: f64,
    pub sum_score_squared: f64,
}

/// An engine's fitted Elo rating and approximate rating deviation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rating {
    pub index: usize,
    pub elo: f64,
    pub rd: f64,
    pub games: usize,
}

/// Fitted ratings and their cluster-robust, sum-to-zero covariance in Elo².
#[derive(Clone, Debug, PartialEq)]
pub struct RatingModel {
    pub ratings: Vec<Rating>,
    pub covariance: Vec<Vec<f64>>,
    pub fit_diagnostics: FitDiagnostics,
}

/// Why fitting stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitReason {
    Converged,
    NoObservations,
    LinearSolveFailed,
    LineSearchFailed,
    MaximumIterations,
}

impl FitReason {
    pub const fn is_healthy(self) -> bool {
        matches!(self, Self::Converged | Self::NoObservations)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Converged => "converged",
            Self::NoObservations => "no observations",
            Self::LinearSolveFailed => "linear solve failed",
            Self::LineSearchFailed => "line search failed",
            Self::MaximumIterations => "maximum iterations reached",
        }
    }
}

/// Compact health metadata from fitting a rating model.
///
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FitDiagnostics {
    pub reason: FitReason,
    pub iterations: usize,
    pub rejected_edges: usize,
    pub covariance_inverse_fallback: bool,
    pub sanitized_covariance_entries: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchupResidual {
    pub engine_a: usize,
    pub engine_b: usize,
    pub pairs: usize,
    pub observed_score: f64,
    pub expected_score: f64,
    pub residual_ppg: f64,
    pub standardized_residual: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransitivityDiagnostics {
    pub observed_edges: usize,
    pub possible_edges: usize,
    pub connected_components: usize,
    pub cycle_degrees: usize,
    pub residuals: Vec<MatchupResidual>,
}

impl RatingModel {
    /// Returns the variance of the Elo contrast `rating[a] - rating[b]`.
    ///
    /// Invalid indices return NaN.
    fn contrast_variance(&self, a: usize, b: usize) -> f64 {
        if a >= self.covariance.len()
            || b >= self.covariance.len()
            || self.covariance[a].len() <= b
            || self.covariance[b].len() <= a
        {
            return f64::NAN;
        }
        if a == b {
            return 0.0;
        }
        (self.covariance[a][a] + self.covariance[b][b]
            - self.covariance[a][b]
            - self.covariance[b][a])
            .max(0.0)
    }
}

/// Computes descriptive observed-versus-model matchup residuals.
///
/// Residuals are not significance tests: the same adaptive observations were
/// used to fit the model. Pair-aware bootstrap calibration can be layered on
/// these sufficient statistics without retaining individual game data.
pub fn transitivity_diagnostics(
    model: &RatingModel,
    edges: &[RankingEdge],
    minimum_pairs: usize,
) -> TransitivityDiagnostics {
    let engine_count = model.ratings.len();
    let mut parent = (0..engine_count).collect::<Vec<_>>();
    let mut observed_edges = 0usize;
    let mut residuals = Vec::new();

    for edge in edges.iter().filter(|edge| {
        valid_edge(engine_count, edge)
            && edge.completed_pairs >= minimum_pairs
            && edge.engine_a < model.ratings.len()
            && edge.engine_b < model.ratings.len()
    }) {
        observed_edges += 1;
        union(&mut parent, edge.engine_a, edge.engine_b);
        let observed = edge.score_sum_a / edge.rated_games as f64;
        let expected = logistic(
            (model.ratings[edge.engine_a].elo - model.ratings[edge.engine_b].elo) * ELO_TO_LOG_ODDS,
        );
        let residual = observed - expected;
        let cluster_squared = edge.sum_score_squared - 2.0 * observed * edge.sum_m_score
            + observed * observed * edge.sum_m_squared;
        let standard_error = if edge.completed_pairs > 1 {
            let correction = edge.completed_pairs as f64 / (edge.completed_pairs - 1) as f64;
            Some((correction * cluster_squared.max(0.0)).sqrt() / edge.rated_games as f64)
        } else {
            None
        };
        residuals.push(MatchupResidual {
            engine_a: edge.engine_a,
            engine_b: edge.engine_b,
            pairs: edge.completed_pairs,
            observed_score: observed,
            expected_score: expected,
            residual_ppg: 6.0 * residual,
            standardized_residual: standard_error
                .filter(|error| *error > 0.0)
                .map(|error| residual / error),
        });
    }
    residuals.sort_by(|left, right| {
        right
            .residual_ppg
            .abs()
            .total_cmp(&left.residual_ppg.abs())
            .then_with(|| (left.engine_a, left.engine_b).cmp(&(right.engine_a, right.engine_b)))
    });

    let connected_components = if engine_count == 0 {
        0
    } else {
        (0..engine_count)
            .map(|index| root(&mut parent, index))
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    TransitivityDiagnostics {
        observed_edges,
        possible_edges: engine_count.saturating_mul(engine_count.saturating_sub(1)) / 2,
        connected_components,
        cycle_degrees: observed_edges
            .saturating_sub(engine_count.saturating_sub(connected_components)),
        residuals,
    }
}

fn root(parent: &mut [usize], index: usize) -> usize {
    if parent[index] != index {
        parent[index] = root(parent, parent[index]);
    }
    parent[index]
}

fn union(parent: &mut [usize], left: usize, right: usize) {
    let left = root(parent, left);
    let right = root(parent, right);
    if left != right {
        parent[right] = left;
    }
}

/// Fits a Bradley-Terry MAP model from pair aggregates.
///
/// Invalid edges are ignored. The likelihood treats each rated game as one
/// normalized point, while covariance uses completed-pair score clusters.
pub fn fit_rating_model(engine_count: usize, edges: &[RankingEdge]) -> RatingModel {
    if engine_count == 0 {
        return RatingModel {
            ratings: Vec::new(),
            covariance: Vec::new(),
            fit_diagnostics: FitDiagnostics {
                reason: FitReason::NoObservations,
                iterations: 0,
                rejected_edges: edges.len(),
                covariance_inverse_fallback: false,
                sanitized_covariance_entries: 0,
            },
        };
    }

    let supplied_edge_count = edges.len();
    let edges = edges
        .iter()
        .filter(|edge| valid_edge(engine_count, edge))
        .copied()
        .collect::<Vec<_>>();
    let mut fit_diagnostics = FitDiagnostics {
        reason: FitReason::MaximumIterations,
        iterations: 0,
        rejected_edges: supplied_edge_count - edges.len(),
        covariance_inverse_fallback: false,
        sanitized_covariance_entries: 0,
    };
    let prior_precision = ((PRIOR_RD * ELO_TO_LOG_ODDS).powi(2)).recip();
    let mut strengths = vec![0.0; engine_count];

    for iteration in 0..MAX_ITERATIONS {
        fit_diagnostics.iterations = iteration + 1;
        let (gradient, information) = derivatives(&strengths, &edges, prior_precision);
        let Some(step) = cholesky_solve(&information, &gradient) else {
            fit_diagnostics.reason = FitReason::LinearSolveFailed;
            break;
        };
        let largest_step = step
            .iter()
            .fold(0.0_f64, |largest, value| largest.max(value.abs()));
        if largest_step < 1e-12 {
            fit_diagnostics.reason = FitReason::Converged;
            break;
        }

        let objective = log_posterior(&strengths, &edges, prior_precision);
        let mut scale = 1.0;
        let line_search_failed = loop {
            let mut candidate = strengths
                .iter()
                .zip(&step)
                .map(|(strength, step)| strength + scale * step)
                .collect::<Vec<_>>();
            center(&mut candidate);
            let accepted = log_posterior(&candidate, &edges, prior_precision) >= objective;
            if accepted || scale <= 1e-8 {
                strengths = candidate;
                break !accepted;
            }
            scale *= 0.5;
        };
        if line_search_failed {
            fit_diagnostics.reason = FitReason::LineSearchFailed;
            break;
        }
        if scale * largest_step < 1e-10 {
            fit_diagnostics.reason = FitReason::Converged;
            break;
        }
    }
    if edges.is_empty() && fit_diagnostics.reason == FitReason::Converged {
        fit_diagnostics.reason = FitReason::NoObservations;
    }

    let (_, information) = derivatives(&strengths, &edges, prior_precision);
    let bread_inverse = dense_inverse(&information).unwrap_or_else(|| {
        fit_diagnostics.covariance_inverse_fallback = true;
        let variance = prior_precision.recip();
        let mut fallback = vec![vec![0.0; engine_count]; engine_count];
        for (index, row) in fallback.iter_mut().enumerate() {
            row[index] = variance;
        }
        fallback
    });
    let completed_pairs = edges.iter().map(|edge| edge.completed_pairs).sum::<usize>();
    let robust_weight =
        completed_pairs as f64 / (completed_pairs as f64 + ROBUST_COVARIANCE_PRIOR_PAIRS);
    let cluster_correction = if completed_pairs > 1 {
        completed_pairs as f64 / (completed_pairs - 1) as f64
    } else {
        1.0
    };
    let mut cluster_meat = vec![vec![0.0; engine_count]; engine_count];
    for edge in &edges {
        let probability = logistic(strengths[edge.engine_a] - strengths[edge.engine_b]);
        let cluster_score = edge.sum_score_squared - 2.0 * probability * edge.sum_m_score
            + probability * probability * edge.sum_m_squared;
        add_pair_matrix(
            &mut cluster_meat,
            edge.engine_a,
            edge.engine_b,
            cluster_correction * cluster_score.max(0.0),
        );
    }
    let mut meat = information.clone();
    for row in 0..engine_count {
        for column in 0..engine_count {
            let prior = if row == column { prior_precision } else { 0.0 };
            let model_information = information[row][column] - prior;
            meat[row][column] = prior
                + (1.0 - robust_weight) * model_information
                + robust_weight * cluster_meat[row][column];
        }
    }
    let covariance_log_odds = multiply_three(&bread_inverse, &meat, &bread_inverse);
    let mut rating_covariance = project_centered_components(&covariance_log_odds, &edges);
    let mut covariance = project_centered(covariance_log_odds);
    let elo_scale_squared = ELO_TO_LOG_ODDS.recip().powi(2);
    for matrix in [&mut covariance, &mut rating_covariance] {
        for row in matrix.iter_mut() {
            for value in row {
                *value *= elo_scale_squared;
                if !value.is_finite() {
                    *value = 0.0;
                    fit_diagnostics.sanitized_covariance_entries += 1;
                }
            }
        }
        symmetrize_and_sanitize(matrix);
    }

    let mut games_per_engine = vec![0_usize; engine_count];
    for edge in &edges {
        games_per_engine[edge.engine_a] =
            games_per_engine[edge.engine_a].saturating_add(edge.rated_games);
        games_per_engine[edge.engine_b] =
            games_per_engine[edge.engine_b].saturating_add(edge.rated_games);
    }
    let ratings = strengths
        .into_iter()
        .enumerate()
        .map(|(index, strength)| Rating {
            index,
            elo: 1500.0 + strength / ELO_TO_LOG_ODDS,
            rd: rating_covariance[index][index].sqrt().min(MAX_RD),
            games: games_per_engine[index],
        })
        .collect();
    RatingModel {
        ratings,
        covariance,
        fit_diagnostics,
    }
}

fn valid_edge(engine_count: usize, edge: &&RankingEdge) -> bool {
    edge.engine_a < engine_count
        && edge.engine_b < engine_count
        && edge.engine_a != edge.engine_b
        && edge.rated_games > 0
        && edge.completed_pairs > 0
        && edge.score_sum_a.is_finite()
        && (0.0..=edge.rated_games as f64).contains(&edge.score_sum_a)
        && edge.sum_m_squared.is_finite()
        && edge.sum_m_squared >= 0.0
        && edge.sum_m_score.is_finite()
        && edge.sum_m_score >= 0.0
        && edge.sum_score_squared.is_finite()
        && edge.sum_score_squared >= 0.0
}

fn derivatives(
    strengths: &[f64],
    edges: &[RankingEdge],
    prior_precision: f64,
) -> (Vec<f64>, Vec<Vec<f64>>) {
    let mut gradient = strengths
        .iter()
        .map(|strength| -prior_precision * strength)
        .collect::<Vec<_>>();
    let mut information = vec![vec![0.0; strengths.len()]; strengths.len()];
    for (index, row) in information.iter_mut().enumerate() {
        row[index] = prior_precision;
    }
    for edge in edges {
        let probability = logistic(strengths[edge.engine_a] - strengths[edge.engine_b]);
        let residual = edge.score_sum_a - edge.rated_games as f64 * probability;
        gradient[edge.engine_a] += residual;
        gradient[edge.engine_b] -= residual;
        add_pair_matrix(
            &mut information,
            edge.engine_a,
            edge.engine_b,
            edge.rated_games as f64 * probability * (1.0 - probability),
        );
    }
    (gradient, information)
}

fn add_pair_matrix(matrix: &mut [Vec<f64>], a: usize, b: usize, value: f64) {
    matrix[a][a] += value;
    matrix[b][b] += value;
    matrix[a][b] -= value;
    matrix[b][a] -= value;
}

fn log_posterior(strengths: &[f64], edges: &[RankingEdge], prior_precision: f64) -> f64 {
    let prior = -0.5 * prior_precision * strengths.iter().map(|value| value * value).sum::<f64>();
    edges.iter().fold(prior, |objective, edge| {
        let difference = strengths[edge.engine_a] - strengths[edge.engine_b];
        objective + edge.score_sum_a * difference - edge.rated_games as f64 * softplus(difference)
    })
}

fn softplus(value: f64) -> f64 {
    if value > 0.0 {
        value + (-value).exp().ln_1p()
    } else {
        value.exp().ln_1p()
    }
}

fn center(values: &mut [f64]) {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    for value in values {
        *value -= mean;
    }
}

fn cholesky(matrix: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let size = matrix.len();
    let mut lower = vec![vec![0.0; size]; size];
    for row in 0..size {
        for column in 0..=row {
            let mut value = matrix[row][column];
            for (left, right) in lower[row][..column].iter().zip(&lower[column][..column]) {
                value -= left * right;
            }
            if row == column {
                if !value.is_finite() || value <= 0.0 {
                    return None;
                }
                lower[row][column] = value.sqrt();
            } else {
                lower[row][column] = value / lower[column][column];
            }
        }
    }
    Some(lower)
}

fn cholesky_solve(matrix: &[Vec<f64>], right: &[f64]) -> Option<Vec<f64>> {
    let lower = cholesky(matrix)?;
    Some(solve_cholesky(&lower, right))
}

fn solve_cholesky(lower: &[Vec<f64>], right: &[f64]) -> Vec<f64> {
    let size = lower.len();
    let mut result = vec![0.0; size];
    for row in 0..size {
        let known = (0..row)
            .map(|column| lower[row][column] * result[column])
            .sum::<f64>();
        result[row] = (right[row] - known) / lower[row][row];
    }
    for row in (0..size).rev() {
        let known = (row + 1..size)
            .map(|column| lower[column][row] * result[column])
            .sum::<f64>();
        result[row] = (result[row] - known) / lower[row][row];
    }
    result
}

fn dense_inverse(matrix: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let lower = cholesky(matrix)?;
    let size = matrix.len();
    let mut inverse = vec![vec![0.0; size]; size];
    for column in 0..size {
        let mut unit = vec![0.0; size];
        unit[column] = 1.0;
        let solution = solve_cholesky(&lower, &unit);
        for row in 0..size {
            inverse[row][column] = solution[row];
        }
    }
    symmetrize_and_sanitize(&mut inverse);
    Some(inverse)
}

fn multiply_three(left: &[Vec<f64>], middle: &[Vec<f64>], right: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let size = left.len();
    let mut intermediate = vec![vec![0.0; size]; size];
    let mut result = vec![vec![0.0; size]; size];
    for row in 0..size {
        for inner in 0..size {
            for column in 0..size {
                intermediate[row][column] += left[row][inner] * middle[inner][column];
            }
        }
    }
    for row in 0..size {
        for inner in 0..size {
            for column in 0..size {
                result[row][column] += intermediate[row][inner] * right[inner][column];
            }
        }
    }
    result
}

fn project_centered(matrix: Vec<Vec<f64>>) -> Vec<Vec<f64>> {
    let size = matrix.len();
    let row_means = matrix
        .iter()
        .map(|row| row.iter().sum::<f64>() / size as f64)
        .collect::<Vec<_>>();
    let column_means = (0..size)
        .map(|column| matrix.iter().map(|row| row[column]).sum::<f64>() / size as f64)
        .collect::<Vec<_>>();
    let grand_mean = row_means.iter().sum::<f64>() / size as f64;
    (0..size)
        .map(|row| {
            (0..size)
                .map(|column| {
                    matrix[row][column] - row_means[row] - column_means[column] + grand_mean
                })
                .collect()
        })
        .collect()
}

fn project_centered_components(matrix: &[Vec<f64>], edges: &[RankingEdge]) -> Vec<Vec<f64>> {
    let size = matrix.len();
    let mut parent = (0..size).collect::<Vec<_>>();
    for edge in edges {
        union(&mut parent, edge.engine_a, edge.engine_b);
    }
    let mut components = BTreeMap::<usize, Vec<usize>>::new();
    for index in 0..size {
        components
            .entry(root(&mut parent, index))
            .or_default()
            .push(index);
    }
    let mut projected = vec![vec![0.0; size]; size];
    for component in components.values() {
        if component.len() == 1 {
            let index = component[0];
            projected[index][index] = matrix[index][index];
            continue;
        }
        let divisor = component.len() as f64;
        let row_means = component
            .iter()
            .map(|&row| {
                component
                    .iter()
                    .map(|&column| matrix[row][column])
                    .sum::<f64>()
                    / divisor
            })
            .collect::<Vec<_>>();
        let grand_mean = row_means.iter().sum::<f64>() / divisor;
        for (row_position, &row) in component.iter().enumerate() {
            for (column_position, &column) in component.iter().enumerate() {
                projected[row][column] =
                    matrix[row][column] - row_means[row_position] - row_means[column_position]
                        + grand_mean;
            }
        }
    }
    projected
}

fn symmetrize_and_sanitize(matrix: &mut [Vec<f64>]) {
    for row in 0..matrix.len() {
        let (previous_rows, current_and_later) = matrix.split_at_mut(row);
        let current = &mut current_and_later[0];
        for (column, previous) in previous_rows.iter_mut().enumerate() {
            let value = 0.5 * (current[column] + previous[row]);
            current[column] = value;
            previous[row] = value;
        }
        current[row] = current[row].max(0.0);
    }
}

fn select_information_pair(
    model: &RatingModel,
    pair_counts: &[Vec<usize>],
    average_decision_time: &[Option<Duration>],
    last_played_batch: &[Option<usize>],
    next_batch: usize,
) -> Option<(usize, usize)> {
    let ratings = &model.ratings;
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
        || model.covariance.len() <= max_index
        || model.covariance.iter().any(|row| row.len() <= max_index)
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
            .max(ACCEPTABLE_MOVE_SECONDS)
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
            let uncertainty = model.contrast_variance(left.index, right.index);
            let information = probability * (1.0 - probability) * uncertainty;
            let score = information / (engine_cost(left.index) + engine_cost(right.index)).sqrt();
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

/// Selects placement games or an information-maximizing matchup.
pub fn select_pair_for_model(
    model: &RatingModel,
    pair_counts: &[Vec<usize>],
    average_decision_time: &[Option<Duration>],
    last_played_batch: &[Option<usize>],
    next_batch: usize,
    placement_opponents: usize,
    placement_pairs: usize,
) -> Option<(usize, usize)> {
    select_pair(
        model,
        SelectionContext {
            pair_counts,
            average_decision_time,
            last_played_batch,
            next_batch,
            placement_opponents,
            placement_pairs,
        },
    )
}

struct SelectionContext<'a> {
    pair_counts: &'a [Vec<usize>],
    average_decision_time: &'a [Option<Duration>],
    last_played_batch: &'a [Option<usize>],
    next_batch: usize,
    placement_opponents: usize,
    placement_pairs: usize,
}

fn select_pair(model: &RatingModel, context: SelectionContext<'_>) -> Option<(usize, usize)> {
    let ratings = &model.ratings;
    let SelectionContext {
        pair_counts,
        average_decision_time,
        last_played_batch,
        next_batch,
        placement_opponents,
        placement_pairs,
    } = context;
    let information_choice = select_information_pair(
        model,
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
            let uncertainty = model.contrast_variance(left.index, right.index);
            let information = probability * (1.0 - probability) * uncertainty;
            let left_cost = average_decision_time[left.index]
                .map_or(1.0, |cost| cost.as_secs_f64())
                .max(ACCEPTABLE_MOVE_SECONDS);
            let right_cost = average_decision_time[right.index]
                .map_or(1.0, |cost| cost.as_secs_f64())
                .max(ACCEPTABLE_MOVE_SECONDS);
            let utility = information / (left_cost + right_cost).sqrt();
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

    fn edge(a: usize, b: usize, games: usize, score: f64) -> RankingEdge {
        RankingEdge {
            engine_a: a,
            engine_b: b,
            rated_games: games,
            score_sum_a: score,
            completed_pairs: games,
            sum_m_squared: games as f64,
            sum_m_score: score,
            sum_score_squared: score,
        }
    }

    #[test]
    fn stronger_engines_rank_above_weaker_engines() {
        let model = fit_rating_model(
            3,
            &[
                edge(0, 1, 40, 40.0),
                edge(0, 2, 40, 40.0),
                edge(1, 2, 40, 40.0),
            ],
        );
        let ratings = model.ratings;

        assert!(ratings[0].elo > ratings[1].elo);
        assert!(ratings[1].elo > ratings[2].elo);
        assert!((ratings.iter().map(|rating| rating.elo).sum::<f64>() / 3.0 - 1500.0).abs() < 1e-9);
        assert_eq!(ratings[0].games, 80);
    }

    #[test]
    fn fit_diagnostics_report_healthy_fit_and_rejected_edges() {
        let invalid = RankingEdge {
            score_sum_a: f64::NAN,
            ..edge(0, 1, 10, 5.0)
        };
        let model = fit_rating_model(2, &[edge(0, 1, 10, 6.0), invalid]);

        assert!(model.fit_diagnostics.reason.is_healthy());
        assert!(model.fit_diagnostics.iterations > 0);
        assert_eq!(model.fit_diagnostics.rejected_edges, 1);
        assert!(!model.fit_diagnostics.covariance_inverse_fallback);
        assert_eq!(model.fit_diagnostics.sanitized_covariance_entries, 0);
    }

    #[test]
    fn point_losses_outweigh_more_normal_wins() {
        let ratings = fit_rating_model(2, &[edge(0, 1, 10, 4.0)]).ratings;

        assert!(ratings[0].elo < ratings[1].elo);
    }

    #[test]
    fn uncertainty_continues_falling_with_more_games() {
        let sparse = fit_rating_model(2, &[edge(0, 1, 20, 10.0)]);
        let dense = fit_rating_model(2, &[edge(0, 1, 2_000, 1_000.0)]);

        assert!(dense.contrast_variance(0, 1) < sparse.contrast_variance(0, 1));
    }

    #[test]
    fn covariance_is_symmetric_centered_and_finite() {
        let model = fit_rating_model(3, &[edge(0, 1, 20, 12.0), edge(1, 2, 15, 6.0)]);

        for row in 0..3 {
            assert!(model.covariance[row][row].is_finite());
            assert!(model.covariance[row][row] >= 0.0);
            assert!(model.covariance[row].iter().sum::<f64>().abs() < 1e-8);
            for column in 0..3 {
                assert!(
                    (model.covariance[row][column] - model.covariance[column][row]).abs() < 1e-10
                );
            }
        }
    }

    #[test]
    fn two_engine_contrast_matches_balanced_analytic_model() {
        let model = fit_rating_model(2, &[edge(0, 1, 2, 1.0)]);
        let prior_precision = ((PRIOR_RD * ELO_TO_LOG_ODDS).powi(2)).recip();
        let robust_weight = 2.0 / (2.0 + ROBUST_COVARIANCE_PRIOR_PAIRS);
        let meat_edge = (1.0 - robust_weight) * 0.5 + robust_weight;
        let expected = 2.0 * (prior_precision + 2.0 * meat_edge)
            / (prior_precision + 1.0).powi(2)
            / ELO_TO_LOG_ODDS.powi(2);

        assert!((model.ratings[0].elo - 1500.0).abs() < 1e-10);
        assert!((model.contrast_variance(0, 1) - expected).abs() < 1e-8);
        assert_eq!(model.contrast_variance(0, 0), 0.0);
        assert!(model.contrast_variance(0, 2).is_nan());
    }

    #[test]
    fn opponent_graph_reduces_indirect_contrast_uncertainty() {
        let chain = fit_rating_model(3, &[edge(0, 1, 20, 10.0), edge(1, 2, 20, 10.0)]);
        let triangle = fit_rating_model(
            3,
            &[
                edge(0, 1, 20, 10.0),
                edge(1, 2, 20, 10.0),
                edge(0, 2, 20, 10.0),
            ],
        );

        assert!(triangle.contrast_variance(0, 2) < chain.contrast_variance(0, 2));
    }

    #[test]
    fn pair_cluster_moments_change_covariance_not_point_estimates() {
        let independent = RankingEdge {
            engine_a: 0,
            engine_b: 1,
            rated_games: 4,
            score_sum_a: 2.0,
            completed_pairs: 4,
            sum_m_squared: 4.0,
            sum_m_score: 2.0,
            sum_score_squared: 2.0,
        };
        let one_pair = RankingEdge {
            completed_pairs: 1,
            sum_m_squared: 16.0,
            sum_m_score: 8.0,
            sum_score_squared: 4.0,
            ..independent
        };

        let independent = fit_rating_model(2, &[independent]);
        let clustered = fit_rating_model(2, &[one_pair]);

        assert_eq!(independent.ratings[0].elo, clustered.ratings[0].elo);
        assert!(clustered.contrast_variance(0, 1) < independent.contrast_variance(0, 1));
    }

    #[test]
    fn one_balanced_pair_keeps_substantial_prior_uncertainty() {
        let balanced_pair = RankingEdge {
            engine_a: 0,
            engine_b: 1,
            rated_games: 2,
            score_sum_a: 1.0,
            completed_pairs: 1,
            sum_m_squared: 4.0,
            sum_m_score: 2.0,
            sum_score_squared: 1.0,
        };

        let model = fit_rating_model(2, &[balanced_pair]);

        assert!(model.contrast_variance(0, 1).sqrt() > 150.0);
    }

    #[test]
    fn diagnostics_identify_cycle_space_and_matchup_residuals() {
        let edges = [
            edge(0, 1, 100, 60.0),
            edge(1, 2, 100, 60.0),
            edge(0, 2, 100, 40.0),
        ];
        let model = fit_rating_model(3, &edges);

        let diagnostics = transitivity_diagnostics(&model, &edges, 30);

        assert_eq!(diagnostics.observed_edges, 3);
        assert_eq!(diagnostics.possible_edges, 3);
        assert_eq!(diagnostics.connected_components, 1);
        assert_eq!(diagnostics.cycle_degrees, 1);
        assert_eq!(diagnostics.residuals.len(), 3);
        assert!(
            diagnostics
                .residuals
                .iter()
                .all(|residual| residual.residual_ppg.abs() > 0.5)
        );
    }

    #[test]
    fn full_model_handles_no_games_and_disconnected_engines() {
        let empty = fit_rating_model(3, &[]);
        assert!(empty.fit_diagnostics.reason.is_healthy());
        assert!(empty.ratings.iter().all(|rating| rating.elo == 1500.0));
        assert!(
            empty
                .ratings
                .iter()
                .all(|rating| (rating.rd - PRIOR_RD).abs() < 1e-8)
        );
        assert!(
            empty
                .covariance
                .iter()
                .flatten()
                .all(|value| value.is_finite())
        );

        let disconnected = fit_rating_model(4, &[edge(0, 1, 1, 1.0)]);
        assert_eq!(disconnected.ratings[2].elo, 1500.0);
        assert_eq!(disconnected.ratings[3].elo, 1500.0);
        assert_eq!(disconnected.ratings[2].games, 0);
        assert!((disconnected.ratings[2].rd - PRIOR_RD).abs() < 1e-8);
        assert!(disconnected.contrast_variance(2, 3) > 0.0);

        let connected = fit_rating_model(2, &[edge(0, 1, 100, 50.0)]);
        let with_new_engine = fit_rating_model(3, &[edge(0, 1, 100, 50.0)]);
        assert!((connected.ratings[0].rd - with_new_engine.ratings[0].rd).abs() < 1e-8);
    }

    #[test]
    fn permuting_engine_indices_permutes_the_model() {
        let original = fit_rating_model(3, &[edge(0, 1, 12, 8.0), edge(1, 2, 10, 3.0)]);
        let permuted = fit_rating_model(3, &[edge(2, 0, 12, 8.0), edge(0, 1, 10, 3.0)]);
        let permutation = [2, 0, 1];

        for old in 0..3 {
            assert!(
                (original.ratings[old].elo - permuted.ratings[permutation[old]].elo).abs() < 1e-8
            );
            for other in 0..3 {
                assert!(
                    (original.covariance[old][other]
                        - permuted.covariance[permutation[old]][permutation[other]])
                        .abs()
                        < 1e-8
                );
            }
        }
    }

    #[test]
    fn information_selection_avoids_a_gross_mismatch() {
        let mut model = fit_rating_model(3, &[]);
        model.ratings[0].elo = 1100.0;
        model.ratings[2].elo = 1550.0;
        let pair_counts = counts([[0, 4, 4], [4, 0, 4], [4, 4, 0]]);

        assert_eq!(
            select_pair_for_model(&model, &pair_counts, &[None; 3], &[None; 3], 0, 0, 0),
            Some((1, 2))
        );
    }

    #[test]
    fn selection_ties_are_stable_by_engine_index() {
        let mut model = fit_rating_model(3, &[]);
        model.ratings.rotate_right(1);
        let pair_counts = counts([[0, 3, 3], [3, 0, 3], [3, 3, 0]]);

        assert_eq!(
            select_pair_for_model(&model, &pair_counts, &[None; 3], &[None; 3], 0, 0, 0),
            Some((0, 1))
        );
    }

    #[test]
    fn information_selection_penalizes_slow_matchups() {
        let model = fit_rating_model(3, &[]);
        let pair_counts = counts([[0, 3, 3], [3, 0, 3], [3, 3, 0]]);

        assert_eq!(
            select_pair_for_model(
                &model,
                &pair_counts,
                &[
                    Some(Duration::from_millis(10)),
                    Some(Duration::from_millis(10)),
                    Some(Duration::from_secs(1))
                ],
                &[Some(9); 3],
                10,
                0,
                0,
            ),
            Some((0, 1))
        );
    }

    #[test]
    fn cheap_saturated_engines_do_not_crowd_out_uncertain_engines() {
        let model = fit_rating_model(4, &[edge(2, 3, 2_000, 1_000.0)]);
        let pair_counts = vec![vec![0; 4]; 4];

        assert_eq!(
            select_pair_for_model(
                &model,
                &pair_counts,
                &[
                    Some(Duration::from_millis(20)),
                    Some(Duration::from_millis(20)),
                    Some(Duration::from_micros(100)),
                    Some(Duration::from_micros(100))
                ],
                &[Some(9); 4],
                10,
                0,
                0,
            ),
            Some((0, 1))
        );
    }

    #[test]
    fn information_selection_moves_on_from_saturated_pair() {
        let model = fit_rating_model(4, &[edge(0, 1, 2_000, 1_000.0)]);
        let pair_counts = vec![vec![0; 4]; 4];

        assert_eq!(
            select_pair_for_model(
                &model,
                &pair_counts,
                &[Some(Duration::from_millis(1)); 4],
                &[Some(9); 4],
                10,
                0,
                0,
            ),
            Some((2, 3))
        );
    }

    #[test]
    fn idle_engine_is_not_starved_by_runtime_cost() {
        let model = fit_rating_model(3, &[]);
        let pair_counts = counts([[0, 3, 3], [3, 0, 3], [3, 3, 0]]);

        assert_eq!(
            select_pair_for_model(
                &model,
                &pair_counts,
                &[
                    Some(Duration::from_millis(10)),
                    Some(Duration::from_millis(10)),
                    Some(Duration::from_secs(1))
                ],
                &[Some(24), Some(24), Some(5)],
                25,
                0,
                0,
            ),
            Some((0, 2))
        );
    }

    #[test]
    fn selection_rejects_malformed_inputs() {
        let duplicate = RatingModel {
            ratings: vec![rating(0, 1500.0, 100.0), rating(0, 1500.0, 100.0)],
            covariance: vec![vec![1.0]],
            fit_diagnostics: FitDiagnostics {
                reason: FitReason::NoObservations,
                iterations: 0,
                rejected_edges: 0,
                covariance_inverse_fallback: false,
                sanitized_covariance_entries: 0,
            },
        };
        assert_eq!(
            select_pair_for_model(&duplicate, &[vec![0]], &[None], &[None], 0, 0, 0),
            None
        );

        let model = fit_rating_model(2, &[]);
        assert_eq!(
            select_pair_for_model(
                &model,
                &[vec![0, 1], vec![2, 0]],
                &[None; 2],
                &[None; 2],
                0,
                0,
                0,
            ),
            None
        );
        let one_engine = fit_rating_model(1, &[]);
        assert_eq!(
            select_pair_for_model(&one_engine, &[vec![0]], &[None], &[None], 0, 0, 0),
            None
        );
    }

    #[test]
    fn placement_does_not_require_every_pair() {
        let model = fit_rating_model(4, &[]);
        let pair_counts = vec![
            vec![0, 20, 20, 0],
            vec![20, 0, 0, 0],
            vec![20, 0, 0, 0],
            vec![0, 0, 0, 0],
        ];

        assert_eq!(
            select_pair_for_model(&model, &pair_counts, &[None; 4], &[None; 4], 0, 2, 20),
            Some((1, 2))
        );
    }

    #[test]
    fn placement_spreads_games_before_deepening_a_matchup() {
        let model = fit_rating_model(3, &[]);
        let pair_counts = counts([[0, 5, 0], [5, 0, 0], [0, 0, 0]]);

        assert_eq!(
            select_pair_for_model(&model, &pair_counts, &[None; 3], &[None; 3], 0, 2, 10),
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
