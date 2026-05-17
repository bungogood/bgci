use bkgm::dice::Dice;
use bkgm::{encode_move_steps, Game};

use super::runtime::{run_ubgi_stdio, UbgiEngine, UbgiError, UbgiMove};

pub fn run(_args: &[String]) -> Result<(), String> {
    let mut adapter = RandomAdapter;
    run_ubgi_stdio(&mut adapter);
    Ok(())
}

struct RandomAdapter;

impl UbgiEngine for RandomAdapter {
    fn id_name(&self) -> &'static str {
        "random_engine 0.1"
    }

    fn id_version(&self) -> &'static str {
        "0.1"
    }

    fn choose_move(&mut self, game: &Game, dice: Dice) -> Result<UbgiMove, UbgiError> {
        let legal_positions = game.legal_positions(&dice);
        if legal_positions.is_empty() {
            return Err(UbgiError::bad_state("no_encodable_legal_moves"));
        }
        let index = fastrand::usize(..legal_positions.len());
        encode_move_steps(game.position(), legal_positions[index], dice)
            .map_err(|err| UbgiError::bad_state(format!("move_encode {err}")))
    }
}
