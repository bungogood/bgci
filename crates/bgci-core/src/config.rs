use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::engines;
use bkgm::{EngineSpec, Variant, format_engine_spec, parse_engine_spec};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MatchupConfig {
    pub games: usize,
    pub parallel: usize,
    pub seed: u64,
    pub max_plies: usize,
    pub variant: String,
    pub log_level: String,
    pub engine_a: EngineInput,
    pub engine_b: EngineInput,
}

impl Default for MatchupConfig {
    fn default() -> Self {
        Self {
            games: 20,
            parallel: 1,
            seed: 42,
            max_plies: 512,
            variant: "backgammon".to_string(),
            log_level: "off".to_string(),
            engine_a: EngineInput::default_profile("random"),
            engine_b: EngineInput::default_profile("random"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(deserialize_with = "deserialize_command")]
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub ubgi: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineLaunch {
    command: Vec<String>,
    env: BTreeMap<String, String>,
    ubgi: BTreeMap<String, String>,
}

impl EngineLaunch {
    pub fn new(
        command: Vec<String>,
        env: BTreeMap<String, String>,
        ubgi: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        if command.is_empty() || command[0].trim().is_empty() {
            return Err("engine launch command cannot be empty".to_string());
        }
        Ok(Self { command, env, ubgi })
    }

    pub fn command(&self) -> &[String] {
        &self.command
    }

    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    pub fn ubgi(&self) -> &BTreeMap<String, String> {
        &self.ubgi
    }

    pub fn ubgi_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.ubgi
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineMetadata {
    pub family: Option<String>,
    pub version: Option<String>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEngine {
    pub name: String,
    pub launch: EngineLaunch,
    pub metadata: EngineMetadata,
}

#[derive(Debug, Clone)]
pub struct ResolvedMatchup {
    pub games: usize,
    pub parallel: usize,
    pub seed: u64,
    pub max_plies: usize,
    pub variant: Variant,
    pub engine_a: ResolvedEngine,
    pub engine_b: ResolvedEngine,
}

#[derive(Debug, Clone)]
pub struct ProfileDetail {
    pub name: String,
    pub family: Option<String>,
    pub version: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub url: Option<String>,
    pub source: String,
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub ubgi: BTreeMap<String, String>,
}

impl EngineInput {
    fn default_profile(profile: &str) -> Self {
        Self {
            name: None,
            family: None,
            version: None,
            labels: BTreeMap::new(),
            profile: Some(profile.to_string()),
            command: Vec::new(),
            env: BTreeMap::new(),
            ubgi: BTreeMap::new(),
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
#[serde(deny_unknown_fields)]
struct ProfileTemplate {
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(deserialize_with = "deserialize_command")]
    command: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    ubgi: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct ConfiguredProfile {
    name: String,
    template: ProfileTemplate,
}

type Profiles = BTreeMap<String, ConfiguredProfile>;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UserConfig {
    profiles: BTreeMap<String, ProfileTemplate>,
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

pub fn resolve_engine_input(engine: EngineInput) -> Result<ResolvedEngine, String> {
    let profiles = load_user_profiles()?;
    resolve_engine_input_from_profiles(engine, &profiles)
}

pub fn resolve_profile_reference(alias: &str) -> Result<ResolvedEngine, String> {
    let profiles = load_user_profiles()?;
    resolve_profile_reference_from_profiles(alias, &profiles)
}

fn resolve_profile_reference_from_profiles(
    alias: &str,
    profiles: &Profiles,
) -> Result<ResolvedEngine, String> {
    resolve_engine_input_from_profiles(EngineInput::default_profile(alias), profiles)
}

pub fn resolve_engine_spec(spec: &str) -> Result<ResolvedEngine, String> {
    let profiles = load_user_profiles()?;
    resolve_engine_spec_from_profiles(spec, &profiles)
}

pub fn resolve_engine_specs(specs: &[String]) -> Result<Vec<ResolvedEngine>, String> {
    let profiles = load_user_profiles()?;
    specs
        .iter()
        .map(|spec| resolve_engine_spec_from_profiles(spec, &profiles))
        .collect()
}

fn resolve_engine_spec_from_profiles(
    spec: &str,
    profiles: &Profiles,
) -> Result<ResolvedEngine, String> {
    let parsed = parse_engine_spec(spec).map_err(|err| err.to_string())?;
    if parsed.version.is_some() {
        return Err(format!(
            "profile versions are not selectors; use an exact profile alias instead of '{}@{}'",
            parsed.alias,
            parsed.version.as_deref().unwrap_or_default()
        ));
    }

    let mut config = resolve_profile_reference_from_profiles(&parsed.alias, profiles)?;
    config.launch.ubgi_mut().extend(parsed.options);
    canonicalize_resolved_engine_name(&mut config);
    Ok(config)
}

pub fn canonicalize_resolved_engine_name(config: &mut ResolvedEngine) {
    let alias = config
        .name
        .split_once(':')
        .map_or(config.name.as_str(), |(alias, _)| alias)
        .to_string();
    config.name = format_engine_spec(&EngineSpec {
        alias,
        version: None,
        options: config.launch.ubgi().clone(),
    });
}

fn resolve_engine_input_from_profiles(
    mut engine: EngineInput,
    profiles: &Profiles,
) -> Result<ResolvedEngine, String> {
    let has_profile = engine.profile.is_some();
    let has_command = !engine.command.is_empty();
    let input_name = engine.name.as_deref().unwrap_or("<unnamed>");

    if has_profile && has_command {
        return Err(format!(
            "engine '{input_name}' has both 'profile' and 'command'; choose one"
        ));
    }
    if !has_profile && !has_command {
        return Err(format!(
            "engine '{input_name}' must set either 'profile' or 'command'"
        ));
    }
    if has_command {
        let name = engine
            .name
            .take()
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| "a direct engine command requires a non-empty 'name'".to_string())?;
        engine.name = Some(name);
        expand_tilde_in_command(&mut engine.command);
        return into_resolved_engine(engine);
    }

    let alias = engine
        .profile
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if alias.is_empty() {
        return Err("engine has empty 'profile' reference".to_string());
    }

    if let Some(kind) = engines::builtin_engine_name(&alias) {
        engine.name = Some(kind.to_string());
        set_builtin_engine_command(&mut engine, kind)?;
        expand_tilde_in_command(&mut engine.command);
        let mut resolved = into_resolved_engine(engine)?;
        canonicalize_resolved_engine_name(&mut resolved);
        return Ok(resolved);
    }

    if let Some(profile) = profiles.get(&alias) {
        let template = &profile.template;
        engine.name = Some(profile.name.clone());
        engine.command = template.command.clone();
        engine.profile = None;
        if engine.family.is_none() {
            engine.family = template.family.clone();
        }
        if engine.version.is_none() {
            engine.version = template.version.clone();
        }
        if engine.labels.is_empty() {
            engine.labels = template.labels.clone();
        }
        let mut merged_env = template.env.clone();
        merged_env.extend(engine.env);
        engine.env = merged_env;
        let mut merged_ubgi = template.ubgi.clone();
        merged_ubgi.extend(engine.ubgi);
        engine.ubgi = merged_ubgi;

        if engine.command.len() == 1 {
            let nested_alias = engine.command[0].trim().to_ascii_lowercase();
            if let Some(kind) = engines::builtin_engine_name(&nested_alias) {
                set_builtin_engine_command(&mut engine, kind)?;
            }
        }
        expand_tilde_in_command(&mut engine.command);
        let mut resolved = into_resolved_engine(engine)?;
        canonicalize_resolved_engine_name(&mut resolved);
        return Ok(resolved);
    }

    Err(format!("unknown profile alias '{alias}'"))
}

fn into_resolved_engine(engine: EngineInput) -> Result<ResolvedEngine, String> {
    Ok(ResolvedEngine {
        name: engine
            .name
            .ok_or_else(|| "resolved engine has no stable name".to_string())?,
        launch: EngineLaunch::new(engine.command, engine.env, engine.ubgi)?,
        metadata: EngineMetadata {
            family: engine.family,
            version: engine.version,
            labels: engine.labels,
        },
    })
}

fn builtin_profile_names() -> Vec<String> {
    engines::BUILTIN_ENGINE_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

fn set_builtin_engine_command(engine: &mut EngineInput, kind: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("resolve current executable: {e}"))?;
    engine.command = vec![
        exe.to_string_lossy().into_owned(),
        "engine".to_string(),
        kind.to_string(),
    ];
    engine.profile = None;
    Ok(())
}

fn load_user_profiles() -> Result<Profiles, String> {
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
    normalize_profiles(parsed)
}

fn normalize_profiles(config: UserConfig) -> Result<Profiles, String> {
    let mut profiles = BTreeMap::new();
    for (name, template) in config.profiles {
        let normalized = name.to_ascii_lowercase();
        if engines::builtin_engine_name(&normalized).is_some() {
            return Err(format!(
                "configured profile '{name}' conflicts with a built-in profile"
            ));
        }
        let parsed = parse_engine_spec(&name)
            .map_err(|error| format!("invalid profile alias '{name}': {error}"))?;
        if parsed.alias != name || parsed.version.is_some() || !parsed.options.is_empty() {
            return Err(format!(
                "invalid profile alias '{name}': aliases cannot contain version or UBGI syntax"
            ));
        }
        if profiles
            .insert(normalized.clone(), ConfiguredProfile { name, template })
            .is_some()
        {
            return Err(format!(
                "duplicate profile aliases differing only by case: '{normalized}'"
            ));
        }
    }
    Ok(profiles)
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

pub fn list_profile_details() -> Result<Vec<ProfileDetail>, String> {
    let mut by_name = BTreeMap::new();
    for name in builtin_profile_names() {
        by_name.insert(
            name.clone(),
            ProfileDetail {
                name: name.clone(),
                family: None,
                version: None,
                labels: BTreeMap::new(),
                url: None,
                source: "builtin".to_string(),
                command: builtin_display_command(&name),
                env: BTreeMap::new(),
                ubgi: BTreeMap::new(),
            },
        );
    }

    for profile in load_user_profiles()?.into_values() {
        let template = profile.template;
        let mut command = template.command;
        expand_tilde_in_command(&mut command);
        if command.len() == 1 {
            let nested_alias = command[0].trim().to_ascii_lowercase();
            if let Some(kind) = engines::builtin_engine_name(&nested_alias) {
                command = builtin_display_command(kind);
            }
        }
        by_name.insert(
            profile.name.clone(),
            ProfileDetail {
                name: profile.name,
                family: template.family,
                version: template.version,
                labels: template.labels,
                url: template.url,
                source: "user".to_string(),
                command,
                env: template.env,
                ubgi: template.ubgi,
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
    use super::{
        MatchupConfig, UserConfig, canonicalize_resolved_engine_name, normalize_profiles,
        resolve_engine_input_from_profiles, resolve_engine_spec_from_profiles,
    };

    fn profiles(input: &str) -> super::Profiles {
        normalize_profiles(toml::from_str::<UserConfig>(input).unwrap()).unwrap()
    }

    #[test]
    fn resolves_direct_commands_and_exact_profile_aliases() {
        let matchup: MatchupConfig = toml::from_str(
            r#"
            [engine_a]
            name = "scalar"
            command = "engine-a"
            labels = { model = "small" }
            [engine_a.ubgi]
            "engine.ply" = "1"

            [engine_b]
            name = "array"
            command = ["engine-b", "--flag"]
            "#,
        )
        .unwrap();
        let resolved_a =
            resolve_engine_input_from_profiles(matchup.engine_a, &Default::default()).unwrap();
        let resolved_b =
            resolve_engine_input_from_profiles(matchup.engine_b, &Default::default()).unwrap();
        assert_eq!(resolved_a.name, "scalar");
        assert_eq!(resolved_a.metadata.labels["model"], "small");
        assert_eq!(resolved_a.launch.ubgi()["engine.ply"], "1");
        assert_eq!(resolved_b.launch.command(), ["engine-b", "--flag"]);

        let profiles = profiles(
            r#"
            [profiles.Hedgehog-Star2]
            family = "hedgehog"
            version = "fox-v0.32"
            labels = { model = "fox" }
            command = ["hedgehog", "--model", "fox.ogxf"]
            [profiles.Hedgehog-Star2.ubgi]
            "engine.ply" = "2"
            "engine.search" = "star2"
            "#,
        );
        let config =
            resolve_engine_spec_from_profiles("hedgehog-star2:ply=3,search=expectimax", &profiles)
                .unwrap();
        assert_eq!(config.name, "Hedgehog-Star2:ply=3,search=expectimax");
        assert_eq!(config.metadata.family.as_deref(), Some("hedgehog"));
        assert_eq!(config.metadata.version.as_deref(), Some("fox-v0.32"));
        assert_eq!(config.metadata.labels["model"], "fox");
        assert_eq!(config.launch.ubgi()["engine.ply"], "3");
    }

    #[test]
    fn rejects_old_vocabulary_and_non_alias_selectors() {
        for input in [
            "[engines.hawk]\ncommand = 'hawk'",
            "[profiles.hawk]\ncommand = 'hawk'\nconfiguration = { model = 'x' }",
            "[profiles.hawk]\ncommand = 'hawk'\noptions = {}",
        ] {
            assert!(toml::from_str::<UserConfig>(input).is_err(), "{input}");
        }
        assert!(
            toml::from_str::<MatchupConfig>(
                "[engine_a]\nname='a'\nengine='random'\n[engine_b]\nprofile='random'"
            )
            .is_err()
        );
        assert!(toml::from_str::<MatchupConfig>("pairs=10").is_err());

        let profiles = profiles(
            "[profiles.hawk-v1]\nfamily='hawk'\nversion='v1'\nlabels={model='small'}\ncommand='hawk'",
        );
        assert!(resolve_engine_spec_from_profiles("hawk@v1", &profiles).is_err());
        assert!(resolve_engine_spec_from_profiles("hawk:model=small", &profiles).is_err());
    }

    #[test]
    fn rejects_case_insensitive_duplicate_profile_aliases() {
        let config: UserConfig = toml::from_str(
            r#"
            [profiles.Hawk]
            command = "hawk-a"
            [profiles.hawk]
            command = "hawk-b"
            "#,
        )
        .unwrap();
        let error = normalize_profiles(config).unwrap_err();
        assert!(error.contains("duplicate profile aliases differing only by case"));
    }

    #[test]
    fn rejects_builtin_collisions_and_non_roundtrippable_aliases() {
        for input in [
            "[profiles.random]\ncommand = 'custom-random'",
            "[profiles.'hawk@v1']\ncommand = 'hawk'",
            "[profiles.'hawk:ply=2']\ncommand = 'hawk'",
        ] {
            let config: UserConfig = toml::from_str(input).unwrap();
            assert!(normalize_profiles(config).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn canonicalizes_only_alias_and_effective_ubgi() {
        let profiles = profiles(
            r#"
            [profiles.hawk-fast]
            family = "hawk"
            version = "v1"
            labels = { model = "small" }
            command = "hawk"
            [profiles.hawk-fast.ubgi]
            "engine.ply" = "1"
            "engine.search" = "star2"
            "game.variant" = "backgammon"
            "#,
        );
        let mut engine = resolve_engine_spec_from_profiles("HAWK-FAST:ply=2", &profiles).unwrap();
        engine.launch.ubgi_mut().remove("engine.search");
        engine.launch.ubgi_mut().remove("game.variant");
        engine
            .launch
            .ubgi_mut()
            .insert("engine.ply".to_string(), "3".to_string());
        canonicalize_resolved_engine_name(&mut engine);
        assert_eq!(engine.name, "hawk-fast:ply=3");
    }
}
