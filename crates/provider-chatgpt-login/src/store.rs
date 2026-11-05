//! Where the sign-in tokens live: the operating system's credential store.
//!
//! Windows keeps them in Credential Manager and macOS in the login Keychain. Linux uses the Secret Service
//! (GNOME Keyring, KWallet) when one is running. Only on a Linux machine without a keyring does the store fall
//! back to a private file (mode 0600) in the per-user data folder.
//!
//! Upstream resolved a token from an environment variable, then its own file, then the Codex CLI's
//! `~/.codex/auth.json`, and wrote refreshed tokens back into whichever file they came from. That rotates another
//! tool's refresh token behind its back. This store reads and writes its own entries and nothing else.
//!
//! Windows limits one credential to 2,560 bytes and the tokens are longer, so the JSON is split across numbered
//! entries (`auth.0`, `auth.1`, ...) with a count in `auth`. A credential file written by version 0.1 is moved
//! into the keychain on first use and then deleted.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use imagegen_core::config::EnvSource;
use serde::{Deserialize, Serialize};

/// Environment variable that forces the file store at a path (tests, portable installs).
pub const PATH_ENV: &str = "HOI4_IMAGEGEN_CHATGPT_AUTH";
/// Keychain service name.
pub const SERVICE: &str = "hoi4-imagegen.chatgpt-login";
/// Characters per keychain entry. Windows stores UTF-16, so 1,000 characters fit its 2,560-byte limit.
const CHUNK: usize = 1000;
/// Upper bound on chunks, so a corrupt count cannot loop for long.
const MAX_CHUNKS: usize = 64;

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
    /// When the credentials were last written (RFC 3339).
    pub last_refresh: String,
}

#[derive(Debug, Clone)]
enum Backend {
    Keychain { legacy_file: Option<PathBuf> },
    File(PathBuf),
}

/// Handle to the credential store.
#[derive(Debug, Clone)]
pub struct CredentialStore {
    backend: Backend,
}

/// Per-user data folder of this tool, or `None` when the environment names no home.
pub fn data_dir(env: &dyn EnvSource) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        env.get("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env.get("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        env.get("XDG_DATA_HOME").map(PathBuf::from).or_else(|| env.get("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.map(|b| b.join("hoi4-imagegen").join("chatgpt-login"))
}

/// The version 0.1 credential file location (and the Linux fallback file).
pub fn file_path(env: &dyn EnvSource) -> Option<PathBuf> {
    data_dir(env).map(|d| d.join("auth.json"))
}

impl CredentialStore {
    /// The store for this machine: the keychain, or the forced file from [`PATH_ENV`], or on Linux without a
    /// keyring the private file. Fails only when no store is usable at all.
    pub fn for_environment(env: &dyn EnvSource) -> Result<Self, String> {
        if let Some(p) = env.get(PATH_ENV) {
            return Ok(Self::at(p));
        }
        if keychain_usable() {
            return Ok(CredentialStore { backend: Backend::Keychain { legacy_file: file_path(env) } });
        }
        if cfg!(any(windows, target_os = "macos")) {
            return Err("the operating system credential store is not reachable".into());
        }
        file_path(env)
            .map(Self::at)
            .ok_or_else(|| "no keyring is running and neither XDG_DATA_HOME nor HOME is set, so there is nowhere private to keep the sign-in".into())
    }

    /// A file store at a path (tests, the Linux fallback).
    pub fn at(path: impl Into<PathBuf>) -> Self {
        CredentialStore { backend: Backend::File(path.into()) }
    }

    /// Where the credentials live, for messages.
    pub fn location(&self) -> String {
        match &self.backend {
            Backend::Keychain { .. } => {
                format!("operating system credential store (service {SERVICE})")
            }
            Backend::File(p) => p.display().to_string(),
        }
    }

    /// The per-user folder for this store's side files (the cross-process lock and the route state). It is the
    /// folder of the credential file, or the per-user data folder for the keychain. `None` only when the
    /// environment names no home, and then the route fails closed instead of guessing a folder.
    pub fn state_dir(&self) -> Option<PathBuf> {
        match &self.backend {
            Backend::File(p) => p.parent().map(Path::to_path_buf),
            Backend::Keychain { legacy_file } => legacy_file.as_ref().and_then(|f| f.parent()).map(Path::to_path_buf),
        }
    }

    /// The file path, when this is a file store.
    pub fn path(&self) -> Option<&Path> {
        match &self.backend {
            Backend::File(p) => Some(p),
            Backend::Keychain { .. } => None,
        }
    }

    /// Read the credentials, if any.
    pub fn load(&self) -> Result<Option<Credentials>, String> {
        match &self.backend {
            Backend::File(p) => load_file(p),
            Backend::Keychain { legacy_file } => {
                if let Some(json) = keychain_read()? {
                    return serde_json::from_str(&json).map(Some).map_err(|e| format!("the stored sign-in is damaged ({e}); sign in again"));
                }
                // one-time move of a version 0.1 file into the keychain
                if let Some(old) = legacy_file {
                    if let Some(creds) = load_file(old)? {
                        keychain_write(&serde_json::to_string(&creds).map_err(|e| e.to_string())?)?;
                        let _ = fs::remove_file(old);
                        return Ok(Some(creds));
                    }
                }
                Ok(None)
            }
        }
    }

    /// Write the credentials.
    pub fn save(&self, creds: &Credentials) -> Result<(), String> {
        let json = serde_json::to_string(creds).map_err(|e| e.to_string())?;
        match &self.backend {
            Backend::File(p) => save_file(p, json.as_bytes()),
            Backend::Keychain { .. } => keychain_write(&json),
        }
    }

    /// Delete the credentials. Returns whether any existed.
    pub fn delete(&self) -> Result<bool, String> {
        match &self.backend {
            Backend::File(p) => delete_file(p),
            Backend::Keychain { legacy_file } => {
                let mut existed = keychain_delete()?;
                if let Some(old) = legacy_file {
                    existed |= delete_file(old)?;
                }
                Ok(existed)
            }
        }
    }
}

fn load_file(path: &Path) -> Result<Option<Credentials>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("{} is not a credential file: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

fn save_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
}

fn delete_file(path: &Path) -> Result<bool, String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("cannot delete {}: {e}", path.display())),
    }
}

fn entry(name: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, name).map_err(|e| format!("credential store: {e}"))
}

fn keychain_usable() -> bool {
    matches!(entry("probe").map(|e| e.get_password()), Ok(Ok(_)) | Ok(Err(keyring::Error::NoEntry)))
}

fn get(name: &str) -> Result<Option<String>, String> {
    match entry(name)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("credential store: {e}")),
    }
}

fn keychain_read() -> Result<Option<String>, String> {
    let Some(count) = get("auth")? else {
        return Ok(None);
    };
    let n: usize = count.parse().map_err(|_| "the stored sign-in is damaged (bad chunk count); sign in again".to_string())?;
    if n == 0 || n > MAX_CHUNKS {
        return Err("the stored sign-in is damaged (bad chunk count); sign in again".into());
    }
    let mut json = String::new();
    for i in 0..n {
        json.push_str(&get(&format!("auth.{i}"))?.ok_or("the stored sign-in is incomplete; sign in again")?);
    }
    Ok(Some(json))
}

fn keychain_write(json: &str) -> Result<(), String> {
    let chars: Vec<char> = json.chars().collect();
    let chunks: Vec<String> = chars.chunks(CHUNK).map(|c| c.iter().collect()).collect();
    if chunks.len() > MAX_CHUNKS {
        return Err("the sign-in tokens are unexpectedly large".into());
    }
    let old = get("auth")?.and_then(|c| c.parse::<usize>().ok()).unwrap_or(0).min(MAX_CHUNKS);
    for (i, c) in chunks.iter().enumerate() {
        entry(&format!("auth.{i}"))?.set_password(c).map_err(|e| format!("credential store: {e}"))?;
    }
    entry("auth")?.set_password(&chunks.len().to_string()).map_err(|e| format!("credential store: {e}"))?;
    for i in chunks.len()..old {
        let _ = entry(&format!("auth.{i}")).map(|e| e.delete_credential());
    }
    Ok(())
}

fn keychain_delete() -> Result<bool, String> {
    let Some(count) = get("auth")? else {
        return Ok(false);
    };
    let n = count.parse::<usize>().unwrap_or(MAX_CHUNKS).min(MAX_CHUNKS);
    for i in 0..n {
        let _ = entry(&format!("auth.{i}")).map(|e| e.delete_credential());
    }
    entry("auth")?.delete_credential().map_err(|e| format!("credential store: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imagegen_core::config::MapEnv;

    fn creds() -> Credentials {
        // SYNTHETIC values
        Credentials {
            access: "access-synthetic".into(),
            refresh: Some("refresh-synthetic".into()),
            account_id: Some("acct-synthetic".into()),
            expires_ms: Some(1),
            last_refresh: "2026-10-31T00:00:00Z".into(),
        }
    }

    fn work_dir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new().prefix("c").tempdir_in(base).unwrap()
    }

    #[test]
    fn file_store_round_trip() {
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
        fs::write(store.path().unwrap(), "not json").unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn the_forced_path_wins_and_paths_are_never_the_codex_file() {
        let dir = work_dir();
        let forced = dir.path().join("forced.json").display().to_string();
        let env = MapEnv::from_pairs(&[(PATH_ENV, forced.as_str())]);
        let store = CredentialStore::for_environment(&env).unwrap();
        assert_eq!(store.path().unwrap().display().to_string(), forced);

        let env = MapEnv::from_pairs(&[("APPDATA", "C:/Users/x/AppData/Roaming"), ("HOME", "/home/x"), ("XDG_DATA_HOME", "/home/x/.local/share")]);
        let text = file_path(&env).unwrap().to_string_lossy().replace('\\', "/");
        assert!(text.contains("hoi4-imagegen/chatgpt-login/auth.json"), "{text}");
        assert!(!text.contains(".codex"), "must never touch the Codex CLI's credentials: {text}");
    }

    #[test]
    fn no_home_means_no_file_path_never_the_current_directory() {
        assert_eq!(file_path(&MapEnv::from_pairs(&[])), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = work_dir();
        let store = CredentialStore::at(dir.path().join("auth.json"));
        store.save(&creds()).unwrap();
        assert_eq!(fs::metadata(store.path().unwrap()).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn long_tokens_split_into_chunks_that_fit_windows() {
        let json = "x".repeat(CHUNK * 3 + 7);
        let chars: Vec<char> = json.chars().collect();
        let chunks: Vec<String> = chars.chunks(CHUNK).map(|c| c.iter().collect()).collect();
        assert_eq!(chunks.len(), 4);
        assert!(chunks.iter().all(|c| c.encode_utf16().count() * 2 <= 2560));
        assert_eq!(chunks.concat(), json);
    }
}
