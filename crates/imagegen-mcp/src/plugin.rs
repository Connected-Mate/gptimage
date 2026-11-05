//! The one place that wires the ChatGPT-login provider into the registry.
//!
//! Decision D-12a: the provider ships in every local build and registers itself in local stdio and command-line
//! modes. Once the user has signed in (`hoi4-imagegen login`) it is the default local provider. In
//! [`RunMode::Hosted`] it is never registered, and [`refuses_hosted`] is what HOI4 Factory's hosted profile test
//! relies on.

use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource};
use imagegen_providers::{HttpTransport, ProviderRegistry};
use provider_chatgpt_login::{evaluate_gates, ChatGptLoginProvider, PluginMode};
use serde::Serialize;

use crate::engine::RunMode;

/// What the provider's registration found, shown by `hoi4.image_status`.
#[derive(Debug, Clone, Serialize)]
pub struct PluginReport {
    /// Always true: the provider is part of every build. Kept for status readers of version 0.1.
    pub compiled: bool,
    /// True when it is registered as a provider.
    pub enabled: bool,
    /// True when it is registered and the user is signed in.
    pub signed_in: bool,
    /// The fixed model and effort.
    pub model: String,
    /// One-line state.
    pub state: String,
    /// What to do next, or why it is unavailable.
    pub reasons: Vec<String>,
}

pub(crate) fn plugin_mode(mode: RunMode) -> PluginMode {
    match mode {
        RunMode::LocalStdio => PluginMode::LocalStdio,
        RunMode::Cli => PluginMode::Cli,
        RunMode::Hosted => PluginMode::Hosted,
    }
}

/// True when the provider can never be selected in this mode. Hosted always is.
pub fn refuses_hosted(mode: RunMode) -> bool {
    evaluate_gates(plugin_mode(mode)).is_err()
}

/// Register the provider when the mode allows it.
pub fn register(
    registry: &mut ProviderRegistry,
    config: &Config,
    env: &dyn EnvSource,
    mode: RunMode,
    transport: Arc<dyn HttpTransport>,
) -> PluginReport {
    let model = format!("{} at {}", provider_chatgpt_login::MODEL, provider_chatgpt_login::EFFORT_POLICY);
    if let Err(why) = evaluate_gates(plugin_mode(mode)) {
        registry.problem("chatgpt_login", why.clone());
        return PluginReport {
            compiled: true,
            enabled: false,
            signed_in: false,
            model,
            state: "not available in hosted mode".into(),
            reasons: vec![why],
        };
    }
    match ChatGptLoginProvider::from_environment_with(env, transport, config.provider("chatgpt_login").concurrency) {
        Ok(p) => {
            let signed_in = imagegen_providers::ImageProvider::is_configured(&p);
            registry.insert(Arc::new(p));
            PluginReport {
                compiled: true,
                enabled: true,
                signed_in,
                model,
                state: if signed_in {
                    "signed in; the default local provider".into()
                } else {
                    "available; not signed in (hoi4.image_login or `hoi4-imagegen login` opens the browser sign-in)".into()
                },
                reasons: if signed_in { Vec::new() } else { vec!["run `hoi4-imagegen login` once to sign in with your ChatGPT account".into()] },
            }
        }
        Err(why) => {
            registry.problem("chatgpt_login", why.clone());
            PluginReport { compiled: true, enabled: false, signed_in: false, model, state: "credential store unavailable".into(), reasons: vec![why] }
        }
    }
}
