use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::engines;
use bkgm::{EngineSpec, format_engine_spec, parse_engine_spec};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MatchupConfig {
    pub pairs: usize,
    pub parallel: usize,
    pub seed: u64,
    pub max_plies: usize,
    pub variant: String,
    pub log_level: String,
    pub engine_a: EngineConfig,
    pub engine_b: EngineConfig,
}

impl Default for MatchupConfig {
    fn default() -> Self {
        Self {
            pairs: 10,
            parallel: 1,
            seed: 42,
            max_plies: 512,
            variant: "backgammon".to_string(),
            log_level: "off".to_string(),
            engine_a: EngineConfig::default_a(),
            engine_b: EngineConfig::default_b(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineConfig {
    pub name: String,
    #[serde(default, skip_serializing)]
    pub family: Option<String>,
    #[serde(default, skip_serializing)]
    pub version: Option<String>,
    #[serde(default, skip_serializing)]
    pub configuration: BTreeMap<String, String>,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(deserialize_with = "deserialize_command")]
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct EngineAliasDetail {
    pub name: String,
    pub family: Option<String>,
    pub version: Option<String>,
    pub configuration: BTreeMap<String, String>,
    pub url: Option<String>,
    pub source: String,
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub options: BTreeMap<String, String>,
}

impl EngineConfig {
    fn default_a() -> Self {
        Self {
            name: "random-a".to_string(),
            family: None,
            version: None,
            configuration: BTreeMap::new(),
            engine: Some("random".to_string()),
            command: Vec::new(),
            env: BTreeMap::new(),
            options: BTreeMap::new(),
        }
    }

    fn default_b() -> Self {
        Self {
            name: "random-b".to_string(),
            family: None,
            version: None,
            configuration: BTreeMap::new(),
            engine: Some("random".to_string()),
            command: Vec::new(),
            env: BTreeMap::new(),
            options: BTreeMap::new(),
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
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    configuration: BTreeMap<String, String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(deserialize_with = "deserialize_command")]
    command: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    options: BTreeMap<String, String>,
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

pub fn resolve_engine_shortcuts(cfg: &mut MatchupConfig) -> Result<(), String> {
    let registry = load_user_engine_registry()?;
    resolve_engine_alias(&mut cfg.engine_a, &registry)?;
    resolve_engine_alias(&mut cfg.engine_b, &registry)?;
    Ok(())
}

pub fn resolve_engine_reference(alias: &str) -> Result<EngineConfig, String> {
    let registry = load_user_engine_registry()?;
    let mut engine = EngineConfig {
        name: alias.to_string(),
        family: None,
        version: None,
        configuration: BTreeMap::new(),
        engine: Some(alias.to_string()),
        command: Vec::new(),
        env: BTreeMap::new(),
        options: BTreeMap::new(),
    };
    resolve_engine_alias(&mut engine, &registry)?;
    Ok(engine)
}

pub fn resolve_engine_spec(spec: &str) -> Result<(String, EngineConfig), String> {
    let parsed = parse_engine_spec(spec).map_err(|err| err.to_string())?;

    let (engine_ref, resolved_version) =
        resolve_engine_ref_for_spec(&parsed.alias, parsed.version.as_deref())?;
    let mut cfg = resolve_engine_reference(&engine_ref)?;
    let base_options = cfg.options.clone();
    for (k, v) in parsed.options {
        cfg.options.insert(k, v);
    }

    let identity_options = non_default_options(&cfg.options, &base_options);
    let key = format_engine_spec(&EngineSpec {
        alias: parsed.alias.clone(),
        version: resolved_version,
        options: identity_options,
    });
    Ok((key, cfg))
}

pub fn engine_identity_from_spec_with_options(
    spec: &str,
    effective_options: &BTreeMap<String, String>,
) -> Result<String, String> {
    let parsed = parse_engine_spec(spec).map_err(|err| err.to_string())?;
    let (engine_ref, resolved_version) =
        resolve_engine_ref_for_spec(&parsed.alias, parsed.version.as_deref())?;
    let base = resolve_engine_reference(&engine_ref)?;
    let identity_options = non_default_options(effective_options, &base.options);
    Ok(format_engine_spec(&EngineSpec {
        alias: parsed.alias,
        version: resolved_version,
        options: identity_options,
    }))
}

fn resolve_engine_ref_for_spec(
    alias: &str,
    version: Option<&str>,
) -> Result<(String, Option<String>), String> {
    if let Some(v) = version {
        return Ok((format!("{}@{}", alias, v), Some(v.to_string())));
    }

    let registry = load_user_engine_registry()?;
    let prefix = format!("{}@", alias.to_ascii_lowercase());
    let mut candidates: Vec<String> = registry
        .keys()
        .filter(|k| k.starts_with(&prefix))
        .cloned()
        .collect();
    if candidates.is_empty() {
        return Ok((alias.to_string(), None));
    }
    candidates.sort_by(|a, b| compare_versioned_alias(a, b));
    let selected = candidates
        .last()
        .cloned()
        .unwrap_or_else(|| alias.to_string());
    let selected_version = selected.split_once('@').map(|(_, v)| v.to_string());
    Ok((selected, selected_version))
}

fn compare_versioned_alias(a: &str, b: &str) -> std::cmp::Ordering {
    let va = a.split_once('@').map(|(_, v)| v).unwrap_or("");
    let vb = b.split_once('@').map(|(_, v)| v).unwrap_or("");
    compare_version_strings(va, vb)
}

fn compare_version_strings(a: &str, b: &str) -> std::cmp::Ordering {
    let pa: Vec<u64> = a
        .trim_start_matches('v')
        .split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect();
    let pb: Vec<u64> = b
        .trim_start_matches('v')
        .split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect();
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let av = *pa.get(i).unwrap_or(&0);
        let bv = *pb.get(i).unwrap_or(&0);
        match av.cmp(&bv) {
            std::cmp::Ordering::Equal => {}
            ord => return ord,
        }
    }
    a.cmp(b)
}

fn non_default_options(
    effective: &BTreeMap<String, String>,
    defaults: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in effective {
        if defaults.get(k) != Some(v) {
            out.insert(k.clone(), v.clone());
        }
    }
    out
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
        if engine.family.is_none() {
            engine.family = template.family.clone();
        }
        if engine.version.is_none() {
            engine.version = template.version.clone();
        }
        if engine.configuration.is_empty() {
            engine.configuration = template.configuration.clone();
        }
        let mut merged_env = template.env.clone();
        for (key, value) in &engine.env {
            merged_env.insert(key.clone(), value.clone());
        }
        engine.env = merged_env;
        let mut merged_options = template.options.clone();
        for (key, value) in &engine.options {
            merged_options.insert(key.clone(), value.clone());
        }
        engine.options = merged_options;

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
                family: None,
                version: None,
                configuration: BTreeMap::new(),
                url: None,
                source: "builtin".to_string(),
                command: builtin_display_command(&name),
                env: BTreeMap::new(),
                options: BTreeMap::new(),
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
                family: template.family,
                version: template.version,
                configuration: template.configuration,
                url: template.url,
                source: "user".to_string(),
                command,
                env: template.env,
                options: template.options,
            },
        );
    }

    let mut details: Vec<_> = by_name.into_values().collect();
    details.sort_by(|a, b| {
        a.family
            .as_deref()
            .unwrap_or("")
            .cmp(b.family.as_deref().unwrap_or(""))
            .then_with(|| a.name.cmp(&b.name))
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

#[cfg(test)]
mod tests {
    use super::{UserConfig, non_default_options};
    use std::collections::BTreeMap;

    #[test]
    fn strips_default_options_for_identity() {
        let mut effective = BTreeMap::new();
        effective.insert("engine.ply".to_string(), "1".to_string());
        effective.insert("engine.top_k".to_string(), "8".to_string());
        let mut defaults = BTreeMap::new();
        defaults.insert("engine.ply".to_string(), "1".to_string());
        let out = non_default_options(&effective, &defaults);
        assert_eq!(out.get("engine.ply"), None);
        assert_eq!(out.get("engine.top_k"), Some(&"8".to_string()));
    }

    #[test]
    fn parses_family_url_command_and_options() {
        let config: UserConfig = toml::from_str(
            r#"
            [engines.kestral-light]
            family = "kestral"
            version = "v2"
            configuration = { model = "light" }
            url = "https://example.com/kestral"
            command = ["/opt/kestral", "--model", "light.bin"]

            [engines.kestral-light.options]
            "engine.ply" = "1"
            "#,
        )
        .unwrap();
        let engine = &config.engines["kestral-light"];

        assert_eq!(engine.family.as_deref(), Some("kestral"));
        assert_eq!(engine.version.as_deref(), Some("v2"));
        assert_eq!(engine.configuration["model"], "light");
        assert_eq!(engine.url.as_deref(), Some("https://example.com/kestral"));
        assert_eq!(engine.command[0], "/opt/kestral");
        assert_eq!(engine.options["engine.ply"], "1");
    }
}
