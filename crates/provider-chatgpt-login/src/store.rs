//! The plugin's own credential file.
//!
//! Upstream resolved a token from an environment variable, then its own file, then the Codex CLI's
//! `~/.codex/auth.json`, and wrote refreshed tokens back into whichever file they came from. That rotates another
//! tool's refresh token behind its back. This store reads and writes one file of its own and nothing else.
//!
//! The file is created with mode 0600 on Unix. Windows has no equivalent in `std`; the file lives in the
//! per-user application data folder, whose default access list is limited to the user, administrators and the
//! system. Treat the file like a password.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use imagegen_core::config::EnvSource;
use serde::{Deserialize, Serialize};

/// Environment variable that moves the credential file (tests, portable installs).
pub const PATH_ENV: &str = "HOI4_IMAGEGEN_CHATGPT_AUTH";

/// Saved credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    /// Access token.
    pub access: String,
    /// Refresh token.
    pub refresh: Option<String>,
    /// ChatGPT account id.
    pub account_id: Option<String>,
    /// Expiry, epoch milliseconds.
    pub expires_ms: Option<u64>,
    /// When the file was last written (RFC 3339).
    pub last_refresh: String,
}

/// Handle to the file.
#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

/// Default location, in the per-user data folder.
pub fn default_path(env: &dyn EnvSource) -> PathBuf {
    if let Some(p) = env.get(PATH_ENV) {
        return PathBuf::from(p);
    }
    let base = if cfg!(windows) {
        env.get("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env.get("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        env.get("XDG_DATA_HOME").map(PathBuf::from).or_else(|| env.get("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("hoi4-imagegen").join("chatgpt-login").join("auth.json")
}

impl CredentialStore {
    /// A store at a path.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        CredentialStore { path: path.into() }
    }

    /// The file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the credentials, if any.
    pub fn load(&self) -> Result<Option<Credentials>, String> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("{} is not a credential file: {e}", self.path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read {}: {e}", self.path.display())),
        }
    }

    /// Write the credentials, replacing the file atomically.
    pub fn save(&self, creds: &Credentials) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
            f.write_all(&serde_json::to_vec_pretty(creds).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
        }
        fs::rename(&tmp, &self.path).map_err(|e| format!("cannot replace {}: {e}", self.path.display()))
    }

    /// Delete the file. Returns whether it existed.
    pub fn delete(&self) -> Result<bool, String> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("cannot delete {}: {e}", self.path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imagegen_core::config::MapEnv;

    fn creds() -> Credentials {
        // SYNTHETIC values
        Credentials { access: "access-synthetic".into(), refresh: Some("refresh-synthetic".into()), account_id: Some("acct-synthetic".into()), expires_ms: Some(1), last_refresh: "2026-10-31T00:00:00Z".into() }
    }

    fn work_dir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new().prefix("c").tempdir_in(base).unwrap()
    }

    #[test]
    fn save_load_delete_round_trip() {
        let dir = work_dir();
        let store = CredentialStore::at(dir.path().join("a/b/auth.json"));
        assert_eq!(store.load().unwrap(), None);
        store.save(&creds()).unwrap();
        assert_eq!(store.load().unwrap(), Some(creds()));
        let mut newer = creds();
        newer.access = "access-2".into();
        store.save(&newer).unwrap();
        assert_eq!(store.load().unwrap().unwrap().access, "access-2");
        assert!(store.delete().unwrap());
        assert!(!store.delete().unwrap());
    }

    #[test]
    fn garbage_is_an_error_not_silently_empty() {
        let dir = work_dir();
        let store = CredentialStore::at(dir.path().join("auth.json"));
        fs::write(store.path(), "not json").unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn the_default_path_is_this_tools_own_and_never_the_codex_file() {
        let env = MapEnv::from_pairs(&[("APPDATA", "C:/Users/x/AppData/Roaming"), ("HOME", "/home/x"), ("XDG_DATA_HOME", "/home/x/.local/share")]);
        let p = default_path(&env);
        let text = p.to_string_lossy().replace('\\', "/");
        assert!(text.contains("hoi4-imagegen/chatgpt-login/auth.json"), "{text}");
        assert!(!text.contains(".codex"), "must never touch the Codex CLI's credentials: {text}");
        let moved = MapEnv::from_pairs(&[(PATH_ENV, "/tmp/elsewhere.json")]);
        assert_eq!(default_path(&moved), PathBuf::from("/tmp/elsewhere.json"));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = work_dir();
        let store = CredentialStore::at(dir.path().join("auth.json"));
        store.save(&creds()).unwrap();
        assert_eq!(fs::metadata(store.path()).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
