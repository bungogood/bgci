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
        let legal_positions = game.legal_positions(&dice);
        if legal_positions.is_empty() {
            return Err("no_encodable_legal_moves".to_string());
        }
        let index = fastrand::usize(..legal_positions.len());
        game.position()
            .encode_move(legal_positions[index], dice)
            .map_err(|err| format!("move_encode {err}"))
    }
}
