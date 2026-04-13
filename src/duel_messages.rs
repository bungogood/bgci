use crate::duel_game::DuelGameResult;

pub(crate) enum WorkerMessage {
    Game(CompletedGame),
    Error(String),
    Done,
}

pub(crate) struct CompletedGame {
    pub(crate) game_idx: usize,
    pub(crate) a_is_x: bool,
    pub(crate) result: DuelGameResult,
}
