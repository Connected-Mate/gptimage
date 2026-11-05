//! The risk note, shown once at sign-in, and the record that the user accepted it.
//!
//! The record lives in the user's own data folder, never in a project, so a cloned mod cannot accept it for
//! anyone. Changing [`RISK_NOTE_VERSION`] shows the note again at the next sign-in.

use std::fs;
use std::path::PathBuf;

use imagegen_core::config::EnvSource;
use serde::{Deserialize, Serialize};

use crate::store::data_dir;

/// Bump when the note's substance changes.
pub const RISK_NOTE_VERSION: u32 = 3;

/// The note, shown before the browser opens.
pub const RISK_NOTE: &str = "\
ChatGPT login: read this once before signing in.

* This route uses your own ChatGPT plan to make images. It presents itself as the OpenAI Codex CLI to a backend
  that OpenAI does not document for other tools. It can stop working or be blocked without notice.
* Heavy use counts against your plan's limits and, in the worst case, OpenAI could restrict the account.
  Whether this use fits OpenAI's terms has not been confirmed.
* Running many generations at once, or from several tools that all present as Codex, can make OpenAI revoke
  every Codex sign-in on your account, which also signs you out of Codex itself. This happened on 2026-11-02.
  hoi4-imagegen now sends one request at a time on your machine (across every process), spaces generations at
  least 20 seconds apart and backs off when OpenAI says to slow down. If the server does revoke the sign-in,
  hoi4-imagegen records it, stops calling, and tells you to run `hoi4-imagegen login` again.
* It is for your own account on your own computer only. Never share the sign-in or run it on a server.
  HOI4 Factory's hosted service never uses it.
* Images use the gpt-6-luna model at the highest reasoning effort the backend accepts (high, then medium,
  then low). If it accepts none of them, the request goes without an effort and the image may be made at the
  backend's own default effort. Each image records the effort it ran at. Your password is typed only into OpenAI's page;
  this tool keeps the resulting tokens in your operating system's credential store.
* Documented API providers (OpenAI API key, Gemini, fal, ComfyUI) remain available as alternatives.
";

/// What was accepted, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acceptance {
    /// Note version accepted.
    pub version: u32,
    /// When (RFC 3339).
    pub accepted_at: String,
}

fn record_path(env: &dyn EnvSource) -> Option<PathBuf> {
    data_dir(env).map(|d| d.join("risk-note-accepted.json"))
}

/// Whether the current note was accepted on this machine.
pub fn accepted(env: &dyn EnvSource) -> bool {
    record_path(env)
        .and_then(|p| fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Acceptance>(&b).ok())
        .map(|a| a.version >= RISK_NOTE_VERSION)
        .unwrap_or(false)
}

/// Record acceptance of the current note.
pub fn record(env: &dyn EnvSource, now_rfc3339: &str) -> Result<(), String> {
    let path = record_path(env).ok_or("no per-user data folder (APPDATA or HOME) to record the acceptance in")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let body =
        serde_json::to_vec_pretty(&Acceptance { version: RISK_NOTE_VERSION, accepted_at: now_rfc3339.to_string() }).map_err(|e| e.to_string())?;
    fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use imagegen_core::config::MapEnv;
    use std::path::Path;

    #[test]
    fn acceptance_is_per_user_and_versioned() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new().prefix("n").tempdir_in(base).unwrap();
        let d = dir.path().display().to_string();
        let env = MapEnv::from_pairs(&[("APPDATA", d.as_str()), ("HOME", d.as_str()), ("XDG_DATA_HOME", d.as_str())]);
        assert!(!accepted(&env));
        record(&env, "2026-11-01T00:00:00Z").unwrap();
        assert!(accepted(&env));
        let p = record_path(&env).unwrap();
        fs::write(&p, br#"{"version":0,"accepted_at":"x"}"#).unwrap();
        assert!(!accepted(&env), "an older note version must be shown again");
        assert!(RISK_NOTE.contains("gpt-6-luna") && RISK_NOTE.contains("never uses it"));
    }
}
