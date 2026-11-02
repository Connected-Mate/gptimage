//! The login route's guards: revocation is recorded and reported, one request at a time, pacing and 429 backoff.
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
    assert!(e.message.contains("code token_revoked") && e.message.ends_with("run `hoi4-imagegen login` to sign in again"), "{}", e.message);
    assert_eq!(rig.backend_calls(), 1);

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

    // `login-status --check` makes no call either
    let e = rig.provider.check(&ctx(&rig.sleeper)).unwrap_err();
    assert!(e.message.contains("signed out by the server"));
    assert_eq!(rig.fixture.requests().len(), 1);

    // a fresh login (or logout) clears it, and the route works again
    rig.route().clear().unwrap();
    assert!(rig.provider.revocation_notice().is_none());
    assert!(rig.generate().is_ok());
    assert_eq!(rig.backend_calls(), 2);
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
}

#[test]
fn a_429_that_never_ends_stops_after_the_limit_and_holds_the_next_call_back() {
    let rig = Rig::new(None);
    rig.fixture.respond(Method::Post, BACKEND_PATH, 429, &[], "slow down");

    let e = rig.generate().unwrap_err();
    assert_eq!((e.kind, e.status), (ErrorKind::RateLimited, Some(429)));
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
}

impl HttpTransport for Overlap {
    fn execute(&self, _req: &HttpRequest, _max: usize, _timeout: Duration) -> Result<HttpResponse, TransportError> {
        let now = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(60));
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

#[test]
fn concurrent_generations_from_separate_providers_never_overlap() {
    let dir = work_dir();
    let store = CredentialStore::at(dir.path().join("auth.json"));
    store.save(&synthetic(None)).unwrap();
    let backend = Arc::new(Overlap { inside: AtomicUsize::new(0), max: AtomicUsize::new(0), total: AtomicUsize::new(0) });
    let clock = Arc::new(TestClock(AtomicI64::new(NOW)));
    let mut handles = Vec::new();
    for _ in 0..4 {
        // each thread has its own provider and lock handle, as separate processes would
        let p = ChatGptLoginProvider::new(CredentialStore::at(dir.path().join("auth.json")), backend.clone(), clock.clone());
        handles.push(std::thread::spawn(move || {
            let c = CallContext { cancel: CancelToken::new(), sleeper: Arc::new(LockOnlySleeper), jobs: None };
            p.generate(&request(), &c).map(|_| ())
        }));
    }
    for h in handles {
        h.join().unwrap().expect("every caller gets its turn");
    }
    assert_eq!(backend.total.load(Ordering::SeqCst), 4);
    assert_eq!(backend.max.load(Ordering::SeqCst), 1, "never two requests in flight");
}

#[test]
fn a_caller_that_waits_too_long_gets_the_clear_error() {
    let rig = Rig::new(None);
    rig.fixture.respond(Method::Post, BACKEND_PATH, 200, &[], RECORDED);
    // another "process" holds the route; the sleeper does not sleep, so the bounded wait runs out at once
    let held = rig.route().acquire(&CancelToken::new(), &*rig.sleeper).unwrap();
    let e = rig.generate().unwrap_err();
    assert!(e.message.contains("another hoi4-imagegen process is using the ChatGPT login; try again shortly"), "{}", e.message);
    assert_eq!(rig.backend_calls(), 0);
    drop(held);
    assert!(rig.generate().is_ok(), "the lock was released by the drop");
}
