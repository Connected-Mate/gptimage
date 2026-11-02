//! Building the providers a project has configured.
//!
//! Keys come from environment variables only. A project file can choose models, quality and endpoints, but it
//! cannot send a key to a host of its own choosing: a `base_url` outside the built-in allowlist is accepted only
//! for loopback hosts or when the user's environment names the host in `HOI4_IMAGEGEN_ALLOWED_HOSTS`.

use std::collections::BTreeMap;
use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource, ProviderConfig};
use imagegen_core::fsio::ProjectFs;

use crate::comfyui::{ComfyProvider, ComfyTemplate};
use crate::fal::FalProvider;
use crate::gemini::GeminiProvider;
use crate::openai::OpenAiProvider;
use crate::transport::{is_loopback_host, parse_url, HostPolicy, HttpTransport};
use crate::types::ImageProvider;

/// Names of the environment variables read for credentials, per provider (first match wins).
pub fn key_env_names(provider: &str, cfg: &ProviderConfig) -> Vec<String> {
    if let Some(custom) = &cfg.api_key_env {
        return vec![custom.clone()];
    }
    match provider {
        "openai" => vec!["OPENAI_API_KEY".into()],
        "gemini" => vec!["GEMINI_API_KEY".into(), "GOOGLE_API_KEY".into()],
        "fal" => vec!["FAL_KEY".into()],
        "comfyui" => vec!["COMFYUI_API_KEY".into()],
        "replicate" => vec!["REPLICATE_API_TOKEN".into()],
        "bfl" => vec!["BFL_API_KEY".into()],
        "stability" => vec!["STABILITY_API_KEY".into()],
        "ideogram" => vec!["IDEOGRAM_API_KEY".into()],
        "recraft" => vec!["RECRAFT_API_KEY".into()],
        "leonardo" => vec!["LEONARDO_API_KEY".into()],
        "xai" => vec!["XAI_API_KEY".into()],
        "together" => vec!["TOGETHER_API_KEY".into()],
        "openrouter" => vec!["OPENROUTER_API_KEY".into()],
        "openai_compatible" => vec!["OPENAI_COMPATIBLE_API_KEY".into()],
        "huggingface" => vec!["HF_TOKEN".into()],
        _ => vec![],
    }
}

pub(crate) fn read_key(provider: &str, cfg: &ProviderConfig, env: &dyn EnvSource) -> Option<String> {
    key_env_names(provider, cfg).iter().find_map(|n| env.get(n))
}

fn extra_hosts(env: &dyn EnvSource) -> Vec<String> {
    env.get("HOI4_IMAGEGEN_ALLOWED_HOSTS")
        .map(|v| v.split(',').map(|h| h.trim().to_ascii_lowercase()).filter(|h| !h.is_empty()).collect())
        .unwrap_or_default()
}

/// Work out the policy for a provider with a possibly customized base URL.
pub(crate) fn policy_for(builtin_hosts: &[&str], base_url: Option<&str>, env: &dyn EnvSource) -> Result<HostPolicy, String> {
    let mut policy = HostPolicy::api(builtin_hosts);
    let Some(url) = base_url else { return Ok(policy) };
    let parsed = parse_url(url)?;
    if builtin_hosts.iter().any(|h| *h == parsed.host) {
        return Ok(policy);
    }
    if is_loopback_host(&parsed.host) {
        policy.allowed_hosts.push(parsed.host);
        policy.allow_loopback_http = true;
        return Ok(policy);
    }
    if extra_hosts(env).contains(&parsed.host) {
        policy.allowed_hosts.push(parsed.host);
        return Ok(policy);
    }
    Err(format!(
        "base_url host '{}' is not on the built-in allowlist. A project file cannot send an API key to a new host: if you trust it, name it in your own environment with HOI4_IMAGEGEN_ALLOWED_HOSTS={}",
        parsed.host, parsed.host
    ))
}

/// The configured providers.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, Arc<dyn ImageProvider>>,
    problems: BTreeMap<String, String>,
}

impl ProviderRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build every provider the config does not disable.
    pub fn build(fs: &ProjectFs, config: &Config, env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) -> Self {
        let mut reg = ProviderRegistry::new();
        let enabled = |id: &str| config.provider(id).enabled != Some(false);

        if enabled("openai") {
            let cfg = config.provider("openai");
            match policy_for(&["api.openai.com"], cfg.base_url.as_deref(), env) {
                Ok(policy) => {
                    let key = read_key("openai", &cfg, env);
                    reg.insert(Arc::new(OpenAiProvider::new(cfg, key, transport.clone(), policy, None)));
                }
                Err(why) => reg.problem("openai", why),
            }
        }
        if enabled("gemini") {
            let cfg = config.provider("gemini");
            match policy_for(&["generativelanguage.googleapis.com"], cfg.base_url.as_deref(), env) {
                Ok(policy) => {
                    let key = read_key("gemini", &cfg, env);
                    reg.insert(Arc::new(GeminiProvider::new(cfg, key, transport.clone(), policy)));
                }
                Err(why) => reg.problem("gemini", why),
            }
        }
        if enabled("fal") {
            let cfg = config.provider("fal");
            match policy_for(&["queue.fal.run", "fal.run", ".fal.media", ".fal.run"], cfg.base_url.as_deref(), env) {
                Ok(policy) => {
                    let key = read_key("fal", &cfg, env);
                    reg.insert(Arc::new(FalProvider::new(cfg, key, transport.clone(), policy)));
                }
                Err(why) => reg.problem("fal", why),
            }
        }
        // Each module registers its own providers (v0.3.0 backends).
        crate::openai_compat::register(&mut reg, config, env, transport.clone());
        crate::replicate::register(&mut reg, config, env, transport.clone());
        crate::bfl::register(&mut reg, config, env, transport.clone());
        crate::leonardo::register(&mut reg, config, env, transport.clone());
        crate::stability::register(&mut reg, config, env, transport.clone());
        crate::ideogram::register(&mut reg, config, env, transport.clone());
        crate::recraft::register(&mut reg, config, env, transport.clone());
        crate::sdwebui::register(&mut reg, config, env, transport.clone());
        if enabled("comfyui") {
            let cfg = config.provider("comfyui");
            if cfg.endpoint.is_some() {
                match load_comfy(fs, &cfg, env, transport.clone()) {
                    Ok(p) => reg.insert(Arc::new(p)),
                    Err(why) => reg.problem("comfyui", why),
                }
            }
        }
        reg
    }

    /// Add or replace a provider.
    pub fn insert(&mut self, provider: Arc<dyn ImageProvider>) {
        self.problems.remove(provider.id());
        self.providers.insert(provider.id().to_string(), provider);
    }

    /// Record why a provider could not be built.
    pub fn problem(&mut self, id: &str, why: String) {
        self.problems.insert(id.to_string(), why);
    }

    /// Provider by id.
    pub fn get(&self, id: &str) -> Option<Arc<dyn ImageProvider>> {
        self.providers.get(id).cloned()
    }

    /// All provider ids.
    pub fn ids(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    /// Providers whose credentials or endpoint are present.
    pub fn configured_ids(&self) -> Vec<String> {
        self.providers.iter().filter(|(_, p)| p.is_configured()).map(|(id, _)| id.clone()).collect()
    }

    /// Why providers are missing.
    pub fn problems(&self) -> &BTreeMap<String, String> {
        &self.problems
    }
}

fn load_comfy(fs: &ProjectFs, cfg: &ProviderConfig, env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) -> Result<ComfyProvider, String> {
    let rel = cfg.workflow.as_deref().ok_or("providers.comfyui.workflow is not set (a project-relative workflow template)")?;
    let bytes = fs.read(rel, 4 * 1024 * 1024).map_err(|e| format!("cannot read the workflow template {rel}: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{rel} is not UTF-8"))?;
    let template = ComfyTemplate::from_json(&text)?;
    ComfyProvider::new(cfg, template, read_key("comfyui", cfg, env), transport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FixtureTransport;
    use imagegen_core::config::MapEnv;

    fn fixture() -> Arc<dyn HttpTransport> {
        Arc::new(FixtureTransport::new())
    }

    fn project() -> (tempfile::TempDir, ProjectFs) {
        let dir = tempfile::tempdir().unwrap();
        let fs = ProjectFs::new(dir.path()).unwrap();
        (dir, fs)
    }

    #[test]
    fn keys_come_from_the_environment_and_status_never_needs_their_values() {
        let (_d, fs) = project();
        let env = MapEnv::from_pairs(&[("OPENAI_API_KEY", "sk-test-1"), ("GOOGLE_API_KEY", "g-test-1")]);
        let reg = ProviderRegistry::build(&fs, &Config::default(), &env, fixture());
        // every built-in backend is offered (doctor names the key it lacks); only those with keys are configured
        assert_eq!(
            reg.ids(),
            vec![
                "automatic1111",
                "bfl",
                "fal",
                "forge",
                "gemini",
                "huggingface",
                "ideogram",
                "leonardo",
                "openai",
                "openrouter",
                "recraft",
                "replicate",
                "stability",
                "together",
                "xai"
            ]
        );
        assert_eq!(reg.configured_ids(), vec!["gemini", "openai"]);
        assert!(reg.problems().is_empty());
    }

    #[test]
    fn providers_can_be_switched_off() {
        let (_d, fs) = project();
        let cfg = Config::parse("[providers.fal]\nenabled = false\n[providers.gemini]\nenabled = false\n").unwrap();
        let reg = ProviderRegistry::build(&fs, &cfg, &MapEnv::default(), fixture());
        let ids = reg.ids();
        for off in ["fal", "gemini"] {
            assert!(!ids.iter().any(|i| i == off), "{off} is switched off: {ids:?}");
        }
        assert!(ids.iter().any(|i| i == "openai") && ids.iter().any(|i| i == "replicate"));
    }

    #[test]
    fn a_project_file_cannot_redirect_a_key_to_a_new_host() {
        let (_d, fs) = project();
        let cfg = Config::parse("[providers.openai]\nbase_url = \"https://evil.example\"\n").unwrap();
        let env = MapEnv::from_pairs(&[("OPENAI_API_KEY", "sk-test-1")]);
        let reg = ProviderRegistry::build(&fs, &cfg, &env, fixture());
        assert!(reg.get("openai").is_none(), "the provider must not exist, so the key is never even read for it");
        assert!(reg.problems()["openai"].contains("HOI4_IMAGEGEN_ALLOWED_HOSTS"));

        // the user's own environment can allow it (Azure OpenAI, a gateway)
        let env = MapEnv::from_pairs(&[("OPENAI_API_KEY", "sk-test-1"), ("HOI4_IMAGEGEN_ALLOWED_HOSTS", "evil.example, other.example")]);
        let reg = ProviderRegistry::build(&fs, &cfg, &env, fixture());
        assert!(reg.get("openai").is_some());
    }

    #[test]
    fn loopback_base_urls_are_allowed_without_an_acknowledgement() {
        let (_d, fs) = project();
        let cfg = Config::parse("[providers.openai]\nbase_url = \"http://127.0.0.1:9\"\n").unwrap();
        let reg = ProviderRegistry::build(&fs, &cfg, &MapEnv::default(), fixture());
        assert!(reg.get("openai").is_some());
        let cfg = Config::parse("[providers.openai]\nbase_url = \"https://api.openai.com/\"\n").unwrap();
        let reg = ProviderRegistry::build(&fs, &cfg, &MapEnv::default(), fixture());
        assert!(reg.get("openai").is_some());
        let cfg = Config::parse("[providers.openai]\nbase_url = \"not a url\"\n").unwrap();
        let reg = ProviderRegistry::build(&fs, &cfg, &MapEnv::default(), fixture());
        assert!(reg.get("openai").is_none() && reg.problems().contains_key("openai"));
    }

    #[test]
    fn comfyui_needs_an_endpoint_and_a_valid_workflow_in_the_project() {
        let (_d, fs) = project();
        let cfg = Config::parse("[providers.comfyui]\nendpoint = \"http://127.0.0.1:8188\"\nworkflow = \"wf/t2i.json\"\n").unwrap();
        let reg = ProviderRegistry::build(&fs, &cfg, &MapEnv::default(), fixture());
        assert!(reg.get("comfyui").is_none());
        assert!(reg.problems()["comfyui"].contains("cannot read the workflow"));

        fs.write_new("wf/t2i.json", br#"{"name":"t2i","slots":{"prompt":{"node":"6","input":"text"}},"output_node":"9","graph":{"6":{"inputs":{"text":""}},"9":{"inputs":{}}}}"#).unwrap();
        let reg = ProviderRegistry::build(&fs, &cfg, &MapEnv::default(), fixture());
        let p = reg.get("comfyui").unwrap();
        assert!(p.is_configured());
        assert_eq!(p.default_model(false), "workflow:t2i");

        // no endpoint: not offered at all, and not reported as a problem either
        let reg = ProviderRegistry::build(&fs, &Config::default(), &MapEnv::default(), fixture());
        assert!(reg.get("comfyui").is_none() && !reg.problems().contains_key("comfyui"));
        // a path that escapes the project is refused
        let cfg = Config::parse("[providers.comfyui]\nendpoint = \"http://127.0.0.1:8188\"\nworkflow = \"../outside.json\"\n").unwrap();
        let reg = ProviderRegistry::build(&fs, &cfg, &MapEnv::default(), fixture());
        assert!(reg.problems()["comfyui"].contains("cannot read"));
    }

    #[test]
    fn custom_key_variable_names_are_honoured() {
        let (_d, fs) = project();
        let cfg = Config::parse("[providers.fal]\napi_key_env = \"MY_FAL\"\n").unwrap();
        let reg = ProviderRegistry::build(&fs, &cfg, &MapEnv::from_pairs(&[("FAL_KEY", "ignored"), ("MY_FAL", "mine")]), fixture());
        assert!(reg.get("fal").unwrap().is_configured());
        assert_eq!(key_env_names("fal", &cfg.provider("fal")), vec!["MY_FAL"]);
        let reg = ProviderRegistry::build(&fs, &cfg, &MapEnv::from_pairs(&[("FAL_KEY", "ignored")]), fixture());
        assert!(!reg.get("fal").unwrap().is_configured(), "only the named variable is read");
    }
}
