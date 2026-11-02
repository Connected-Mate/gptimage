//! Keeping the ChatGPT-login route polite: one request at a time on the machine, a minimum gap between
//! generations, backoff on 429, and a durable record that the server revoked the sign-in.
//!
//! Background (incident, 2026-11-02, about 14:29 to 14:40 UTC): every Codex-client OAuth grant on the maintainer's
//! account was revoked by the server while several sessions generated through the Codex client id at once. The
//! likely trigger is concurrency. So every request this route makes (a generation, a token refresh, the sign-in
//! code exchange) holds one exclusive, cross-process lock, and generations are spaced out.
//!
//! Two files live next to the tokens in the per-user data folder (see [`crate::store::CredentialStore::state_dir`]):
//! * `route.lock` is never read or written. It only carries an operating system file lock, which the system
//!   releases when the holding process dies, so a crash cannot leave the route stuck.
//! * `route-state.json` holds [`RouteState`]: the time of the last generation request (for pacing), the 429
//!   backoff, and the revocation record. It never contains a token, a header or a response body.
//!
//! Waiting for the lock or for the pacing gap honours the call's cancel token and uses the call's sleeper, so
//! tests never really wait.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use imagegen_core::error::RetryClass;
use imagegen_core::hashing::rfc3339_utc;
use imagegen_providers::types::{CancelToken, ErrorKind, ProviderError, Sleeper};
use serde::{Deserialize, Serialize};

/// Smallest gap between two generation requests on this machine, across every process.
pub const MIN_GAP_SECS: i64 = 20;
/// Longest a caller waits for another hoi4-imagegen process to finish its request.
pub const LOCK_WAIT_MAX: Duration = Duration::from_secs(600);
/// How often a waiting caller tries the lock again.
pub const LOCK_POLL: Duration = Duration::from_millis(250);
/// First backoff after a 429. Doubles with every further 429 in a row.
pub const RATE_LIMIT_BACKOFF_BASE_SECS: i64 = 30;
/// The backoff never grows beyond this.
pub const RATE_LIMIT_BACKOFF_MAX_SECS: i64 = 300;
/// How many times one generation is retried after a 429 before the error is returned.
pub const RATE_LIMIT_MAX_RETRIES: u32 = 3;
/// Waits for pacing and backoff sleep in slices of this length so a cancel is noticed within a second.
const SLEEP_SLICE: Duration = Duration::from_secs(1);

const LOCK_FILE: &str = "route.lock";
const STATE_FILE: &str = "route-state.json";
const STATE_VERSION: u32 = 1;

/// The sentence every revoked report starts with is built here, so status, doctor and generation agree.
pub const RELOGIN_HINT: &str = "run `hoi4-imagegen login` to sign in again";

/// The record that the server revoked the sign-in. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revoked {
    /// When this machine first saw the revocation (RFC 3339, UTC).
    pub at: String,
    /// The same instant in epoch seconds.
    pub at_epoch: i64,
    /// A short error code: `token_revoked` when the body said so, else the body's own error code, else `http_<status>`.
    pub code: String,
    /// The HTTP status of the answer.
    pub status: u16,
    /// Which call saw it: `generation`, `token refresh`.
    pub during: String,
}

/// The 429 backoff in force.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    /// 429 answers in a row.
    pub failures: u32,
    /// No generation request before this epoch second.
    pub retry_after_epoch: i64,
}

/// Everything the route remembers between processes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RouteState {
    /// Format version.
    #[serde(default)]
    pub version: u32,
    /// Set once the server revoked the sign-in; cleared by `login` and `logout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked: Option<Revoked>,
    /// Epoch second of the last generation request that was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_request_epoch: Option<i64>,
    /// The backoff after a 429, if one is in force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimit>,
}

impl RouteState {
    /// The sentence that tells the user the server signed them out. `None` when there is no revocation.
    pub fn revoked_message(&self) -> Option<String> {
        self.revoked.as_ref().map(|r| format!("signed out by the server (token revoked at {}, code {}): {RELOGIN_HINT}", r.at, r.code))
    }
}

/// The error every call returns while a revocation is recorded. No request is made.
pub fn revoked_error(r: &Revoked) -> ProviderError {
    ProviderError::new(
        ErrorKind::Auth,
        RetryClass::RequiresAuthorization,
        format!("signed out by the server (token revoked at {}, code {}): {RELOGIN_HINT}", r.at, r.code),
    )
    .with_status(r.status)
}

/// Whether an answer means the server invalidated the sign-in: any 401, or a body that says so.
pub fn is_revocation(status: u16, body: &[u8]) -> bool {
    if status == 401 {
        return true;
    }
    let lower = String::from_utf8_lossy(body).to_ascii_lowercase();
    lower.contains("token_revoked") || lower.contains("invalidated oauth token")
}

/// A short, safe error code from an answer body. Never the body itself.
pub fn error_code(status: u16, body: &[u8]) -> String {
    let lower = String::from_utf8_lossy(body).to_ascii_lowercase();
    if lower.contains("token_revoked") {
        return "token_revoked".into();
    }
    let from_json = serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| {
        v.pointer("/error/code").or_else(|| v.get("code")).or_else(|| v.get("error")).and_then(serde_json::Value::as_str).map(str::to_string)
    });
    match from_json {
        Some(c) if !c.is_empty() && c.len() <= 64 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.') => c,
        _ => format!("http_{status}"),
    }
}

/// The backoff after `failures` 429 answers in a row, in seconds.
pub fn backoff_secs(failures: u32) -> i64 {
    let shift = failures.saturating_sub(1).min(16);
    (RATE_LIMIT_BACKOFF_BASE_SECS.saturating_mul(1_i64 << shift)).min(RATE_LIMIT_BACKOFF_MAX_SECS)
}

/// The route's folder: its lock and its state.
#[derive(Debug, Clone)]
pub struct Route {
    dir: Option<PathBuf>,
}

impl Route {
    /// The route for a state folder. `None` makes every lock attempt fail closed.
    pub fn new(dir: Option<PathBuf>) -> Self {
        Route { dir }
    }

    fn dir(&self) -> Result<&Path, ProviderError> {
        self.dir.as_deref().ok_or_else(|| {
            ProviderError::new(
                ErrorKind::NotConfigured,
                RetryClass::RequiresAuthorization,
                "no per-user data folder (APPDATA, HOME or XDG_DATA_HOME) to keep the ChatGPT login's lock and state in, so no request is made",
            )
        })
    }

    /// Read the state. A missing or damaged file reads as the empty state.
    pub fn state(&self) -> RouteState {
        let Some(dir) = &self.dir else { return RouteState::default() };
        read_state(&dir.join(STATE_FILE))
    }

    /// Forget a recorded revocation and any backoff. Called by a successful `login` and by `logout`.
    pub fn clear(&self) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let mut st = read_state(&dir.join(STATE_FILE));
        st.revoked = None;
        st.rate_limit = None;
        write_state(&dir.join(STATE_FILE), &st)
    }

    /// Take the exclusive lock, waiting politely. Honours `cancel` and gives up after [`LOCK_WAIT_MAX`].
    pub fn acquire(&self, cancel: &CancelToken, sleeper: &dyn Sleeper) -> Result<RouteGuard, ProviderError> {
        self.acquire_within(cancel, sleeper, LOCK_WAIT_MAX)
    }

    /// [`Route::acquire`] with an explicit bound (tests).
    pub fn acquire_within(&self, cancel: &CancelToken, sleeper: &dyn Sleeper, max_wait: Duration) -> Result<RouteGuard, ProviderError> {
        let dir = self.dir()?;
        fs::create_dir_all(dir)
            .map_err(|e| ProviderError::new(ErrorKind::Transport, RetryClass::Terminal, format!("cannot create {}: {e}", dir.display())))?;
        let lock_path = dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| ProviderError::new(ErrorKind::Transport, RetryClass::Terminal, format!("cannot open {}: {e}", lock_path.display())))?;
        let polls = (max_wait.as_millis() / LOCK_POLL.as_millis().max(1)).max(1);
        for _ in 0..=polls {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            match file.try_lock() {
                Ok(()) => return Ok(RouteGuard { _file: file, state_path: dir.join(STATE_FILE) }),
                Err(TryLockError::WouldBlock) => sleeper.sleep(LOCK_POLL),
                Err(TryLockError::Error(e)) => {
                    return Err(ProviderError::new(ErrorKind::Transport, RetryClass::Terminal, format!("cannot lock {}: {e}", lock_path.display())));
                }
            }
        }
        Err(ProviderError::new(
            ErrorKind::Transport,
            RetryClass::SafeSameInputs,
            "another hoi4-imagegen process is using the ChatGPT login; try again shortly (the route sends one request at a time so OpenAI does not revoke the sign-in)",
        ))
    }
}

/// Records every sleep and never waits (tests).
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CountingSleeper(pub std::sync::Mutex<Vec<Duration>>);

#[cfg(test)]
impl CountingSleeper {
    pub(crate) fn total(&self) -> Duration {
        self.0.lock().unwrap().iter().sum()
    }
}

#[cfg(test)]
impl Sleeper for CountingSleeper {
    fn sleep(&self, d: Duration) {
        self.0.lock().unwrap().push(d);
    }
}

fn cancelled() -> ProviderError {
    ProviderError::new(ErrorKind::Cancelled, RetryClass::SafeSameInputs, "cancelled while waiting for the ChatGPT login")
}

/// Proof that this process holds the route's lock. Dropping it (or the process dying) releases the lock.
#[derive(Debug)]
pub struct RouteGuard {
    _file: File,
    state_path: PathBuf,
}

impl RouteGuard {
    /// The current state.
    pub fn state(&self) -> RouteState {
        read_state(&self.state_path)
    }

    /// Change the state and save it. A save that fails is reported, never ignored: pacing and revocation depend on it.
    pub fn update(&self, f: impl FnOnce(&mut RouteState)) -> Result<(), ProviderError> {
        let mut st = self.state();
        f(&mut st);
        write_state(&self.state_path, &st).map_err(|e| ProviderError::new(ErrorKind::Transport, RetryClass::Terminal, e))
    }

    /// Record that the server revoked the sign-in. Stores the time and a short code, never the body.
    pub fn record_revoked(&self, now_secs: i64, status: u16, body: &[u8], during: &str) -> Revoked {
        let revoked = Revoked { at: rfc3339_utc(now_secs), at_epoch: now_secs, code: error_code(status, body), status, during: during.to_string() };
        let copy = revoked.clone();
        // Even if the save fails the caller still gets the error; the next call will see the 401 again.
        let _ = self.update(|s| s.revoked = Some(copy));
        revoked
    }

    /// Wait until `gap_until` (epoch seconds) has passed, in slices, honouring `cancel`. The wait is computed once
    /// from `now_secs`, so a sleeper that does not really sleep ends it at once.
    pub fn wait_until(&self, now_secs: i64, until_secs: i64, cancel: &CancelToken, sleeper: &dyn Sleeper) -> Result<(), ProviderError> {
        let mut remaining = (until_secs - now_secs).max(0) as u64;
        while remaining > 0 {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            let slice = remaining.min(SLEEP_SLICE.as_secs());
            sleeper.sleep(Duration::from_secs(slice));
            remaining -= slice;
        }
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        Ok(())
    }

    /// The earliest second the next generation request may be sent, from the pacing gap and any backoff.
    pub fn next_allowed(&self, now_secs: i64) -> i64 {
        let st = self.state();
        let mut at = now_secs;
        if let Some(last) = st.last_request_epoch {
            // a last-request time in the future (clock moved back) never holds a caller for more than one gap
            at = at.max((last + MIN_GAP_SECS).min(now_secs + MIN_GAP_SECS));
        }
        if let Some(rl) = &st.rate_limit {
            at = at.max(rl.retry_after_epoch.min(now_secs + RATE_LIMIT_BACKOFF_MAX_SECS));
        }
        at
    }
}

fn read_state(path: &Path) -> RouteState {
    fs::read(path).ok().and_then(|b| serde_json::from_slice::<RouteState>(&b).ok()).unwrap_or_default()
}

fn write_state(path: &Path, st: &RouteState) -> Result<(), String> {
    let mut st = st.clone();
    st.version = STATE_VERSION;
    let bytes = serde_json::to_vec_pretty(&st).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = File::create(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn work_dir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new().prefix("rt").tempdir_in(base).unwrap()
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!([1, 2, 3, 4, 5, 6, 40].map(backoff_secs), [30, 60, 120, 240, 300, 300, 300]);
    }

    #[test]
    fn revocation_is_recognised_by_status_or_body() {
        assert!(is_revocation(401, b""));
        assert!(is_revocation(403, br#"{"error":{"code":"token_revoked"}}"#));
        assert!(is_revocation(400, b"Encountered invalidated oauth token for user"));
        assert!(!is_revocation(429, b"slow down"));
        assert!(!is_revocation(500, b"boom"));
    }

    #[test]
    fn the_error_code_is_short_and_never_the_body() {
        assert_eq!(error_code(401, b"Encountered invalidated oauth token for user token_revoked"), "token_revoked");
        assert_eq!(error_code(401, br#"{"error":{"code":"expired_token"}}"#), "expired_token");
        assert_eq!(error_code(401, br#"{"error":"invalid_grant"}"#), "invalid_grant");
        assert_eq!(error_code(401, b"access-token-secret-looking text"), "http_401");
        assert_eq!(error_code(401, br#"{"error":{"code":"has spaces and secret"}}"#), "http_401");
    }

    #[test]
    fn state_round_trips_and_a_damaged_file_reads_as_empty() {
        let d = work_dir();
        let route = Route::new(Some(d.path().to_path_buf()));
        assert_eq!(route.state(), RouteState::default());
        let guard = route.acquire(&CancelToken::new(), &CountingSleeper::default()).unwrap();
        guard.record_revoked(1_793_000_000, 401, b"token_revoked", "generation");
        drop(guard);
        let st = route.state();
        assert_eq!(st.revoked.as_ref().unwrap().code, "token_revoked");
        assert!(st.revoked_message().unwrap().contains("hoi4-imagegen login"));
        route.clear().unwrap();
        assert_eq!(route.state().revoked, None);
        fs::write(d.path().join(STATE_FILE), "garbage").unwrap();
        assert_eq!(route.state(), RouteState::default());
    }

    #[test]
    fn the_lock_is_exclusive_and_released_on_drop() {
        let d = work_dir();
        let route = Route::new(Some(d.path().to_path_buf()));
        let first = route.acquire(&CancelToken::new(), &CountingSleeper::default()).unwrap();
        let sleeper = CountingSleeper::default();
        let e = route.acquire_within(&CancelToken::new(), &sleeper, Duration::from_secs(1)).unwrap_err();
        assert!(e.message.contains("another hoi4-imagegen process is using the ChatGPT login; try again shortly"), "{}", e.message);
        assert_eq!(e.retry, RetryClass::SafeSameInputs);
        assert!(!sleeper.0.lock().unwrap().is_empty(), "it waited through the sleeper, not by spinning");
        drop(first);
        route.acquire(&CancelToken::new(), &CountingSleeper::default()).expect("free again after the guard dropped");
    }

    #[test]
    fn a_waiting_caller_honours_cancel() {
        let d = work_dir();
        let route = Route::new(Some(d.path().to_path_buf()));
        let _held = route.acquire(&CancelToken::new(), &CountingSleeper::default()).unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let e = route.acquire(&cancel, &CountingSleeper::default()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Cancelled);
    }

    /// Cancels the token after its first sleep.
    struct CancelOnSleep(CancelToken, AtomicUsize);
    impl Sleeper for CancelOnSleep {
        fn sleep(&self, _d: Duration) {
            self.1.fetch_add(1, Ordering::SeqCst);
            self.0.cancel();
        }
    }

    #[test]
    fn a_cancel_during_the_wait_is_noticed() {
        let d = work_dir();
        let route = Route::new(Some(d.path().to_path_buf()));
        let _held = route.acquire(&CancelToken::new(), &CountingSleeper::default()).unwrap();
        let cancel = CancelToken::new();
        let sleeper = CancelOnSleep(cancel.clone(), AtomicUsize::new(0));
        let e = route.acquire(&cancel, &sleeper).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Cancelled);
        assert_eq!(sleeper.1.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn two_threads_never_hold_the_route_together() {
        let d = work_dir();
        let route = Arc::new(Route::new(Some(d.path().to_path_buf())));
        let inside = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let (route, inside, max_seen) = (route.clone(), inside.clone(), max_seen.clone());
            handles.push(std::thread::spawn(move || {
                for _ in 0..3 {
                    let guard = route.acquire(&CancelToken::new(), &imagegen_providers::types::RealSleeper).unwrap();
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    max_seen.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(15));
                    inside.fetch_sub(1, Ordering::SeqCst);
                    drop(guard);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(max_seen.load(Ordering::SeqCst), 1, "the lock serialises callers");
    }

    #[test]
    fn pacing_waits_for_the_gap_and_the_backoff_and_ignores_a_future_stamp() {
        let d = work_dir();
        let route = Route::new(Some(d.path().to_path_buf()));
        let guard = route.acquire(&CancelToken::new(), &CountingSleeper::default()).unwrap();
        assert_eq!(guard.next_allowed(1000), 1000, "nothing recorded: no wait");
        guard.update(|s| s.last_request_epoch = Some(995)).unwrap();
        assert_eq!(guard.next_allowed(1000), 1015, "20 s after the last request");
        guard.update(|s| s.rate_limit = Some(RateLimit { failures: 2, retry_after_epoch: 1060 })).unwrap();
        assert_eq!(guard.next_allowed(1000), 1060, "the backoff is longer than the gap");
        guard.update(|s| s.last_request_epoch = Some(1_000_000)).unwrap();
        assert!(guard.next_allowed(1000) <= 1060, "a stamp far in the future (clock moved back) cannot hold anyone for long");

        let sleeper = CountingSleeper::default();
        guard.wait_until(1000, 1003, &CancelToken::new(), &sleeper).unwrap();
        assert_eq!(sleeper.total(), Duration::from_secs(3));
    }

    #[test]
    fn no_data_folder_fails_closed() {
        let e = Route::new(None).acquire(&CancelToken::new(), &CountingSleeper::default()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotConfigured);
    }
}
