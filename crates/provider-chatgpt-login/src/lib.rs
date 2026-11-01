//! The ChatGPT-login provider: sign in with your own ChatGPT account and generate images on your plan.
//!
//! Derived from Connected-Mate/gptimage (MIT, commit dfb4948): `src/auth.js`, `src/login.js` and `src/codex.js`
//! are ported to Rust here and nowhere else. See `docs/chatgpt-login.md`.
//!
//! Decision D-12a (hoi4-factory, 2026-11-01): this is a first-class local route. It ships in every local build,
//! needs one explicit browser sign-in that shows the risk note once, and becomes the default local provider once
//! the user is signed in. It is never part of hosted HOI4 Factory: [`PluginMode::Hosted`] is always refused.
//!
//! What this is, plainly (the risk note shown at sign-in says the same):
//! * It presents itself as the OpenAI Codex CLI to an endpoint OpenAI does not publish for third parties. It can
//!   change or be blocked without notice. Heavy use can hit plan limits or, in the worst case, restrict the account.
//! * It is for one person's own account on their own machine. Never share tokens, never run it on a server.
//!
//! The maintainer fixed the model: every image request runs on [`MODEL`] at [`REASONING_EFFORT`] reasoning effort.
//! There is no override and no fallback. A response that does not echo both is refused.
//!
//! It keeps its own credentials only (it never reads or writes `~/.codex/auth.json`), has no token environment
//! variable, reports usage as unknown and never charges any operator allowance.

#![forbid(unsafe_code)]
// ProviderError is the shared error type of every provider; boxing it here alone would buy nothing.
#![allow(clippy::result_large_err)]

pub mod cli;
pub mod consent;
pub mod oauth;
pub mod provider;
pub mod store;

pub use provider::ChatGptLoginProvider;

// ---------------------------------------------------------------------------------------------------------
// Everything that identifies this client to the backend lives here, in one place, on purpose.
//
// CLIENT_ID and ORIGINATOR are the public identifiers of the official Codex CLI. Presenting them makes this tool
// look like that CLI to OpenAI's servers. If OpenAI changes them or objects to their use, this route stops working.
// ---------------------------------------------------------------------------------------------------------

/// OAuth client id of the official Codex CLI (public).
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Client identifier sent to the backend; matches the official Codex CLI.
pub const ORIGINATOR: &str = "codex_cli_rs";
/// Authorization endpoint.
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
/// Token endpoint.
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// The responses endpoint of the Codex backend.
pub const BACKEND_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
/// The only model this route uses (maintainer decision, 2026-11-01). No override, no fallback.
pub const MODEL: &str = "gpt-6-luna";
/// The only reasoning effort this route uses (maintainer decision, 2026-11-01).
pub const REASONING_EFFORT: &str = "high";
/// Scopes requested at sign-in.
pub const SCOPE: &str = "openid profile email offline_access";
/// Loopback port the sign-in redirect uses.
pub const CALLBACK_PORT: u16 = 1455;
/// Path of the sign-in redirect.
pub const CALLBACK_PATH: &str = "/auth/callback";
/// Hosts the provider may contact.
pub const HOSTS: [&str; 2] = ["chatgpt.com", "auth.openai.com"];

/// How the host process runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginMode {
    /// A local MCP server on stdio.
    LocalStdio,
    /// The command-line tool on the user's machine.
    Cli,
    /// Hosted HOI4 Factory or any networked service. Always refused.
    Hosted,
}

/// The one gate left after D-12a: the provider never runs hosted.
pub fn evaluate_gates(mode: PluginMode) -> Result<(), String> {
    if mode == PluginMode::Hosted {
        return Err(
            "the ChatGPT-login provider is never available in a hosted or networked deployment (decision D-12a); use a documented API provider"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_modes_pass_and_hosted_is_always_refused() {
        assert!(evaluate_gates(PluginMode::LocalStdio).is_ok());
        assert!(evaluate_gates(PluginMode::Cli).is_ok());
        assert!(evaluate_gates(PluginMode::Hosted).unwrap_err().contains("hosted"));
    }

    #[test]
    fn the_identifying_constants_are_in_one_place() {
        assert!(BACKEND_URL.starts_with("https://") && TOKEN_URL.starts_with("https://") && AUTHORIZE_URL.starts_with("https://"));
        assert_eq!(HOSTS, ["chatgpt.com", "auth.openai.com"]);
        assert_eq!((MODEL, REASONING_EFFORT), ("gpt-6-luna", "high"));
    }
}
