use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bkgm::Variant;
use bkgm::dice_gen::FastrandDice;
use tokio::sync::mpsc;
use tokio::task;
use tokio::task::JoinHandle;

use crate::config::ResolvedEngine;
use crate::duel_game::{DuelGameResult, play_game, seed_for_game, singleton_leg};
use crate::engine::EngineProcess;

pub(crate) enum WorkerMessage {
    Game {
        game_idx: usize,
        pair_index: usize,
        leg: usize,
        result: DuelGameResult,
    },
    Error(String),
}

#[derive(Clone)]
pub(crate) struct LocalWorkerSpec {
    pub(crate) workers: usize,
    pub(crate) games: usize,
    pub(crate) variant: Variant,
    pub(crate) max_plies: usize,
    pub(crate) base_seed: u64,
    pub(crate) engine_a: ResolvedEngine,
    pub(crate) engine_b: ResolvedEngine,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) record_transcripts: bool,
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

    let cluster_count = spec.games.div_ceil(2);
    'clusters: for pair_index in (worker_id..cluster_count).step_by(spec.workers) {
        let first_game_idx = pair_index * 2;
        let legs = if first_game_idx + 1 < spec.games {
            [Some(0), Some(1)]
        } else {
            [Some(singleton_leg(spec.base_seed, pair_index)), None]
        };
        for (offset, leg) in legs.into_iter().flatten().enumerate() {
            if spec.cancel.load(Ordering::Relaxed) {
                break 'clusters;
            }
            let game_idx = first_game_idx + offset;
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
            let mut dice_gen = FastrandDice::with_seed(seed_for_game(spec.base_seed, pair_index));
            let result = play_game(
                spec.variant,
                spec.max_plies,
                &mut dice_gen,
                &mut engine_a,
                &mut engine_b,
                leg == 0,
                spec.record_transcripts,
            )
            .map_err(|error| format!("worker {worker} game {} failed: {error}", game_idx + 1))?;
            let _ = tx.send(WorkerMessage::Game {
                game_idx,
                pair_index,
                leg,
                result,
            });
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
