use std::time::Duration;

pub struct StatusView<'a> {
    pub engine_a: &'a str,
    pub engine_b: &'a str,
    pub games_done: usize,
    pub a_avg_pts: f64,
    pub a_avg_ci95: f64,
    pub a_win_pct: f64,
    pub b_win_pct: f64,
    pub a_gammons: usize,
    pub b_gammons: usize,
    pub a_backgammons: usize,
    pub b_backgammons: usize,
    pub a_normals: usize,
    pub b_normals: usize,
    pub incomplete_count: usize,
    pub a_points_as_x: f32,
    pub a_points_as_o: f32,
    pub b_points_as_x: f32,
    pub b_points_as_o: f32,
    pub a_games_as_x: usize,
    pub a_games_as_o: usize,
    pub b_games_as_x: usize,
    pub b_games_as_o: usize,
    pub a_avg_ms: f64,
    pub b_avg_ms: f64,
    pub games_per_sec: f64,
    pub avg_ply: f64,
    pub elapsed: Duration,
}

pub fn render_status_lines(s: StatusView<'_>) -> (String, String, String, String, String, String) {
    let total_games = s.games_done;
    let a_normal_pct = ratio_pct(s.a_normals, total_games);
    let a_gammon_pct = ratio_pct(s.a_gammons, total_games);
    let a_backgammon_pct = ratio_pct(s.a_backgammons, total_games);
    let b_normal_pct = ratio_pct(s.b_normals, total_games);
    let b_gammon_pct = ratio_pct(s.b_gammons, total_games);
    let b_backgammon_pct = ratio_pct(s.b_backgammons, total_games);
    let incomplete_pct = ratio_pct(s.incomplete_count, s.games_done);

    let line_engines = format!("ENGINES A={}   B={}", s.engine_a, s.engine_b);
    let line_result = format!(
        " RESULT A vs B {:+.3} ± {:.3} ppg   win {:.1}/{:.1}%   over {} games",
        s.a_avg_pts, s.a_avg_ci95, s.a_win_pct, s.b_win_pct, s.games_done,
    );
    let line_rate = format!(
        "   RATE {:.2} g/s   avg ply {:.1}   elapsed {}",
        s.games_per_sec,
        s.avg_ply,
        fmt_duration_short(s.elapsed),
    );
    let line_decide = format!(
        " DECIDE A {:.2} ms/move   B {:.2} ms/move",
        s.a_avg_ms, s.b_avg_ms,
    );
    let line_class = format!(
        "  CLASS A n/g/bg {}-{}-{} ({:.1}/{:.1}/{:.1}%)   B {}-{}-{} ({:.1}/{:.1}/{:.1}%)   incomplete {} ({:.1}%)",
        s.a_normals,
        s.a_gammons,
        s.a_backgammons,
        a_normal_pct,
        a_gammon_pct,
        a_backgammon_pct,
        s.b_normals,
        s.b_gammons,
        s.b_backgammons,
        b_normal_pct,
        b_gammon_pct,
        b_backgammon_pct,
        s.incomplete_count,
        incomplete_pct,
    );
    let line_sides = format!(
        "  SIDES A X:{:+.3} O:{:+.3}   B X:{:+.3} O:{:+.3} ppg",
        per_game(s.a_points_as_x, s.a_games_as_x),
        per_game(s.a_points_as_o, s.a_games_as_o),
        per_game(s.b_points_as_x, s.b_games_as_x),
        per_game(s.b_points_as_o, s.b_games_as_o),
    );
    (
        line_engines,
        line_result,
        line_rate,
        line_decide,
        line_class,
        line_sides,
    )
}

pub fn mean_ci95(sum: f64, sum_sq: f64, n: usize) -> (f64, f64) {
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
    if n == 0 {
        0.0
    } else {
        sum as f64 / n as f64
    }
}

fn fmt_duration_short(d: Duration) -> String {
    let secs = d.as_secs();
    let millis = d.subsec_millis();

    if secs == 0 {
        return format!("{}ms", millis);
    }
    if secs < 60 {
        return format!("{:.2}s", d.as_secs_f64());
    }

    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m {}s", minutes, seconds)
    }
}
