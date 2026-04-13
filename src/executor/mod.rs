use crate::domain::MatchPlan;
use crate::duel_runner::{run_duel, run_duel_with_progress, ProgressSnapshot, RunSummary};
use crate::output_paths::RunPaths;

pub trait DuelExecutor {
    fn execute(&self, plan: &MatchPlan, paths: &RunPaths) -> Result<RunSummary, String>;

    #[allow(dead_code)]
    fn execute_with_progress<F>(
        &self,
        plan: &MatchPlan,
        paths: &RunPaths,
        on_game_done: F,
    ) -> Result<RunSummary, String>
    where
        F: FnMut(&ProgressSnapshot) -> Result<(), String>,
    {
        let _ = on_game_done;
        self.execute(plan, paths)
    }
}

pub struct LocalThreadExecutor;

impl DuelExecutor for LocalThreadExecutor {
    fn execute(&self, plan: &MatchPlan, paths: &RunPaths) -> Result<RunSummary, String> {
        run_duel(&plan.config, plan.variant, paths)
    }

    fn execute_with_progress<F>(
        &self,
        plan: &MatchPlan,
        paths: &RunPaths,
        on_game_done: F,
    ) -> Result<RunSummary, String>
    where
        F: FnMut(&ProgressSnapshot) -> Result<(), String>,
    {
        run_duel_with_progress(&plan.config, plan.variant, paths, on_game_done)
    }
}
