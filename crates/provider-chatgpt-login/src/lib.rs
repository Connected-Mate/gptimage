//! OPT-IN, UNOFFICIAL provider: sign in with a ChatGPT account and generate images through an undocumented backend.
//!
//! Derived from Connected-Mate/gptimage (MIT, commit dfb4948): `src/auth.js`, `src/login.js` and `src/codex.js`
//! are ported to Rust here and nowhere else. Read `docs/unofficial-chatgpt-login.md` before enabling it.
//!
//! What this is, plainly:
//! * It impersonates the OpenAI Codex CLI against an endpoint OpenAI does not publish or support. It can change
//!   or be blocked without notice, and the model slug it uses is not a stable image model.
//! * Upstream's own README warns that heavy use can hit plan rate limits or "worst case, account restrictions".
//!   OpenAI's guidance is to use Platform API keys for general API calls. Whether this use fits OpenAI's Terms of
//!   Use was not verified (the primary text could not be fetched).
//! * It is for one person's own account on their own machine. Never share tokens, never run it on a server.
//!
//! What it does differently from upstream: it keeps its own credential file only (it never reads or writes
//! `~/.codex/auth.json`, so it cannot rotate another tool's refresh token), has no token environment variable,
//! reports usage as unknown and never charges any operator allowance.
//!
//! Isolation: this crate is a workspace member but not a default member. `imagegen-core` and
//! `imagegen-providers` know nothing about it. `imagegen-mcp` and `imagegen-cli` depend on it only through the
//! `unofficial-chatgpt-login` feature, and register it only after [`evaluate_gates`] passes.

#![forbid(unsafe_code)]

pub mod cli;
pub mod oauth;
pub mod provider;
pub mod store;

pub use provider::ChatGptLoginProvider;

// ---------------------------------------------------------------------------------------------------------
// Everything that identifies this client to the backend lives here, in one place, on purpose.
//
// WARNING: CLIENT_ID and ORIGINATOR are the public identifiers of the official Codex CLI. Presenting them makes
// this tool look like that CLI to OpenAI's servers. That is the unofficial part. If OpenAI changes them or
// objects to their use, this plugin stops working, and the account consequences are the user's alone.
// ---------------------------------------------------------------------------------------------------------

/// OAuth client id of the official Codex CLI (public).
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Client identifier sent to the backend; matches the official Codex CLI.
pub const ORIGINATOR: &str = "codex_cli_rs";
/// Authorization endpoint.
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
/// Token endpoint.
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// The undocumented responses endpoint of the Codex backend.
pub const BACKEND_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
/// Model slug that carries the hosted image tool. Not a stable image model; override in config when it rotates.
pub const DEFAULT_MODEL: &str = "gpt-5.5";
/// Scopes requested at sign-in.
pub const SCOPE: &str = "openid profile email offline_access";
/// Loopback port the sign-in redirect uses.
pub const CALLBACK_PORT: u16 = 1455;
/// Path of the sign-in redirect.
pub const CALLBACK_PATH: &str = "/auth/callback";
/// Environment variable that must be exactly `1` for the plugin to start. A project file alone cannot set it.
pub const ACK_ENV: &str = "HOI4_IMAGEGEN_ACCEPT_UNOFFICIAL";
/// Hosts the plugin may contact.
pub const HOSTS: [&str; 2] = ["chatgpt.com", "auth.openai.com"];

/// How the host process runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginMode {
    /// A local MCP server on stdio.
    LocalStdio,
    /// The command-line tool on the user's machine.
    Cli,
    /// Anything networked or hosted. Always refused.
    Hosted,
}

/// Check every gate. All must hold: the cargo feature (implied by this crate being compiled in), the config flag,
/// the environment acknowledgement, and a local mode. Returns every reason it is closed, not just the first.
pub fn evaluate_gates(config_enabled: bool, acknowledgement: Option<&str>, mode: PluginMode) -> Result<(), Vec<String>> {
    let mut reasons = Vec::new();
    if !config_enabled {
        reasons.push("set [providers.chatgpt_login] enabled = true in .hoi4-imagegen/config.toml".to_string());
    }
    if acknowledgement != Some("1") {
        reasons.push(format!("set the environment variable {ACK_ENV}=1 in your own environment (a project file cannot do this for you)"));
    }
    if mode == PluginMode::Hosted {
        reasons.push("this plugin never runs in a hosted or networked mode; use a local stdio server or the command line".to_string());
    }
    if reasons.is_empty() {
        Ok(())
    } else {
        Err(reasons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_gates_must_hold() {
        assert!(evaluate_gates(true, Some("1"), PluginMode::LocalStdio).is_ok());
        assert!(evaluate_gates(true, Some("1"), PluginMode::Cli).is_ok());
    }

    #[test]
    fn each_gate_closes_it_on_its_own() {
        let e = evaluate_gates(false, Some("1"), PluginMode::LocalStdio).unwrap_err();
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("enabled = true"));
        let e = evaluate_gates(true, None, PluginMode::LocalStdio).unwrap_err();
        assert!(e[0].contains(ACK_ENV));
        assert!(evaluate_gates(true, Some("yes"), PluginMode::LocalStdio).is_err(), "only the exact value 1 counts");
        assert!(evaluate_gates(true, Some("true"), PluginMode::LocalStdio).is_err());
        let e = evaluate_gates(true, Some("1"), PluginMode::Hosted).unwrap_err();
        assert!(e[0].contains("hosted"));
    }

    #[test]
    fn every_closed_gate_is_reported() {
        let e = evaluate_gates(false, None, PluginMode::Hosted).unwrap_err();
        assert_eq!(e.len(), 3);
    }

    #[test]
    fn the_identifying_constants_are_in_one_place() {
        assert!(BACKEND_URL.starts_with("https://") && TOKEN_URL.starts_with("https://") && AUTHORIZE_URL.starts_with("https://"));
        assert_eq!(HOSTS, ["chatgpt.com", "auth.openai.com"]);
    }
}
