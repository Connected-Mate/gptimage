//! Project configuration (`.hoi4-imagegen/config.toml`) and the environment.
//!
//! The file holds choices, never secrets. Provider keys are read from environment variables only.
//! Safety switches that widen what leaves the machine (the ChatGPT-login plugin, custom API hosts) cannot
//! be turned on by this file alone: they also need an environment acknowledgement, so a cloned project
//! cannot enable them for its user.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::family::Family;
use crate::profile::RowOverride;

/// Relative path of the state folder inside a project.
pub const STATE_DIR: &str = ".hoi4-imagegen";
/// Relative path of the config file.
pub const CONFIG_PATH: &str = ".hoi4-imagegen/config.toml";
/// Default location of the kit's profile file inside a mod project.
pub const KIT_PROFILES_PATH: &str = ".tools/gfx/profiles.json";

/// Read-only view of environment variables.
pub trait EnvSource: Send + Sync {
    /// Value of a variable, if set and non-empty.
    fn get(&self, key: &str) -> Option<String>;
}

/// The process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }
}

/// A fixed map, for tests.
#[derive(Debug, Default, Clone)]
pub struct MapEnv(pub BTreeMap<String, String>);

impl MapEnv {
    /// Build from pairs.
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        MapEnv(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }
}

impl EnvSource for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned().filter(|v| !v.is_empty())
    }
}

/// Project policy switches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyConfig {
    /// Decision D4: sending installed-game or kit-shelf (Paradox) art to a provider as a reference.
    /// Off by default; opt-in per project.
    pub allow_vanilla_references: bool,
    /// Categories this project never generates, whatever the provider (a project overlay, for
    /// example Chaos Redux never generates historical leaders).
    #[serde(alias = "forbidden_categories")]
    pub forbidden_families: Vec<Family>,
    /// A single provider call whose estimate exceeds this needs `approve_cost`.
    pub max_usd_per_call: f64,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        PolicyConfig { allow_vanilla_references: false, forbidden_families: Vec::new(), max_usd_per_call: 1.0 }
    }
}

/// One provider's settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfig {
    /// Set false to turn a provider off.
    pub enabled: Option<bool>,
    /// Model id, overriding the provider default.
    pub model: Option<String>,
    /// Quality tier (`low`, `medium`, `high` where the model has tiers).
    pub quality: Option<String>,
    /// API base URL. Hosts outside the built-in allowlist also need `HOI4_IMAGEGEN_ALLOWED_HOSTS`.
    pub base_url: Option<String>,
    /// fal endpoint id, or ComfyUI server URL.
    pub endpoint: Option<String>,
    /// ComfyUI workflow template (project-relative path).
    pub workflow: Option<String>,
    /// Environment variable holding the key, when it is not the provider's usual one.
    pub api_key_env: Option<String>,
}

/// Defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DefaultsConfig {
    /// Provider used when a plan does not name one.
    pub provider: Option<String>,
}

/// The parsed config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Installed game folder (the `HOI4_GAME_DIR` environment variable wins).
    pub game_dir: Option<String>,
    /// Kit profile file, relative to the project. Defaults to `.tools/gfx/profiles.json`.
    pub kit_profiles: Option<String>,
    /// Policy.
    pub policy: PolicyConfig,
    /// Defaults.
    pub defaults: DefaultsConfig,
    /// Provider settings by provider id.
    pub providers: BTreeMap<String, ProviderConfig>,
    /// Per-project profile overrides by profile id.
    #[serde(skip_serializing)]
    pub profiles: BTreeMap<String, RowOverride>,
}

impl Config {
    /// Load the project's config, or the defaults when there is none.
    pub fn load(project_root: &Path) -> Result<Config> {
        let path = project_root.join(CONFIG_PATH);
        if !path.is_file() {
            return Ok(Config::default());
        }
        let bytes = crate::fsio::read_limited(&path, 256 * 1024)?;
        let text = String::from_utf8(bytes).map_err(|_| Error::Config(format!("{} is not UTF-8", path.display())))?;
        Config::parse(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
    }

    /// Parse TOML text.
    pub fn parse(text: &str) -> std::result::Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    /// Provider settings, or the empty default.
    pub fn provider(&self, id: &str) -> ProviderConfig {
        self.providers.get(id).cloned().unwrap_or_default()
    }

    /// Path of the kit profile file for a project.
    pub fn kit_profiles_file(&self, project_root: &Path) -> PathBuf {
        project_root.join(self.kit_profiles.as_deref().unwrap_or(KIT_PROFILES_PATH))
    }
}

/// Locate the installed game folder. An explicit setting (`HOI4_GAME_DIR`, then `game_dir` in the config) is the
/// only candidate when present: if it points nowhere the answer is "not found", never a silent fallback to a
/// different install. With no setting, the usual Steam locations are tried.
pub fn find_game_dir(config: &Config, env: &dyn EnvSource) -> Option<PathBuf> {
    let valid = |p: &PathBuf| p.join("interface").is_dir() || p.join("gfx").is_dir();
    if let Some(v) = env.get("HOI4_GAME_DIR") {
        return Some(PathBuf::from(v)).filter(valid);
    }
    if let Some(v) = &config.game_dir {
        return Some(PathBuf::from(v)).filter(valid);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for base in ["C:/Program Files (x86)/Steam", "C:/Program Files/Steam"] {
            candidates.push(PathBuf::from(base).join("steamapps/common/Hearts of Iron IV"));
        }
    }
    if let Some(home) = env.get("HOME").or_else(|| env.get("USERPROFILE")) {
        let home = PathBuf::from(home);
        candidates.push(home.join("Library/Application Support/Steam/steamapps/common/Hearts of Iron IV"));
        candidates.push(home.join(".steam/steam/steamapps/common/Hearts of Iron IV"));
        candidates.push(home.join(".local/share/Steam/steamapps/common/Hearts of Iron IV"));
    }
    candidates.into_iter().find(valid)
}

/// Read `rawVersion` from `launcher-settings.json`.
pub fn game_version(game_dir: &Path) -> Option<String> {
    let bytes = std::fs::read(game_dir.join("launcher-settings.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("rawVersion").and_then(|s| s.as_str()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        let c = Config::default();
        assert!(!c.policy.allow_vanilla_references, "D4: vanilla references are off by default");
        assert!(c.policy.forbidden_families.is_empty());
        assert_eq!(c.policy.max_usd_per_call, 1.0);
    }

    #[test]
    fn parses_a_full_file() {
        let text = r#"
game_dir = "C:/Games/HOI4"
kit_profiles = ".tools/gfx/profiles.json"

[policy]
allow_vanilla_references = true
forbidden_families = ["leader_portrait"]
max_usd_per_call = 0.25

[defaults]
provider = "comfyui"

[providers.openai]
model = "gpt-image-2.5-flare"
quality = "low"

[providers.comfyui]
endpoint = "http://127.0.0.1:8188"
workflow = ".hoi4-imagegen/comfyui/text_to_image.api.json"

[profiles.focus]
canvas = [94, 86]
margin = 2
"#;
        let c = Config::parse(text).unwrap();
        assert!(c.policy.allow_vanilla_references);
        assert_eq!(c.policy.forbidden_families, vec![Family::LeaderPortrait]);
        assert_eq!(c.provider("openai").model.as_deref(), Some("gpt-image-2.5-flare"));
        assert_eq!(c.provider("comfyui").endpoint.as_deref(), Some("http://127.0.0.1:8188"));
        assert_eq!(c.profiles["focus"].canvas, Some([94, 86]));
        assert_eq!(c.defaults.provider.as_deref(), Some("comfyui"));
    }

    #[test]
    fn typos_in_safety_keys_are_errors_not_silent() {
        assert!(Config::parse("[policy]\nallow_vanilla_reference = true\n").is_err());
        assert!(Config::parse("[policy]\nforbidden_families = [\"not_a_family\"]\n").is_err());
        assert!(Config::parse("surprise = 1\n").is_err());
    }

    #[test]
    fn load_reads_the_file_or_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!Config::load(dir.path()).unwrap().policy.allow_vanilla_references);
        std::fs::create_dir_all(dir.path().join(STATE_DIR)).unwrap();
        std::fs::write(dir.path().join(CONFIG_PATH), "[policy]\nallow_vanilla_references = true\n").unwrap();
        assert!(Config::load(dir.path()).unwrap().policy.allow_vanilla_references);
        std::fs::write(dir.path().join(CONFIG_PATH), "[policy\n").unwrap();
        assert!(Config::load(dir.path()).is_err());
    }

    #[test]
    fn game_dir_prefers_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("interface")).unwrap();
        std::fs::write(dir.path().join("launcher-settings.json"), r#"{"rawVersion":"1.19.3.0"}"#).unwrap();
        let env = MapEnv::from_pairs(&[("HOI4_GAME_DIR", &dir.path().display().to_string())]);
        let found = find_game_dir(&Config::default(), &env).unwrap();
        assert_eq!(found, dir.path());
        assert_eq!(game_version(&found).as_deref(), Some("1.19.3.0"));
        // an explicit setting that points nowhere is "not found", not a fallback to another install
        let missing = MapEnv::from_pairs(&[("HOI4_GAME_DIR", "/definitely/not/here")]);
        assert_eq!(find_game_dir(&Config::default(), &missing), None);
        let cfg = Config::parse(
            "game_dir = \"/also/not/here\"
",
        )
        .unwrap();
        assert_eq!(find_game_dir(&cfg, &MapEnv::default()), None);
    }
}
