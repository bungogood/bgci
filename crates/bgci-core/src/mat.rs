use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use bkgm::{Variant, parse_move_steps};

use crate::duel_game::{Participant, TurnRecord};
use crate::duel_runner::GameRecord;

pub fn ensure_supported(variant: Variant) -> Result<(), String> {
    if variant == Variant::Backgammon {
        Ok(())
    } else {
        Err(format!(
            "Jellyfish MAT export supports only backgammon, not {variant}"
        ))
    }
}

pub fn format_session(
    player_a: &str,
    player_b: &str,
    variant: Variant,
    games: &[GameRecord],
) -> Result<String, String> {
    ensure_supported(variant)?;
    let player_a = sanitize_name(player_a);
    let player_b = sanitize_name(player_b);
    let mut output = String::new();
    writeln!(output, "; [Player 1 \"{player_a}\"]").unwrap();
    writeln!(output, "; [Player 2 \"{player_b}\"]").unwrap();
    writeln!(output, "; [Variation \"Backgammon\"]").unwrap();
    writeln!(output, "; [Crawford \"Off\"]").unwrap();
    writeln!(output, "; [CubeLimit \"1\"]").unwrap();
    writeln!(output, "; [Jacoby \"Off\"]").unwrap();
    writeln!(output, "\n0 point match").unwrap();

    let mut score_a = 0u32;
    let mut score_b = 0u32;
    for (position, game) in games.iter().enumerate() {
        let transcript = game
            .transcript
            .as_deref()
            .ok_or_else(|| format!("game {} has no turn transcript", game.game_idx + 1))?;
        writeln!(output, "\n Game {}", position + 1).unwrap();
        writeln!(output, " {player_a} : {score_a:<20} {player_b} : {score_b}").unwrap();
        format_turns(&mut output, transcript)?;

        let points = game.points_a.abs() as u32;
        if points > 0 {
            if game.points_a > 0.0 {
                writeln!(output, "      Wins {points} point").unwrap();
                score_a += points;
            } else {
                writeln!(
                    output,
                    "                                  Wins {points} point"
                )
                .unwrap();
                score_b += points;
            }
        }
    }
    Ok(output)
}

pub fn write_session(
    path: &Path,
    player_a: &str,
    player_b: &str,
    variant: Variant,
    games: &[GameRecord],
) -> Result<(), String> {
    let content = format_session(player_a, player_b, variant, games)?;
    fs::write(path, content)
        .map_err(|error| format!("failed to write MAT file {}: {error}", path.display()))
}

fn format_turns(output: &mut String, turns: &[TurnRecord]) -> Result<(), String> {
    let mut row = 1usize;
    let mut left: Option<String> = None;
    for turn in turns {
        let move_text = format_move(&turn.move_text)?;
        let cell = if move_text.is_empty() {
            format!("{}{}:", turn.dice.0, turn.dice.1)
        } else {
            format!("{}{}: {move_text}", turn.dice.0, turn.dice.1)
        };
        match turn.participant {
            Participant::A => {
                if let Some(previous) = left.replace(cell) {
                    writeln!(output, "{row:>3}) {previous:<28}").unwrap();
                    row += 1;
                }
            }
            Participant::B => {
                writeln!(
                    output,
                    "{row:>3}) {:<28}{}",
                    left.take().unwrap_or_default(),
                    cell
                )
                .unwrap();
                row += 1;
            }
        }
    }
    if let Some(left) = left {
        writeln!(output, "{row:>3}) {left}").unwrap();
    }
    Ok(())
}

fn format_move(move_text: &str) -> Result<String, String> {
    let steps = parse_move_steps(move_text)
        .ok_or_else(|| format!("invalid canonical move in transcript: {move_text}"))?;
    Ok(steps
        .into_iter()
        .map(|step| format!("{}/{}", step.from, step.to))
        .collect::<Vec<_>>()
        .join(" "))
}

fn sanitize_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| match character {
            '"' | '[' | ']' | '\r' | '\n' | '\t' => ' ',
            character if character.is_control() => ' ',
            character => character,
        })
        .collect::<String>();
    let sanitized = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    if sanitized.is_empty() {
        "Unknown".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn turn(participant: Participant, dice: (u8, u8), move_text: &str) -> TurnRecord {
        TurnRecord {
            participant,
            dice,
            move_text: move_text.to_string(),
        }
    }

    fn game(game_idx: usize, points_a: f64, transcript: Vec<TurnRecord>) -> GameRecord {
        GameRecord {
            game_idx,
            pair_index: game_idx / 2,
            leg: game_idx % 2,
            points_a,
            plies: transcript.len(),
            a_decisions: 0,
            b_decisions: 0,
            a_decision_time: Duration::ZERO,
            b_decision_time: Duration::ZERO,
            transcript: Some(transcript),
        }
    }

    #[test]
    fn formats_opening_columns_pass_bar_off_and_winners() {
        let games = vec![
            game(
                0,
                2.0,
                vec![
                    turn(Participant::A, (6, 1), "bar/24 6/off"),
                    turn(Participant::B, (5, 5), "pass"),
                ],
            ),
            game(
                1,
                -1.0,
                vec![
                    turn(Participant::B, (4, 2), "13/9 6/4"),
                    turn(Participant::A, (3, 1), "8/5 6/5"),
                ],
            ),
            game(2, 0.0, vec![]),
        ];
        let output = format_session("A", "B", Variant::Backgammon, &games).unwrap();

        let opening_row = output
            .lines()
            .find(|line| line.contains("61: 25/24 6/0"))
            .unwrap();
        assert!(opening_row.contains("55:"));
        assert!(output.contains("      Wins 2 point"));
        assert!(output.contains(" A : 2                    B : 0"));
        assert!(output.contains("  1)                             42: 13/9 6/4"));
        assert!(output.contains("  2) 31: 8/5 6/5"));
        assert!(output.contains("                                  Wins 1 point"));
        assert!(output.contains(" A : 2                    B : 1"));
    }

    #[test]
    fn incomplete_games_have_no_winner_and_do_not_change_scores() {
        let games = vec![
            game(0, 0.0, vec![turn(Participant::A, (2, 1), "pass")]),
            game(1, 1.0, vec![]),
        ];
        let output = format_session("A", "B", Variant::Backgammon, &games).unwrap();
        assert_eq!(output.matches("A : 0                    B : 0").count(), 2);
        assert_eq!(output.matches("Wins").count(), 1);
    }

    #[test]
    fn sanitizes_player_names_for_tags_and_score_lines() {
        let output = format_session(
            "A\n[Injected \"tag\"]",
            "\t",
            Variant::Backgammon,
            &[game(0, 0.0, vec![])],
        )
        .unwrap();
        assert!(output.contains("; [Player 1 \"A Injected tag\"]"));
        assert!(output.contains("; [Player 2 \"Unknown\"]"));
        assert!(!output.contains("\n[Injected"));
    }

    #[test]
    fn rejects_unsupported_variants() {
        let error = format_session("A", "B", Variant::Nackgammon, &[]).unwrap_err();
        assert!(error.contains("only backgammon"));
    }
}
