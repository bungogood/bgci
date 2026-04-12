use bkgm::dice::Dice;
use bkgm::Game;

use super::runtime::{run_ubgi_loop, UbgiAdapter};

pub fn run(_args: &[String]) -> Result<(), String> {
    let mut adapter = RandomAdapter;
    run_ubgi_loop(&mut adapter);
    Ok(())
}

struct RandomAdapter;

impl UbgiAdapter for RandomAdapter {
    fn id_name(&self) -> &'static str {
        "random_engine 0.1"
    }

    fn id_version(&self) -> &'static str {
        "0.1"
    }

    fn choose_move(&mut self, game: &Game, dice: Dice) -> Result<String, String> {
        let legal_moves = game
            .position()
            .legal_moves(dice)
            .map_err(|err| format!("move_encode {err}"))?;
        if legal_moves.is_empty() {
            return Err("no_encodable_legal_moves".to_string());
        }
        let index = fastrand::usize(..legal_moves.len());
        Ok(legal_moves[index].0.to_string())
    }
}
