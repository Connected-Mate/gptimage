//! The one place that knows about the opt-in ChatGPT-login plugin.
//!
//! In a default build this module only reports that the plugin is not compiled in. The plugin crate is not a
//! dependency unless the `unofficial-chatgpt-login` feature is on, and even then it registers itself only after
//! every gate passes: the feature, a config flag, an environment acknowledgement and local stdio mode.
//! `tests/plugin_isolation.rs` proves a default build contains none of it.

use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource};
use imagegen_providers::{HttpTransport, ProviderRegistry};
use serde::Serialize;

use crate::engine::RunMode;

/// What the gates decided, shown by `hoi4.image_status`.
#[derive(Debug, Clone, Serialize)]
pub struct PluginReport {
    /// True when the plugin crate is part of this build.
    pub compiled: bool,
    /// True when it is registered as a provider.
    pub enabled: bool,
    /// One-line state.
    pub state: String,
    /// Why it is not enabled, or what to do next.
    pub reasons: Vec<String>,
}

#[cfg(not(feature = "unofficial-chatgpt-login"))]
pub fn register(
    _registry: &mut ProviderRegistry,
    config: &Config,
    _env: &dyn EnvSource,
    _mode: RunMode,
    _transport: Arc<dyn HttpTransport>,
) -> PluginReport {
    let asked = config.provider("chatgpt_login").enabled == Some(true);
    PluginReport {
        compiled: false,
        enabled: false,
        state: "not compiled in".into(),
        reasons: if asked {
            vec!["the project config asks for it, but this build does not contain the plugin. It is a separate opt-in source build; see docs/unofficial-chatgpt-login.md".into()]
        } else {
            Vec::new()
        },
    }
}

#[cfg(feature = "unofficial-chatgpt-login")]
pub fn register(
    registry: &mut ProviderRegistry,
    config: &Config,
    env: &dyn EnvSource,
    mode: RunMode,
    transport: Arc<dyn HttpTransport>,
) -> PluginReport {
    use provider_chatgpt_login::{evaluate_gates, ChatGptLoginProvider, PluginMode};

    let plugin_mode = match mode {
        RunMode::LocalStdio => PluginMode::LocalStdio,
        RunMode::Cli => PluginMode::Cli,
        RunMode::Hosted => PluginMode::Hosted,
    };
    let enabled_in_config = config.provider("chatgpt_login").enabled == Some(true);
    match evaluate_gates(enabled_in_config, env.get(provider_chatgpt_login::ACK_ENV).as_deref(), plugin_mode) {
        Ok(()) => {
            registry.insert(Arc::new(ChatGptLoginProvider::from_environment(env, transport)));
            PluginReport { compiled: true, enabled: true, state: "enabled (personal use, unofficial, may break)".into(), reasons: Vec::new() }
        }
        Err(reasons) => PluginReport { compiled: true, enabled: false, state: "compiled in, gates not satisfied".into(), reasons },
    }
}
