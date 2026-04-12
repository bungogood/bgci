use std::sync::OnceLock;
use std::{fs, path::PathBuf};

use bkgm::dice::Dice;
use bkgm::{Game, State, VariantPosition};

use super::runtime::{run_ubgi_loop, UbgiAdapter};

pub fn run(args: &[String]) -> Result<(), String> {
    let overrides = parse_pubeval_args(args)?;
    maybe_set_weight_overrides(overrides)?;
    let mut adapter = PubevalAdapter;
    run_ubgi_loop(&mut adapter);
    Ok(())
}

struct PubevalAdapter;

impl UbgiAdapter for PubevalAdapter {
    fn id_name(&self) -> &'static str {
        "pubeval_engine 1.0"
    }

    fn id_version(&self) -> &'static str {
        "1.0"
    }

    fn choose_move(&mut self, game: &Game, dice: Dice) -> Result<String, String> {
        let legal_moves = game
            .position()
            .legal_moves(dice)
            .map_err(|err| format!("move_encode {err}"))?;
        if legal_moves.is_empty() {
            return Err("no_encodable_legal_moves".to_string());
        }

        let race = is_race_position(game.position());
        let mover_is_x = game.position().turn();
        let w = weights();

        let mut best_idx = 0usize;
        let mut best_score =
            score_position(&to_pubeval_board(legal_moves[0].1, mover_is_x), race, w);
        for (idx, (_, pos)) in legal_moves.iter().enumerate().skip(1) {
            let board = to_pubeval_board(*pos, mover_is_x);
            let score = score_position(&board, race, w);
            if score > best_score {
                best_score = score;
                best_idx = idx;
            }
        }

        Ok(legal_moves[best_idx].0.to_string())
    }
}

struct Weights {
    race: [f32; 122],
    contact: [f32; 122],
}

#[derive(Default)]
struct PubevalArgs {
    race_path: Option<PathBuf>,
    contact_path: Option<PathBuf>,
}

fn weights() -> &'static Weights {
    static WEIGHTS: OnceLock<Weights> = OnceLock::new();
    WEIGHTS.get_or_init(|| {
        let overrides = WEIGHT_OVERRIDES.get();
        let race = overrides
            .and_then(|o| o.race_path.as_ref())
            .map(|path| load_weights_file(path, "WT.race"))
            .unwrap_or_else(|| parse_weights(include_str!("weights/WT.race"), "WT.race"));
        let contact = overrides
            .and_then(|o| o.contact_path.as_ref())
            .map(|path| load_weights_file(path, "WT.cntc"))
            .unwrap_or_else(|| parse_weights(include_str!("weights/WT.cntc"), "WT.cntc"));
        Weights { race, contact }
    })
}

static WEIGHT_OVERRIDES: OnceLock<PubevalArgs> = OnceLock::new();

fn maybe_set_weight_overrides(overrides: PubevalArgs) -> Result<(), String> {
    if overrides.race_path.is_none() && overrides.contact_path.is_none() {
        return Ok(());
    }
    WEIGHT_OVERRIDES
        .set(overrides)
        .map_err(|_| "pubeval weight overrides already initialized".to_string())
}

fn parse_pubeval_args(args: &[String]) -> Result<PubevalArgs, String> {
    let mut parsed = PubevalArgs::default();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--weights-race" => {
                let Some(path) = args.get(i + 1) else {
                    return Err("missing path after --weights-race".to_string());
                };
                parsed.race_path = Some(PathBuf::from(path));
                i += 2;
            }
            "--weights-contact" => {
                let Some(path) = args.get(i + 1) else {
                    return Err("missing path after --weights-contact".to_string());
                };
                parsed.contact_path = Some(PathBuf::from(path));
                i += 2;
            }
            "--weights-dir" => {
                let Some(path) = args.get(i + 1) else {
                    return Err("missing path after --weights-dir".to_string());
                };
                let dir = PathBuf::from(path);
                parsed.race_path = Some(dir.join("WT.race"));
                parsed.contact_path = Some(dir.join("WT.cntc"));
                i += 2;
            }
            flag => {
                return Err(format!(
                    "unknown pubeval arg '{flag}' (supported: --weights-race, --weights-contact, --weights-dir)"
                ));
            }
        }
    }
    Ok(parsed)
}

fn parse_weights(input: &str, label: &str) -> [f32; 122] {
    let mut out = [0.0f32; 122];
    let mut count = 0usize;
    for token in input.split_whitespace() {
        if count >= out.len() {
            panic!("{label} has too many weights");
        }
        out[count] = token
            .parse::<f32>()
            .unwrap_or_else(|_| panic!("{label} has invalid float '{token}'"));
        count += 1;
    }
    assert!(count == 122, "{label} expected 122 weights, got {count}");
    out
}

fn load_weights_file(path: &PathBuf, label: &str) -> [f32; 122] {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {label} '{}': {e}", path.display()));
    parse_weights(&content, label)
}

fn score_position(board: &[i32; 28], race: bool, w: &Weights) -> f32 {
    if board[26] == 15 {
        return 99_999_999.0;
    }

    let x = set_inputs(board);
    let weights = if race { &w.race } else { &w.contact };

    let mut score = 0.0f32;
    for i in 0..122 {
        score += weights[i] * x[i];
    }
    score
}

fn set_inputs(board: &[i32; 28]) -> [f32; 122] {
    let mut x = [0.0f32; 122];
    for j in 1..=24 {
        let idx = 5 * (j - 1);
        let n = board[25 - j];
        if n != 0 {
            if n == -1 {
                x[idx] = 1.0;
            }
            if n == 1 {
                x[idx + 1] = 1.0;
            }
            if n >= 2 {
                x[idx + 2] = 1.0;
            }
            if n == 3 {
                x[idx + 3] = 1.0;
            }
            if n >= 4 {
                x[idx + 4] = (n - 3) as f32 / 2.0;
            }
        }
    }
    x[120] = -(board[0] as f32) / 2.0;
    x[121] = board[26] as f32 / 15.0;
    x
}

fn is_race_position(position: VariantPosition) -> bool {
    match position {
        VariantPosition::Backgammon(p) => is_race_state(p),
        VariantPosition::Nackgammon(p) => is_race_state(p),
        VariantPosition::Longgammon(p) => is_race_state(p),
        VariantPosition::Hypergammon(p) => is_race_state(p),
        VariantPosition::Hypergammon2(p) => is_race_state(p),
        VariantPosition::Hypergammon4(p) => is_race_state(p),
        VariantPosition::Hypergammon5(p) => is_race_state(p),
    }
}

fn is_race_state<S: State>(p: S) -> bool {
    if p.x_bar() > 0 || p.o_bar() > 0 {
        return false;
    }

    let mut highest_x = 0usize;
    let mut lowest_o = 25usize;
    for pip in 1..=24 {
        let n = p.pip(pip);
        if n > 0 && pip > highest_x {
            highest_x = pip;
        }
        if n < 0 && pip < lowest_o {
            lowest_o = pip;
        }
    }

    if highest_x == 0 || lowest_o == 25 {
        return true;
    }
    highest_x < lowest_o
}

fn to_pubeval_board(position: VariantPosition, mover_is_x: bool) -> [i32; 28] {
    match position {
        VariantPosition::Backgammon(p) => encode_state(p, mover_is_x),
        VariantPosition::Nackgammon(p) => encode_state(p, mover_is_x),
        VariantPosition::Longgammon(p) => encode_state(p, mover_is_x),
        VariantPosition::Hypergammon(p) => encode_state(p, mover_is_x),
        VariantPosition::Hypergammon2(p) => encode_state(p, mover_is_x),
        VariantPosition::Hypergammon4(p) => encode_state(p, mover_is_x),
        VariantPosition::Hypergammon5(p) => encode_state(p, mover_is_x),
    }
}

fn encode_state<S: State>(p: S, mover_is_x: bool) -> [i32; 28] {
    let mut board = [0i32; 28];

    for j in 1..=24 {
        let src = if mover_is_x { j } else { 25 - j };
        let n = p.pip(src) as i32;
        board[j] = if mover_is_x { n } else { -n };
    }

    let (own_bar, opp_bar, own_off, opp_off) = if mover_is_x {
        (
            p.x_bar() as i32,
            p.o_bar() as i32,
            p.x_off() as i32,
            p.o_off() as i32,
        )
    } else {
        (
            p.o_bar() as i32,
            p.x_bar() as i32,
            p.o_off() as i32,
            p.x_off() as i32,
        )
    };
    board[25] = own_bar;
    board[0] = -opp_bar;
    board[26] = own_off;
    board[27] = -opp_off;

    board
}
