use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Default)]
pub struct DuelStats {
    a_points: f32,
    pair_points: BTreeMap<usize, (f64, usize)>,
    incomplete: usize,
    total_plies: usize,
    a_decisions: usize,
    b_decisions: usize,
    a_decision_time: Duration,
    b_decision_time: Duration,
    a_wins: usize,
    b_wins: usize,
    a_gammons: usize,
    b_gammons: usize,
    a_backgammons: usize,
    b_backgammons: usize,
    a_normals: usize,
    b_normals: usize,
    a_points_as_x: f32,
    a_points_as_o: f32,
    b_points_as_x: f32,
    b_points_as_o: f32,
    a_games_as_x: usize,
    a_games_as_o: usize,
    b_games_as_x: usize,
    b_games_as_o: usize,
}

impl DuelStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_game(&mut self, update: &GameUpdate) -> f32 {
        let a_game_points = if update.a_is_x {
            update.points_x
        } else {
            update.points_o
        };
        let b_game_points = if update.a_is_x {
            update.points_o
        } else {
            update.points_x
        };

        self.a_points += a_game_points;
        let pair = self.pair_points.entry(update.game_idx / 2).or_default();
        pair.0 += f64::from(a_game_points);
        pair.1 += 1;

        if update.a_is_x {
            self.a_points_as_x += update.points_x;
            self.b_points_as_o += update.points_o;
            self.a_games_as_x += 1;
            self.b_games_as_o += 1;
        } else {
            self.a_points_as_o += update.points_o;
            self.b_points_as_x += update.points_x;
            self.a_games_as_o += 1;
            self.b_games_as_x += 1;
        }

        if update.winner_x.is_none() {
            self.incomplete += 1;
        }

        if a_game_points > 0.0 {
            self.a_wins += 1;
            match a_game_points.abs().round() as i32 {
                1 => self.a_normals += 1,
                2 => self.a_gammons += 1,
                3 => self.a_backgammons += 1,
                _ => {}
            }
        } else if b_game_points > 0.0 {
            self.b_wins += 1;
            match b_game_points.abs().round() as i32 {
                1 => self.b_normals += 1,
                2 => self.b_gammons += 1,
                3 => self.b_backgammons += 1,
                _ => {}
            }
        }

        self.total_plies += update.plies;
        self.a_decisions += update.a_decisions;
        self.b_decisions += update.b_decisions;
        self.a_decision_time += update.a_decision_time;
        self.b_decision_time += update.b_decision_time;

        a_game_points
    }

    pub fn status_lines(
        &self,
        engine_a: &str,
        engine_b: &str,
        games_done: usize,
        elapsed: Duration,
    ) -> [String; 6] {
        let elapsed_secs = elapsed.as_secs_f64();
        let games = games_done.max(1);
        let complete_pair_ppg = self
            .pair_points
            .values()
            .filter(|(_, legs)| *legs == 2)
            .map(|(points, _)| points / 2.0)
            .collect::<Vec<_>>();
        let pair_sum = complete_pair_ppg.iter().sum::<f64>();
        let pair_sq_sum = complete_pair_ppg.iter().map(|value| value * value).sum();
        let (a_avg_pts, a_avg_ci95) = if complete_pair_ppg.is_empty() {
            (self.a_points as f64 / games as f64, 0.0)
        } else {
            mean_ci95(pair_sum, pair_sq_sum, complete_pair_ppg.len())
        };
        let a_avg_ms = if self.a_decisions == 0 {
            0.0
        } else {
            self.a_decision_time.as_secs_f64() * 1000.0 / self.a_decisions as f64
        };
        let b_avg_ms = if self.b_decisions == 0 {
            0.0
        } else {
            self.b_decision_time.as_secs_f64() * 1000.0 / self.b_decisions as f64
        };

        [
            format!("ENGINES A={engine_a}   B={engine_b}"),
            format!(
                " RESULT A vs B {a_avg_pts:+.3} ± {a_avg_ci95:.3} ppg   win {:.1}/{:.1}%   over {games_done} games",
                ratio_pct(self.a_wins, games),
                ratio_pct(self.b_wins, games),
            ),
            format!(
                "   RATE {:.2} g/s   avg ply {:.1}   elapsed {}",
                games_done as f64 / elapsed_secs.max(1e-9),
                self.total_plies as f64 / games as f64,
                fmt_duration_short(elapsed),
            ),
            format!(" DECIDE A {a_avg_ms:.2} ms/move   B {b_avg_ms:.2} ms/move"),
            format!(
                "  CLASS A n/g/bg {}-{}-{} ({:.1}/{:.1}/{:.1}%)   B {}-{}-{} ({:.1}/{:.1}/{:.1}%)   incomplete {} ({:.1}%)",
                self.a_normals,
                self.a_gammons,
                self.a_backgammons,
                ratio_pct(self.a_normals, games_done),
                ratio_pct(self.a_gammons, games_done),
                ratio_pct(self.a_backgammons, games_done),
                self.b_normals,
                self.b_gammons,
                self.b_backgammons,
                ratio_pct(self.b_normals, games_done),
                ratio_pct(self.b_gammons, games_done),
                ratio_pct(self.b_backgammons, games_done),
                self.incomplete,
                ratio_pct(self.incomplete, games_done),
            ),
            format!(
                "  SIDES A X:{:+.3} O:{:+.3}   B X:{:+.3} O:{:+.3} ppg",
                per_game(self.a_points_as_x, self.a_games_as_x),
                per_game(self.a_points_as_o, self.a_games_as_o),
                per_game(self.b_points_as_x, self.b_games_as_x),
                per_game(self.b_points_as_o, self.b_games_as_o),
            ),
        ]
    }
}

fn mean_ci95(sum: f64, sum_sq: f64, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let mean = sum / n as f64;
    if n < 2 {
        return (mean, 0.0);
    }
    let variance = ((sum_sq - (sum * sum) / n as f64) / (n as f64 - 1.0)).max(0.0);
    let se = (variance / n as f64).sqrt();
    (mean, 1.96 * se)
}

fn ratio_pct(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (count as f64 / total as f64) * 100.0
    }
}

fn per_game(sum: f32, n: usize) -> f64 {
    if n == 0 { 0.0 } else { sum as f64 / n as f64 }
}

fn fmt_duration_short(d: Duration) -> String {
    let secs = d.as_secs();
    let millis = d.subsec_millis();

    if secs == 0 {
        return format!("{millis}ms");
    }
    if secs < 60 {
        return format!("{:.2}s", d.as_secs_f64());
    }

    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

pub struct GameUpdate {
    pub game_idx: usize,
    pub a_is_x: bool,
    pub winner_x: Option<bool>,
    pub points_x: f32,
    pub points_o: f32,
    pub plies: usize,
    pub a_decisions: usize,
    pub b_decisions: usize,
    pub a_decision_time: Duration,
    pub b_decision_time: Duration,
}
