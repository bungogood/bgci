use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DuelConfig {
    pub games: usize,
    pub seed: u64,
    pub max_plies: usize,
    pub swap_sides: bool,
    pub variant: String,
    pub log: String,
    pub engine_a: EngineConfig,
    pub engine_b: EngineConfig,
}

impl Default for DuelConfig {
    fn default() -> Self {
        Self {
            games: 20,
            seed: 42,
            max_plies: 512,
            swap_sides: true,
            variant: "backgammon".to_string(),
            log: "off".to_string(),
            engine_a: EngineConfig::default_a(),
            engine_b: EngineConfig::default_b(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EngineConfig {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl EngineConfig {
    fn default_a() -> Self {
        Self {
            name: "random-a".to_string(),
            command: vec![
                "cargo".to_string(),
                "run".to_string(),
                "--quiet".to_string(),
                "--bin".to_string(),
                "random_engine".to_string(),
            ],
            env: BTreeMap::new(),
        }
    }

    fn default_b() -> Self {
        Self {
            name: "random-b".to_string(),
            command: vec![
                "cargo".to_string(),
                "run".to_string(),
                "--quiet".to_string(),
                "--bin".to_string(),
                "random_engine".to_string(),
            ],
            env: BTreeMap::new(),
        }
    }
}

pub fn load_toml<T: for<'de> Deserialize<'de>>(path: impl AsRef<Path>) -> Result<T, String> {
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    toml::from_str(&content).map_err(|e| e.to_string())
}
