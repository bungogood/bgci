use bkgm::dice::Dice;

pub const CMD_UBGI: &str = "ubgi";
pub const CMD_ISREADY: &str = "isready";
pub const CMD_NEWGAME: &str = "newgame";
pub const CMD_GO_CHEQUER: &str = "go chequer";
pub const CMD_QUIT: &str = "quit";

pub fn cmd_position_gnubgid(position_id: &str) -> String {
    format!("position gnubgid {position_id}")
}

pub fn cmd_dice(dice: Dice) -> String {
    let (d1, d2) = match dice {
        Dice::Double(d) => (d, d),
        Dice::Mixed(m) => (m.big(), m.small()),
    };
    format!("dice {d1} {d2}")
}
