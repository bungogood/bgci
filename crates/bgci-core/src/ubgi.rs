use bkgm::dice::Dice;
use bkgm::ubgi::KeyLineSpec;

pub const CMD_UBGI: &str = "ubgi";
pub const CMD_ISREADY: &str = "isready";
pub const CMD_NEWGAME: &str = "newgame";
pub const CMD_GO_CHEQUER: &str = "go chequer";
pub const CMD_QUIT: &str = "quit";

pub fn cmd_set(key: &str, value: &str) -> String {
    format!("set {key} {value}")
}

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

pub enum LineKind<'a> {
    Id,
    Key(KeyLineSpec),
    ReadyOk,
    UbgiOk,
    BestMove(&'a str),
    Error,
    BestOther,
    Other,
}

pub fn classify_line(line: &str) -> LineKind<'_> {
    if line == "readyok" {
        return LineKind::ReadyOk;
    }
    if line == "ubgiok" {
        return LineKind::UbgiOk;
    }
    if line.starts_with("id ") {
        return LineKind::Id;
    }
    if let Some(spec) = bkgm::ubgi::parse_key_line(line) {
        return LineKind::Key(spec);
    }
    if let Some(mv) = line.strip_prefix("bestmove ") {
        return LineKind::BestMove(mv.trim());
    }
    if line.starts_with("best") {
        return LineKind::BestOther;
    }
    if line.starts_with("error ") {
        return LineKind::Error;
    }
    LineKind::Other
}
