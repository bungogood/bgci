use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bgci::engines;
use serde::Deserialize;
use serde::de::{self, Deserializer};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DuelConfig {
    pub games: usize,
    pub parallel: usize,
    pub seed: u64,
    pub max_plies: usize,
    pub swap_sides: bool,
    pub variant: String,
    pub log: String,
    pub timeout_secs: Option<u64>,
    pub output_csv: Option<String>,
    pub output_mat: Option<String>,
    pub output_log: Option<String>,
    pub output_traces_dir: Option<String>,
    pub engine_a: EngineConfig,
    pub engine_b: EngineConfig,
}

impl Default for DuelConfig {
    fn default() -> Self {
        Self {
            games: 20,
            parallel: 1,
            seed: 42,
            max_plies: 512,
            swap_sides: true,
            variant: "backgammon".to_string(),
            log: "off".to_string(),
            timeout_secs: None,
            output_csv: None,
            output_mat: None,
            output_log: None,
            output_traces_dir: None,
            engine_a: EngineConfig::default_a(),
            engine_b: EngineConfig::default_b(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EngineConfig {
    pub name: String,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(deserialize_with = "deserialize_command")]
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct EngineAliasDetail {
    pub name: String,
    pub source: String,
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl EngineConfig {
    fn default_a() -> Self {
        Self {
            name: "random-a".to_string(),
            engine: Some("random".to_string()),
            command: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    fn default_b() -> Self {
        Self {
            name: "random-b".to_string(),
            engine: Some("random".to_string()),
            command: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CommandField {
    Single(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
struct EngineTemplate {
    #[serde(deserialize_with = "deserialize_command")]
    command: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UserConfig {
    engines: BTreeMap<String, EngineTemplate>,
}

fn deserialize_command<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match CommandField::deserialize(deserializer)? {
        CommandField::Single(cmd) => {
            if cmd.trim().is_empty() {
                return Err(de::Error::custom("engine command cannot be empty"));
            }
            Ok(vec![cmd])
        }
        CommandField::Many(cmds) => {
            if cmds.is_empty() {
                return Err(de::Error::custom("engine command cannot be empty"));
            }
            Ok(cmds)
        }
    }
}

pub fn resolve_engine_shortcuts(cfg: &mut DuelConfig) -> Result<(), String> {
    let registry = load_user_engine_registry()?;
    resolve_engine_alias(&mut cfg.engine_a, &registry)?;
    resolve_engine_alias(&mut cfg.engine_b, &registry)?;
    Ok(())
}

pub fn resolve_engine_reference(alias: &str) -> Result<EngineConfig, String> {
    let registry = load_user_engine_registry()?;
    let mut engine = EngineConfig {
        name: alias.to_string(),
        engine: Some(alias.to_string()),
        command: Vec::new(),
        env: BTreeMap::new(),
    };
    resolve_engine_alias(&mut engine, &registry)?;
    Ok(engine)
}

fn resolve_engine_alias(
    engine: &mut EngineConfig,
    registry: &BTreeMap<String, EngineTemplate>,
) -> Result<(), String> {
    let has_engine_ref = engine.engine.is_some();
    let has_command = !engine.command.is_empty();

    if has_engine_ref && has_command {
        return Err(format!(
            "engine '{}' has both 'engine' and 'command'; choose one",
            engine.name
        ));
    }
    if !has_engine_ref && !has_command {
        return Err(format!(
            "engine '{}' must set either 'engine' or 'command'",
            engine.name
        ));
    }
    if has_command {
        expand_tilde_in_command(&mut engine.command);
        return Ok(());
    }

    let alias = engine
        .engine
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if alias.is_empty() {
        return Err(format!(
            "engine '{}' has empty 'engine' reference",
            engine.name
        ));
    }

    if let Some(kind) = engines::builtin_engine_name(&alias) {
        set_builtin_engine_command(engine, kind)?;
        expand_tilde_in_command(&mut engine.command);
        return Ok(());
    }

    if let Some(template) = registry.get(&alias) {
        engine.command = template.command.clone();
        engine.engine = None;
        let mut merged_env = template.env.clone();
        for (key, value) in &engine.env {
            merged_env.insert(key.clone(), value.clone());
        }
        engine.env = merged_env;

        if engine.command.len() == 1 {
            let nested_alias = engine.command[0].trim().to_ascii_lowercase();
            if let Some(kind) = engines::builtin_engine_name(&nested_alias) {
                set_builtin_engine_command(engine, kind)?;
            }
        }

        expand_tilde_in_command(&mut engine.command);

        return Ok(());
    }

    Err(format!(
        "engine '{}' references unknown engine alias '{}'",
        engine.name, alias
    ))
}

fn builtin_engine_names() -> Vec<String> {
    engines::BUILTIN_ENGINE_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

fn set_builtin_engine_command(engine: &mut EngineConfig, kind: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("resolve current executable: {e}"))?;
    engine.command = vec![
        exe.to_string_lossy().into_owned(),
        "engine".to_string(),
        kind.to_string(),
    ];
    engine.engine = None;
    Ok(())
}

fn load_user_engine_registry() -> Result<BTreeMap<String, EngineTemplate>, String> {
    let path = if let Some(explicit) = std::env::var_os("BGCI_CONFIG") {
        Some(PathBuf::from(explicit))
    } else {
        locate_user_config_path()
    };

    let Some(path) = path else {
        return Ok(BTreeMap::new());
    };

    if !path.exists() {
        return Err(format!("config file not found: {}", path.display()));
    }

    let content =
        fs::read_to_string(&path).map_err(|e| format!("read config {}: {e}", path.display()))?;
    let parsed: UserConfig =
        toml::from_str(&content).map_err(|e| format!("parse config {}: {e}", path.display()))?;

    Ok(parsed
        .engines
        .into_iter()
        .map(|(name, template)| (name.to_ascii_lowercase(), template))
        .collect())
}

fn locate_user_config_path() -> Option<PathBuf> {
    if let Some(xdg_home) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg_home).join("bgci/config.toml");
        if path.exists() {
            return Some(path);
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home).join(".config/bgci/config.toml");
        if path.exists() {
            return Some(path);
        }
    }

    None
}

pub fn list_engine_aliases() -> Result<Vec<String>, String> {
    let mut names = builtin_engine_names();
    let registry = load_user_engine_registry()?;
    names.extend(registry.keys().cloned());
    names.sort();
    names.dedup();
    Ok(names)
}

pub fn list_engine_alias_details() -> Result<Vec<EngineAliasDetail>, String> {
    let mut by_name = BTreeMap::new();
    for name in builtin_engine_names() {
        by_name.insert(
            name.clone(),
            EngineAliasDetail {
                name: name.clone(),
                source: "builtin".to_string(),
                command: builtin_display_command(&name),
                env: BTreeMap::new(),
            },
        );
    }

    let registry = load_user_engine_registry()?;
    for (name, template) in registry {
        let mut command = template.command;
        expand_tilde_in_command(&mut command);
        if command.len() == 1 {
            let nested_alias = command[0].trim().to_ascii_lowercase();
            if let Some(kind) = engines::builtin_engine_name(&nested_alias) {
                command = builtin_display_command(kind);
            }
        }
        by_name.insert(
            name.clone(),
            EngineAliasDetail {
                name,
                source: "user".to_string(),
                command,
                env: template.env,
            },
        );
    }

    let mut details: Vec<_> = by_name.into_values().collect();
    details.sort_by(|a, b| match (a.source.as_str(), b.source.as_str()) {
        ("builtin", "user") => std::cmp::Ordering::Less,
        ("user", "builtin") => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    Ok(details)
}

fn builtin_display_command(kind: &str) -> Vec<String> {
    vec!["bgci".to_string(), "engine".to_string(), kind.to_string()]
}

fn expand_tilde_in_command(command: &mut [String]) {
    for token in command {
        *token = shellexpand::full(token)
            .map(|expanded| expanded.into_owned())
            .unwrap_or_else(|_| token.clone());
    }
}

pub fn load_toml<T: for<'de> Deserialize<'de>>(path: impl AsRef<Path>) -> Result<T, String> {
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    toml::from_str(&content).map_err(|e| e.to_string())
}
