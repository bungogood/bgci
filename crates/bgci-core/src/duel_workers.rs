use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bkgm::Variant;
use bkgm::dice_gen::FastrandDice;
use tokio::sync::mpsc;
use tokio::task;
use tokio::task::JoinHandle;

use crate::config::ResolvedEngine;
use crate::duel_game::{DuelGameResult, play_game, seed_for_game};
use crate::engine::EngineProcess;

pub(crate) enum WorkerMessage {
    Game {
        game_idx: usize,
        result: DuelGameResult,
    },
    Error(String),
}

#[derive(Clone)]
pub struct LocalWorkerSpec {
    pub workers: usize,
    pub pairs: usize,
    pub variant: Variant,
    pub max_plies: usize,
    pub base_seed: u64,
    pub engine_a: ResolvedEngine,
    pub engine_b: ResolvedEngine,
    pub cancel: Arc<AtomicBool>,
}

pub(crate) fn spawn_local_workers(
    spec: LocalWorkerSpec,
    tx: mpsc::UnboundedSender<WorkerMessage>,
) -> Vec<JoinHandle<()>> {
    let worker_count = spec.workers;
    (0..worker_count)
        .map(|worker_id| {
            let tx = tx.clone();
            let spec = spec.clone();

            task::spawn_blocking(move || {
                if let Err(error) = run_worker(worker_id, &spec, &tx) {
                    report_error(&spec.cancel, &tx, error);
                }
            })
        })
        .collect()
}

fn run_worker(
    worker_id: usize,
    spec: &LocalWorkerSpec,
    tx: &mpsc::UnboundedSender<WorkerMessage>,
) -> Result<(), String> {
    let worker = worker_id + 1;
    let mut engine_a = EngineProcess::spawn(&spec.engine_a)
        .map_err(|error| format!("worker {worker} failed to spawn engine A: {error}"))?;
    let mut engine_b = EngineProcess::spawn(&spec.engine_b)
        .map_err(|error| format!("worker {worker} failed to spawn engine B: {error}"))?;
    engine_a
        .init_ubgi()
        .and_then(|()| engine_b.init_ubgi())
        .and_then(|()| engine_a.set_variant(spec.variant))
        .and_then(|()| engine_b.set_variant(spec.variant))
        .map_err(|error| format!("worker {worker} engine init failed: {error}"))?;

    'pairs: for pair_idx in (worker_id..spec.pairs).step_by(spec.workers) {
        for leg in 0..2 {
            if spec.cancel.load(Ordering::Relaxed) {
                break 'pairs;
            }
            let game_idx = pair_idx * 2 + leg;
            engine_a.new_game().map_err(|error| {
                format!(
                    "worker {worker} game {} new_game(A) failed: {error}",
                    game_idx + 1
                )
            })?;
            engine_b.new_game().map_err(|error| {
                format!(
                    "worker {worker} game {} new_game(B) failed: {error}",
                    game_idx + 1
                )
            })?;
            let mut dice_gen = FastrandDice::with_seed(seed_for_game(spec.base_seed, pair_idx));
            let result = play_game(
                spec.variant,
                spec.max_plies,
                &mut dice_gen,
                &mut engine_a,
                &mut engine_b,
                leg == 0,
            )
            .map_err(|error| format!("worker {worker} game {} failed: {error}", game_idx + 1))?;
            let _ = tx.send(WorkerMessage::Game { game_idx, result });
        }
    }

    engine_a.quit();
    engine_b.quit();
    Ok(())
}

fn report_error(cancel: &AtomicBool, tx: &mpsc::UnboundedSender<WorkerMessage>, message: String) {
    cancel.store(true, Ordering::Relaxed);
    let _ = tx.send(WorkerMessage::Error(message));
}
