//! The login route's guards: revocation is recorded and reported, N slots shared across processes, pacing per slot,
//! 429 backoff and the fall back to one request at a time.
//!
//! EVIDENCE LABELS. No test here contacts any server (the route is paused after the 2026-11-02 incident).
//! * The 200 stream is the recorded-real response `tests/fixtures/recorded-real/focus-luna-high-2026-11-01.sse`.
//! * The 401 body is DOCUMENTED SHAPE FROM THE INCIDENT REPORT, NOT RECORDED. The report gave only the error text
//!   ("Encountered invalidated oauth token for user") and the code `token_revoked`. The JSON envelope around them
//!   is a guess modelled on OpenAI error objects, so the detection must not depend on the envelope: one test
//!   sends the bare text and one sends a bare 401 with no body at all.
//! * Credentials are SYNTHETIC.

#![allow(clippy::result_large_err)]

use std::path::Path;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use imagegen_core::error::RetryClass;
use imagegen_core::hashing::Clock;
use imagegen_providers::transport::{FixtureTransport, HttpRequest, HttpResponse, HttpTransport, Method, TransportError};
use imagegen_providers::types::{CallContext, CancelToken, ErrorKind, ImageProvider, ImageRequest, ProviderError, ProviderOutput, Sleeper};
use provider_chatgpt_login::provider::CheckOutcome;
use provider_chatgpt_login::route::{self, Route};
use provider_chatgpt_login::store::{CredentialStore, Credentials};
use provider_chatgpt_login::ChatGptLoginProvider;

const RECORDED: &str = include_str!("fixtures/recorded-real/focus-luna-high-2026-11-01.sse");
const BACKEND_PATH: &str = "/backend-api/codex/responses";
/// DOCUMENTED SHAPE FROM THE INCIDENT REPORT, NOT RECORDED.
const REVOKED_BODY: &str =
    r#"{"error":{"message":"Encountered invalidated oauth token for user","type":"invalid_request_error","code":"token_revoked"}}"#;
const NOW: i64 = 1_793_561_534;

fn work_dir() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new().prefix("lg").tempdir_in(base).unwrap()
}

fn request() -> ImageRequest {
    ImageRequest {
        model: provider_chatgpt_login::MODEL.into(),
        prompt: "a fortified concrete wall".into(),
        negative: vec![],
        width: 100,
        height: 88,
        transparent: true,
        quality: None,
        refs: vec![],
        edit: None,
        idempotency_key: None,
        seed: None,
    }
}

fn synthetic(expires_ms: Option<u64>) -> Credentials {
    Credentials {
        access: "access-synthetic".into(),
        refresh: Some("refresh-synthetic".into()),
        account_id: Some("acct-synthetic".into()),
        expires_ms,
        last_refresh: "t".into(),
    }
}

/// A clock the test moves.
struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_secs(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Records every sleep and never waits.
#[derive(Default)]
struct CountingSleeper(Mutex<Vec<Duration>>);
impl CountingSleeper {
    fn total(&self) -> Duration {
        self.0.lock().unwrap().iter().sum()
    }
    fn reset(&self) {
        self.0.lock().unwrap().clear();
    }
}
impl Sleeper for CountingSleeper {
    fn sleep(&self, d: Duration) {
        self.0.lock().unwrap().push(d);
    }
}

fn ctx(sleeper: &Arc<CountingSleeper>) -> CallContext {
    CallContext { cancel: CancelToken::new(), sleeper: sleeper.clone(), jobs: None }
}

struct Rig {
    dir: tempfile::TempDir,
    fixture: Arc<FixtureTransport>,
    clock: Arc<TestClock>,
    provider: ChatGptLoginProvider,
    sleeper: Arc<CountingSleeper>,
}

impl Rig {
    fn new(expires_ms: Option<u64>) -> Self {
        let dir = work_dir();
        let store = CredentialStore::at(dir.path().join("auth.json"));
        store.save(&synthetic(expires_ms)).unwrap();
        let fixture = Arc::new(FixtureTransport::new());
        let clock = Arc::new(TestClock(AtomicI64::new(NOW)));
        let provider = ChatGptLoginProvider::new(store, fixture.clone(), clock.clone());
        Rig { dir, fixture, clock, provider, sleeper: Arc::new(CountingSleeper::default()) }
    }

    fn store(&self) -> CredentialStore {
        CredentialStore::at(self.dir.path().join("auth.json"))
    }

    fn route(&self) -> Route {
        Route::new(self.store().state_dir())
    }

    fn generate(&self) -> Result<ProviderOutput, ProviderError> {
        self.provider.generate(&request(), &ctx(&self.sleeper))
    }

    fn backend_calls(&self) -> usize {
        self.fixture.requests().iter().filter(|r| r.url.ends_with(BACKEND_PATH)).count()
    }
}

#[test]
fn a_401_is_recorded_reported_everywhere_and_no_call_is_made_afterwards() {
    let rig = Rig::new(None);
    rig.fixture.respond(Method::Post, BACKEND_PATH, 401, &[], REVOKED_BODY);

    let e = rig.generate().unwrap_err();
    assert_eq!((e.kind, e.retry), (ErrorKind::Auth, RetryClass::RequiresAuthorization), "never retried automatically");
    assert!(e.message.starts_with("signed out by the server (token revoked at 2026-11-01T19:32:14Z"), "{}", e.message);
    assert!(e.message.contains("code token_revoked): run `hoi4-imagegen login` to sign in again; warning: "), "{}", e.message);
    assert!(
        e.message.ends_with("warning: the ChatGPT-login route dropped to one request at a time on this machine after a sign-out by the server (token_revoked) at 2026-11-01T19:32:14Z; it goes back to several at a time after 2026-11-02T01:32:14Z if no new sign-out or rate limit arrives"),
        "{}",
        e.message
    );
    assert_eq!(rig.backend_calls(), 1);
    assert_eq!(rig.route().with_clock(rig.clock.clone()).effective_concurrency(), 1, "every new caller gets one slot");

    // the record holds a time and a code, never a secret, a header or the body text
    let raw = std::fs::read_to_string(rig.dir.path().join("route-state.json")).unwrap();
    assert!(raw.contains("\"code\": \"token_revoked\"") && raw.contains("\"status\": 401"), "{raw}");
    for secret in ["access-synthetic", "refresh-synthetic", "acct-synthetic", "invalidated oauth token", "Bearer"] {
        assert!(!raw.contains(secret), "the state file must not contain {secret}: {raw}");
    }

    // every later generation says the same and sends nothing, even with a good response queued
    rig.fixture.clear_route(Method::Post, BACKEND_PATH).respond(Method::Post, BACKEND_PATH, 200, &[], RECORDED);
    for _ in 0..2 {
        let again = rig.generate().unwrap_err();
        assert_eq!((again.kind, again.retry), (ErrorKind::Auth, RetryClass::RequiresAuthorization));
        assert!(again.message.contains("signed out by the server"), "{}", again.message);
    }
    assert_eq!(rig.backend_calls(), 1, "no network call after the revocation was recorded");
    assert_eq!(rig.fixture.requests().len(), 1);

    // `login-status` and the doctor's notice read the same record
    let status = provider_chatgpt_login::cli::status_text(&rig.store()).unwrap();
    assert!(status.starts_with("signed out by the server (token revoked at "), "{status}");
    assert!(status.contains("no request is made until you sign in again"));
    assert!(rig.provider.revocation_notice().unwrap().contains("hoi4-imagegen login"));
    let status = provider_chatgpt_login::cli::status_text_at(&rig.store(), NOW).unwrap();
    assert!(status.contains("\nwarning: the ChatGPT-login route dropped to one request at a time on this machine after a sign-out"), "{status}");

    // `login-status --check` makes no call either
    let e = rig.provider.check(&ctx(&rig.sleeper)).unwrap_err();
    assert!(e.message.contains("signed out by the server"));
    assert_eq!(rig.fixture.requests().len(), 1);

    // a fresh login (or logout) clears the revocation, and the route works again, still one request at a time
    rig.route().clear().unwrap();
    assert!(rig.provider.revocation_notice().is_none());
    let out = rig.generate().unwrap();
    assert!(out.notes.iter().any(|n| n.starts_with("warning: the ChatGPT-login route dropped to one request at a time")), "{:?}", out.notes);
    assert_eq!(rig.backend_calls(), 2);

    // six hours after the sign-out the route goes back to the configured number, and the warning is gone
    rig.clock.0.store(NOW + route::FALLBACK_HOLD_SECS, Ordering::SeqCst);
    let out = rig.generate().unwrap();
    assert!(!out.notes.iter().any(|n| n.starts_with("warning:")), "{:?}", out.notes);
    assert_eq!(rig.route().with_clock(rig.clock.clone()).effective_concurrency(), 3);
    assert!(!provider_chatgpt_login::cli::status_text_at(&rig.store(), NOW + route::FALLBACK_HOLD_SECS).unwrap().contains("warning"));
}

#[test]
fn the_bare_incident_text_and_a_bodyless_401_are_detected_too() {
    for (status, body, code) in [
        (401u16, "Encountered invalidated oauth token for user", "http_401"),
        (401, "", "http_401"),
        (403, "Encountered invalidated oauth token for user token_revoked", "token_revoked"),
    ] {
        let rig = Rig::new(None);
        rig.fixture.respond(Method::Post, BACKEND_PATH, status, &[], body);
        let e = rig.generate().unwrap_err();
        assert_eq!(e.kind, ErrorKind::Auth, "{status} {body}");
        assert!(e.message.contains(&format!("code {code}")), "{}", e.message);
        assert!(route::Route::new(rig.store().state_dir()).state().revoked.is_some());
    }
}

#[test]
fn a_revoked_refresh_is_recorded_as_well() {
    let rig = Rig::new(Some(1_000)); // expired, so the refresh goes first
    rig.fixture.respond(Method::Post, "/oauth/token", 401, &[], REVOKED_BODY);
    let e = rig.generate().unwrap_err();
    assert!(e.message.contains("signed out by the server"), "{}", e.message);
    assert_eq!(rig.backend_calls(), 0, "the backend was never reached");
    let st = rig.route().state();
    assert_eq!(st.revoked.as_ref().unwrap().during, "token refresh");
    // the stored tokens are left alone for the user to replace by signing in
    assert!(rig.store().load().unwrap().is_some());
    assert_eq!(rig.fixture.requests().len(), 1);
}

#[test]
fn status_check_refreshes_only_near_expiry_and_reports_the_server_answer() {
    // not near expiry: no network at all, and it says why
    let rig = Rig::new(Some(4_000_000_000_000));
    match rig.provider.check(&ctx(&rig.sleeper)).unwrap() {
        CheckOutcome::NotChecked(why) => assert!(why.contains("no request was made"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(rig.fixture.requests().is_empty());
    let text = provider_chatgpt_login::cli::status_check(&rig.store(), &rig.provider, &ctx(&rig.sleeper)).unwrap();
    assert!(text.contains("server check: not made"), "{text}");

    // near expiry: one refresh, accepted
    let rig = Rig::new(Some(1_000));
    rig.fixture.respond(
        Method::Post,
        "/oauth/token",
        200,
        &[],
        br#"{"access_token":"access-synthetic-2","refresh_token":"refresh-synthetic-2","expires_in":3600}"#.to_vec(),
    );
    assert_eq!(rig.provider.check(&ctx(&rig.sleeper)).unwrap(), CheckOutcome::Accepted);
    assert_eq!(rig.fixture.requests().len(), 1);
    assert!(rig.fixture.requests()[0].url.ends_with("/oauth/token"));

    // near expiry and revoked: reported, recorded
    let rig = Rig::new(Some(1_000));
    rig.fixture.respond(Method::Post, "/oauth/token", 401, &[], REVOKED_BODY);
    let e = provider_chatgpt_login::cli::status_check(&rig.store(), &rig.provider, &ctx(&rig.sleeper)).unwrap_err();
    assert!(e.contains("signed out by the server"), "{e}");
    assert!(rig.route().state().revoked.is_some());
}

#[test]
fn generations_are_paced_across_calls_and_the_stamp_is_shared_through_the_state_file() {
    let rig = Rig::new(None);
    rig.fixture.respond(Method::Post, BACKEND_PATH, 200, &[], RECORDED);

    rig.generate().unwrap();
    assert_eq!(rig.sleeper.total(), Duration::ZERO, "the first generation does not wait");
    assert_eq!(rig.route().state().last_request_epoch, Some(NOW));

    rig.generate().unwrap();
    assert_eq!(rig.sleeper.total(), Duration::from_secs(route::MIN_GAP_SECS as u64), "the second waits out the gap");

    // five seconds later only the rest of the gap is waited
    rig.sleeper.reset();
    rig.clock.0.store(NOW + 5, Ordering::SeqCst);
    rig.generate().unwrap();
    assert_eq!(rig.sleeper.total(), Duration::from_secs(route::MIN_GAP_SECS as u64 - 5));

    // after the gap has passed there is no wait
    rig.sleeper.reset();
    rig.clock.0.store(NOW + 5 + 1000, Ordering::SeqCst);
    rig.generate().unwrap();
    assert_eq!(rig.sleeper.total(), Duration::ZERO);

    // a second provider object (another process in real life) sees the same stamp
    let other = ChatGptLoginProvider::new(rig.store(), rig.fixture.clone(), rig.clock.clone());
    other.generate(&request(), &ctx(&rig.sleeper)).unwrap();
    assert_eq!(rig.sleeper.total(), Duration::from_secs(route::MIN_GAP_SECS as u64));
}

#[test]
fn a_cancel_during_the_pacing_wait_stops_the_call_before_anything_is_sent() {
    let rig = Rig::new(None);
    rig.fixture.respond(Method::Post, BACKEND_PATH, 200, &[], RECORDED);
    rig.generate().unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let c = CallContext { cancel, sleeper: rig.sleeper.clone(), jobs: None };
    let e = rig.provider.generate(&request(), &c).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    assert_eq!(rig.backend_calls(), 1);
}

#[test]
fn a_429_backs_off_exponentially_in_one_line_and_then_succeeds() {
    let rig = Rig::new(None);
    rig.fixture.respond(Method::Post, BACKEND_PATH, 429, &[], "slow down");
    rig.fixture.respond(Method::Post, BACKEND_PATH, 429, &[], "slow down");
    rig.fixture.respond(Method::Post, BACKEND_PATH, 200, &[], RECORDED);

    rig.generate().expect("the third answer is the recorded success");
    assert_eq!(rig.backend_calls(), 3, "the 429s were retried one after the other");
    let waits: Vec<u64> = rig.sleeper.0.lock().unwrap().iter().map(Duration::as_secs).collect();
    assert_eq!(waits.iter().sum::<u64>(), 30 + 60, "30 s, then 60 s: {waits:?}");
    assert!(rig.route().state().rate_limit.is_none(), "a success ends the backoff");
    // every new caller on the machine now gets one slot
    let fallback = rig.route().state().fallback.expect("the 429 dropped the route to one at a time");
    assert_eq!((fallback.reason.as_str(), fallback.code.as_str()), ("rate_limit", "http_429"));
    assert_eq!(rig.route().with_clock(rig.clock.clone()).effective_concurrency(), 1);
}

#[test]
fn after_a_429_the_success_says_the_route_dropped_to_one_at_a_time() {
    let rig = Rig::new(None);
    rig.fixture.respond(Method::Post, BACKEND_PATH, 429, &[], "slow down");
    rig.fixture.respond(Method::Post, BACKEND_PATH, 200, &[], RECORDED);
    let out = rig.generate().unwrap();
    let warning = out.notes.iter().find(|n| n.starts_with("warning:")).expect("a warning note");
    assert_eq!(
        warning,
        "warning: the ChatGPT-login route dropped to one request at a time on this machine after a rate limit (http_429) at 2026-11-01T19:32:14Z; it goes back to several at a time after 2026-11-02T01:32:14Z if no new sign-out or rate limit arrives"
    );
    assert_eq!(rig.provider.fallback_notice().as_deref(), warning.strip_prefix("warning: "));
}

#[test]
fn a_429_that_never_ends_stops_after_the_limit_and_holds_the_next_call_back() {
    let rig = Rig::new(None);
    rig.fixture.respond(Method::Post, BACKEND_PATH, 429, &[], "slow down");

    let e = rig.generate().unwrap_err();
    assert_eq!((e.kind, e.status), (ErrorKind::RateLimited, Some(429)));
    assert!(
        e.message.contains("; warning: the ChatGPT-login route dropped to one request at a time on this machine after a rate limit (http_429)"),
        "{}",
        e.message
    );
    assert_eq!(rig.backend_calls() as u32, 1 + route::RATE_LIMIT_MAX_RETRIES);
    assert_eq!(rig.sleeper.total(), Duration::from_secs(30 + 60 + 120));
    let rl = rig.route().state().rate_limit.unwrap();
    assert_eq!((rl.failures, rl.retry_after_epoch), (4, NOW + 240));

    // the next call, even from another process, first waits out the recorded backoff
    rig.sleeper.reset();
    rig.fixture.clear_route(Method::Post, BACKEND_PATH).respond(Method::Post, BACKEND_PATH, 200, &[], RECORDED);
    rig.generate().unwrap();
    assert_eq!(rig.sleeper.total(), Duration::from_secs(240));
    assert!(rig.route().state().rate_limit.is_none());
}

/// A backend that counts how many requests are inside it at once.
struct Overlap {
    inside: AtomicUsize,
    max: AtomicUsize,
    total: AtomicUsize,
    /// A request waits (up to 3 s) until this many are in flight, so reaching the limit does not depend on timing.
    expect: usize,
}

impl HttpTransport for Overlap {
    fn execute(&self, _req: &HttpRequest, _max: usize, _timeout: Duration) -> Result<HttpResponse, TransportError> {
        let now = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
        for _ in 0..600 {
            if self.inside.load(Ordering::SeqCst) >= self.expect {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.max.fetch_max(self.inside.load(Ordering::SeqCst), Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        self.inside.fetch_sub(1, Ordering::SeqCst);
        self.total.fetch_add(1, Ordering::SeqCst);
        Ok(HttpResponse { status: 200, headers: vec![], body: RECORDED.as_bytes().to_vec() })
    }
}

/// Really waits for lock polls (under a second) and skips the 1 s pacing slices.
struct LockOnlySleeper;
impl Sleeper for LockOnlySleeper {
    fn sleep(&self, d: Duration) {
        if d < Duration::from_secs(1) {
            std::thread::sleep(d);
        }
    }
}

/// `threads` providers, each with its own lock handles as separate processes have, generate at once with `n` slots.
/// Returns the most requests the backend saw in flight together. With `rate_limited`, a 429 is on record first.
fn overlap(n: usize, threads: usize, rate_limited: bool) -> usize {
    let dir = work_dir();
    let store = CredentialStore::at(dir.path().join("auth.json"));
    store.save(&synthetic(None)).unwrap();
    let expect = if rate_limited { 1 } else { n.min(threads) };
    let backend = Arc::new(Overlap { inside: AtomicUsize::new(0), max: AtomicUsize::new(0), total: AtomicUsize::new(0), expect });
    let clock = Arc::new(TestClock(AtomicI64::new(NOW)));
    if rate_limited {
        // an hour ago: the backoff itself is long over, only the fall back to one at a time still holds
        let guard = Route::new(store.state_dir()).acquire(&CancelToken::new(), &LockOnlySleeper).unwrap();
        guard.record_rate_limited(NOW - 3600).unwrap();
    }
    let start = Arc::new(std::sync::Barrier::new(threads));
    let mut handles = Vec::new();
    for _ in 0..threads {
        let p = ChatGptLoginProvider::new(CredentialStore::at(dir.path().join("auth.json")), backend.clone(), clock.clone()).with_concurrency(n);
        let start = start.clone();
        handles.push(std::thread::spawn(move || {
            let c = CallContext { cancel: CancelToken::new(), sleeper: Arc::new(LockOnlySleeper), jobs: None };
            start.wait();
            p.generate(&request(), &c)
        }));
    }
    for h in handles {
        let out = h.join().unwrap().expect("every caller gets its turn");
        assert_eq!(out.notes.iter().any(|n| n.starts_with("warning: the ChatGPT-login route dropped")), rate_limited, "{:?}", out.notes);
    }
    assert_eq!(backend.total.load(Ordering::SeqCst), threads);
    backend.max.load(Ordering::SeqCst)
}

#[test]
fn concurrent_generations_reach_three_and_never_exceed_three() {
    assert_eq!(overlap(3, 7, false), 3);
}

#[test]
fn with_one_slot_concurrent_generations_never_overlap() {
    assert_eq!(overlap(1, 4, false), 1, "N = 1: never two requests in flight");
}

#[test]
fn after_a_recorded_429_concurrent_generations_run_one_at_a_time() {
    assert_eq!(overlap(3, 4, true), 1, "the fall back holds every new caller to one slot");
}

#[test]
fn a_caller_that_waits_too_long_gets_the_clear_error() {
    let rig = Rig::new(None);
    rig.fixture.respond(Method::Post, BACKEND_PATH, 200, &[], RECORDED);
    // other "processes" hold every slot; the sleeper does not sleep, so the bounded wait runs out at once
    let held: Vec<_> = (0..3).map(|_| rig.route().acquire(&CancelToken::new(), &*rig.sleeper).unwrap()).collect();
    let e = rig.generate().unwrap_err();
    assert!(e.message.contains("every ChatGPT-login slot on this machine is in use (3 request(s) at a time); try again shortly"), "{}", e.message);
    assert_eq!(rig.backend_calls(), 0);
    drop(held);
    assert!(rig.generate().is_ok(), "the slots were released by the drop");
}

// ------------------------------------------------------------------ slots shared by separate processes

const CHILD_DIR_ENV: &str = "IG_TEST_SLOT_CHILD_DIR";

/// Run as a child process by `three_processes_hold_the_three_slots_and_a_fourth_caller_waits`: take a slot, report it
/// with a file, and hold it until the parent writes `release`. In a normal test run the variable is unset and this
/// does nothing.
#[test]
fn slot_child_process() {
    let Some(dir) = std::env::var_os(CHILD_DIR_ENV).map(std::path::PathBuf::from) else { return };
    let route = Route::new(Some(dir.clone())).with_concurrency(3);
    let guard = route.acquire_within(&CancelToken::new(), &imagegen_providers::types::RealSleeper, Duration::from_secs(60)).unwrap();
    std::fs::write(dir.join(format!("held-{}", guard.slot())), std::process::id().to_string()).unwrap();
    for _ in 0..1200 {
        if dir.join("release").exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the parent never released the child");
}

#[test]
fn three_processes_hold_the_three_slots_and_a_fourth_caller_waits() {
    let dir = work_dir();
    let exe = std::env::current_exe().unwrap();
    let children: Vec<std::process::Child> = (0..3)
        .map(|_| {
            std::process::Command::new(&exe)
                .args(["slot_child_process", "--exact", "--test-threads=1", "--quiet"])
                .env(CHILD_DIR_ENV, dir.path())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let held = |dir: &Path| (0..6).filter(|s| dir.join(format!("held-{s}")).exists()).collect::<Vec<usize>>();
    for _ in 0..1200 {
        if held(dir.path()).len() == 3 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let slots = held(dir.path());
    // this process is the fourth caller: every slot is taken by another process
    let route = Route::new(Some(dir.path().to_path_buf())).with_concurrency(3);
    let busy = route.acquire_within(&CancelToken::new(), &imagegen_providers::types::RealSleeper, Duration::from_millis(600));
    std::fs::write(dir.path().join("release"), b"").unwrap();
    for mut c in children {
        assert!(c.wait().unwrap().success(), "a child failed");
    }
    assert_eq!(slots, [0, 1, 2], "three processes hold the three slots at once");
    assert!(busy.unwrap_err().message.contains("every ChatGPT-login slot on this machine is in use"));
    assert_eq!(route.acquire(&CancelToken::new(), &imagegen_providers::types::RealSleeper).unwrap().slot(), 0, "released when the processes ended");
}
