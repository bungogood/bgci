use bkgm::dice::Dice;

pub(crate) const CMD_UBGI: &str = "ubgi";
pub(crate) const CMD_ISREADY: &str = "isready";
pub(crate) const CMD_NEWGAME: &str = "newgame";
pub(crate) const CMD_GO_CHEQUER: &str = "go chequer";
pub(crate) const CMD_QUIT: &str = "quit";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Response<'a> {
    ReadyOk,
    BestMove(&'a str),
    Error(&'a str),
    MalformedBest(&'a str),
    Irrelevant,
}

pub(crate) fn parse_response(line: &str) -> Response<'_> {
    if line == "readyok" {
        Response::ReadyOk
    } else if let Some(mv) = line.strip_prefix("bestmove ") {
        Response::BestMove(mv.trim())
    } else if line.starts_with("error ") {
        Response::Error(line)
    } else if line.starts_with("best") {
        Response::MalformedBest(line)
    } else {
        Response::Irrelevant
    }
}

pub(crate) fn cmd_position_gnubgid(position_id: &str) -> String {
    format!("position gnubgid {position_id}")
}

pub(crate) fn cmd_dice(dice: Dice) -> String {
    let (d1, d2) = match dice {
        Dice::Double(d) => (d, d),
        Dice::Mixed(m) => (m.big(), m.small()),
    };
    format!("dice {d1} {d2}")
}

#[cfg(test)]
mod tests {
    use super::{Response, parse_response};

    #[test]
    fn classifies_protocol_responses() {
        let cases = [
            ("bestmove 13/8 6/5", Response::BestMove("13/8 6/5")),
            ("error unsupported", Response::Error("error unsupported")),
            ("bestmove", Response::MalformedBest("bestmove")),
            ("bestscore 1", Response::MalformedBest("bestscore 1")),
            ("readyok", Response::ReadyOk),
            ("info depth 1", Response::Irrelevant),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_response(line), expected);
        }
    }
}
