use std::time::Duration;

use crate::report::{mean_ci95, StatusView};

pub struct DuelStats {
    a_points: f32,
    a_points_sq_sum: f64,
    incomplete: usize,
    total_plies: usize,
    a_decisions: usize,
    b_decisions: usize,
    a_decision_sec: f64,
    b_decision_sec: f64,
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
        Self {
            a_points: 0.0,
            a_points_sq_sum: 0.0,
            incomplete: 0,
            total_plies: 0,
            a_decisions: 0,
            b_decisions: 0,
            a_decision_sec: 0.0,
            b_decision_sec: 0.0,
            a_wins: 0,
            b_wins: 0,
            a_gammons: 0,
            b_gammons: 0,
            a_backgammons: 0,
            b_backgammons: 0,
            a_normals: 0,
            b_normals: 0,
            a_points_as_x: 0.0,
            a_points_as_o: 0.0,
            b_points_as_x: 0.0,
            b_points_as_o: 0.0,
            a_games_as_x: 0,
            a_games_as_o: 0,
            b_games_as_x: 0,
            b_games_as_o: 0,
        }
    }

    pub fn record_game(&mut self, update: &GameUpdate) -> (f32, f32) {
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
        self.a_points_sq_sum += (a_game_points as f64) * (a_game_points as f64);

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
        self.a_decision_sec += update.a_decision_sec;
        self.b_decision_sec += update.b_decision_sec;

        (a_game_points, b_game_points)
    }

    pub fn status_view<'a>(
        &self,
        engine_a: &'a str,
        engine_b: &'a str,
        games_done: usize,
        elapsed: Duration,
    ) -> StatusView<'a> {
        let elapsed_secs = elapsed.as_secs_f64();
        let games = games_done.max(1);
        let (a_avg_pts, a_avg_ci95) = mean_ci95(self.a_points as f64, self.a_points_sq_sum, games);

        StatusView {
            engine_a,
            engine_b,
            games_done,
            a_avg_pts,
            a_avg_ci95,
            a_win_pct: (self.a_wins as f64 / games as f64) * 100.0,
            b_win_pct: (self.b_wins as f64 / games as f64) * 100.0,
            a_gammons: self.a_gammons,
            b_gammons: self.b_gammons,
            a_backgammons: self.a_backgammons,
            b_backgammons: self.b_backgammons,
            a_normals: self.a_normals,
            b_normals: self.b_normals,
            incomplete_count: self.incomplete,
            a_points_as_x: self.a_points_as_x,
            a_points_as_o: self.a_points_as_o,
            b_points_as_x: self.b_points_as_x,
            b_points_as_o: self.b_points_as_o,
            a_games_as_x: self.a_games_as_x,
            a_games_as_o: self.a_games_as_o,
            b_games_as_x: self.b_games_as_x,
            b_games_as_o: self.b_games_as_o,
            a_avg_ms: if self.a_decisions == 0 {
                0.0
            } else {
                (self.a_decision_sec * 1000.0) / self.a_decisions as f64
            },
            b_avg_ms: if self.b_decisions == 0 {
                0.0
            } else {
                (self.b_decision_sec * 1000.0) / self.b_decisions as f64
            },
            games_per_sec: games_done as f64 / elapsed_secs.max(1e-9),
            avg_ply: self.total_plies as f64 / games as f64,
            elapsed,
        }
    }
}

pub struct GameUpdate {
    pub a_is_x: bool,
    pub winner_x: Option<bool>,
    pub points_x: f32,
    pub points_o: f32,
    pub plies: usize,
    pub a_decisions: usize,
    pub b_decisions: usize,
    pub a_decision_sec: f64,
    pub b_decision_sec: f64,
}
