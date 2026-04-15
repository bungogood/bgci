use std::fs;
use std::path::Path;

use bkgm::Variant;

use crate::duel_game::PlyRecord;

pub struct MatGameRecord {
    pub game_idx: usize,
    pub a_is_x: bool,
    pub winner_x: Option<bool>,
    pub points_x: f32,
    pub plies: Vec<PlyRecord>,
}

pub fn write_gnubg_mat(
    path: &Path,
    engine_a: &str,
    engine_b: &str,
    variant: Variant,
    timestamp: &str,
    games: &[MatGameRecord],
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut games_sorted: Vec<&MatGameRecord> = games.iter().collect();
    games_sorted.sort_by_key(|g| g.game_idx);

    let mut out = String::new();
    out.push_str("; [Site \"bgci\"]\n");
    out.push_str("; [Event \"Engine Duel\"]\n");
    if timestamp.len() >= 8 {
        let y = &timestamp[0..4];
        let m = &timestamp[4..6];
        let d = &timestamp[6..8];
        out.push_str(&format!("; [EventDate \"{y}.{m}.{d}\"]\n"));
    }
    out.push_str(&format!("; [Variation \"{}\"]\n", variation_tag(variant)));
    out.push_str("; [Transcriber \"bgci\"]\n");
    out.push_str("; [CubeLimit \"1\"]\n\n");
    out.push_str(" 0 point match\n");

    let mut score_a = 0i32;
    let mut score_b = 0i32;
    let mut exported_game_no = 0usize;

    for game in games_sorted {
        let Some(winner_x) = game.winner_x else {
            continue;
        };

        exported_game_no += 1;
        out.push_str("\n");
        out.push_str(&format!(" Game {}\n", exported_game_no));
        out.push_str(&format!(
            " {} : {}{:>23}{} : {}\n",
            engine_a, score_a, "", engine_b, score_b
        ));

        for (idx, (left, right)) in grouped_turns(game.plies.as_slice()).into_iter().enumerate() {
            out.push_str(&format!("{:>3}) {:<27}", idx + 1, left.unwrap_or_default()));
            if let Some(right_mv) = right {
                out.push_str(&format!(" {}", right_mv));
            }
            out.push('\n');
        }

        let points = game.points_x.abs().round() as i32;
        let point_word = if points == 1 { "point" } else { "points" };
        let winner_is_a = if game.a_is_x { winner_x } else { !winner_x };
        let win_text = format!("Wins {points} {point_word}");
        if winner_is_a {
            out.push_str(&format!("      {win_text}\n"));
            score_a += points;
        } else {
            out.push_str(&format!("{:>35}{win_text}\n", ""));
            score_b += points;
        }
    }

    fs::write(path, out).map_err(|e| e.to_string())
}

fn grouped_turns(plies: &[PlyRecord]) -> Vec<(Option<String>, Option<String>)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < plies.len() {
        let p = &plies[i];
        let current_is_a = p.turn_a;
        let current_is_left = current_is_a;

        if current_is_left {
            let left = format_move(p);
            i += 1;
            let right = if i < plies.len() {
                let next = &plies[i];
                if next.turn_a != current_is_a {
                    i += 1;
                    Some(format_move(next))
                } else {
                    None
                }
            } else {
                None
            };
            out.push((Some(left), right));
        } else {
            let right = format_move(p);
            i += 1;
            out.push((None, Some(right)));
        }
    }

    out
}

fn format_move(ply: &PlyRecord) -> String {
    format!("{}{}: {}", ply.die_1, ply.die_2, ply.move_text)
}

fn variation_tag(variant: Variant) -> &'static str {
    match variant {
        Variant::Backgammon => "Backgammon",
        Variant::Nackgammon => "NackGammon",
        Variant::Longgammon => "Longgammon",
        Variant::Hypergammon => "HyperGammon (3)",
        Variant::Hypergammon2 => "HyperGammon (2)",
        Variant::Hypergammon4 => "HyperGammon (4)",
        Variant::Hypergammon5 => "HyperGammon (5)",
    }
}
