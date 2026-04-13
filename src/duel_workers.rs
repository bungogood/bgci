use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bkgm::dice_gen::FastrandDice;
use bkgm::Variant;
use tokio::sync::mpsc;
use tokio::task;

use crate::config::EngineConfig;
use crate::duel_game::{play_game, seed_for_game};
use crate::duel_messages::{CompletedGame, WorkerMessage};
use crate::engine::EngineProcess;

pub struct LocalWorkerSpec {
    pub workers: usize,
    pub games: usize,
    pub variant: Variant,
    pub max_plies: usize,
    pub swap_sides: bool,
    pub base_seed: u64,
    pub engine_a: EngineConfig,
    pub engine_b: EngineConfig,
    pub cancel: Arc<AtomicBool>,
}

pub fn spawn_local_workers(spec: LocalWorkerSpec, tx: mpsc::UnboundedSender<WorkerMessage>) {
    let worker_count = spec.workers;
    for worker_id in 0..worker_count {
        let tx = tx.clone();
        let worker_variant = spec.variant;
        let engine_a_cfg = spec.engine_a.clone();
        let engine_b_cfg = spec.engine_b.clone();
        let cancel = spec.cancel.clone();
        let max_plies = spec.max_plies;
        let swap_sides = spec.swap_sides;
        let base_seed = spec.base_seed;
        let games = spec.games;

        task::spawn_blocking(move || {
            let mut engine_a = match EngineProcess::spawn(&engine_a_cfg) {
                Ok(e) => e,
                Err(err) => {
                    let _ = tx.send(WorkerMessage::Error(format!(
                        "worker {} failed to spawn engine A: {}",
                        worker_id + 1,
                        err
                    )));
                    let _ = tx.send(WorkerMessage::Done);
                    return;
                }
            };
            let mut engine_b = match EngineProcess::spawn(&engine_b_cfg) {
                Ok(e) => e,
                Err(err) => {
                    let _ = tx.send(WorkerMessage::Error(format!(
                        "worker {} failed to spawn engine B: {}",
                        worker_id + 1,
                        err
                    )));
                    let _ = tx.send(WorkerMessage::Done);
                    return;
                }
            };

            let init_result = (|| -> Result<(), String> {
                engine_a.init_ubgi()?;
                engine_b.init_ubgi()?;
                engine_a.set_variant(worker_variant)?;
                engine_b.set_variant(worker_variant)?;
                Ok(())
            })();

            if let Err(err) = init_result {
                let _ = tx.send(WorkerMessage::Error(format!(
                    "worker {} engine init failed: {}",
                    worker_id + 1,
                    err
                )));
                engine_a.quit();
                engine_b.quit();
                let _ = tx.send(WorkerMessage::Done);
                return;
            }

            for game_idx in (worker_id..games).step_by(worker_count) {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                let a_is_x = !(swap_sides && game_idx % 2 == 1);
                if let Err(err) = engine_a.new_game() {
                    let _ = tx.send(WorkerMessage::Error(format!(
                        "worker {} game {} new_game(A) failed: {}",
                        worker_id + 1,
                        game_idx + 1,
                        err
                    )));
                    break;
                }
                if let Err(err) = engine_b.new_game() {
                    let _ = tx.send(WorkerMessage::Error(format!(
                        "worker {} game {} new_game(B) failed: {}",
                        worker_id + 1,
                        game_idx + 1,
                        err
                    )));
                    break;
                }

                let mut dice_gen = FastrandDice::with_seed(seed_for_game(base_seed, game_idx));
                match play_game(
                    worker_variant,
                    max_plies,
                    &mut dice_gen,
                    &mut engine_a,
                    &mut engine_b,
                    a_is_x,
                ) {
                    Ok(result) => {
                        let _ = tx.send(WorkerMessage::Game(CompletedGame {
                            game_idx,
                            a_is_x,
                            result,
                        }));
                    }
                    Err(err) => {
                        let _ = tx.send(WorkerMessage::Error(format!(
                            "worker {} game {} failed: {}",
                            worker_id + 1,
                            game_idx + 1,
                            err
                        )));
                        break;
                    }
                }
            }

            engine_a.quit();
            engine_b.quit();
            let _ = tx.send(WorkerMessage::Done);
        });
    }
}
