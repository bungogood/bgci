use bkgm::Variant;

use crate::config::DuelConfig;

#[derive(Clone)]
pub struct MatchPlan {
    pub config: DuelConfig,
    pub variant: Variant,
}
