//! RECORDED-REAL REPLAY. `tests/fixtures/recorded-real/focus-luna-high-2026-11-01.sse` is a real response from the
//! ChatGPT backend, captured on 2026-11-01 with `HOI4_IMAGEGEN_RECORD_RESPONSES` during the first signed-in probe and
//! scrubbed by `scripts/make_replay_fixture.py`: account-linked identifiers are replaced and the image is the real
//! output downscaled to 64 px. Faults a live service cannot produce on demand (another model, a lower effort, a
//! missing report, a cut stream, a backend failure, expired or rejected tokens, rate limits) are injected into that
//! recorded response. No test here contacts the backend.

use std::path::Path;
use std::sync::Arc;

use imagegen_core::error::RetryClass;
use imagegen_core::hashing::FixedClock;
use imagegen_providers::transport::{FixtureTransport, Method};
use imagegen_providers::types::{CallContext, ErrorKind, ImageProvider, ImageRequest};
use provider_chatgpt_login::store::{CredentialStore, Credentials};
use provider_chatgpt_login::ChatGptLoginProvider;

const RECORDED: &str = include_str!("fixtures/recorded-real/focus-luna-high-2026-11-01.sse");
const BACKEND_PATH: &str = "/backend-api/codex/responses";

fn work_dir() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new().prefix("r").tempdir_in(base).unwrap()
}

fn request() -> ImageRequest {
    ImageRequest {
        model: provider_chatgpt_login::MODEL.into(),
        prompt: "a fortified concrete wall with a steel gate, symbol of national defence".into(),
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

/// A provider with SYNTHETIC credentials whose backend replies `status` with `body`.
fn replay(status: u16, body: String, expires_ms: Option<u64>) -> (tempfile::TempDir, Arc<FixtureTransport>, ChatGptLoginProvider) {
    let dir = work_dir();
    let store = CredentialStore::at(dir.path().join("auth.json"));
    store
        .save(&Credentials {
            access: "access-synthetic".into(),
            refresh: Some("refresh-synthetic".into()),
            account_id: Some("acct-synthetic".into()),
            expires_ms,
            last_refresh: "t".into(),
        })
        .unwrap();
    let fixture = Arc::new(FixtureTransport::new());
    fixture.respond(Method::Post, BACKEND_PATH, status, &[("content-type", "text/event-stream")], body);
    let p = ChatGptLoginProvider::new(store, fixture.clone(), Arc::new(FixedClock(1_793_561_534)));
    (dir, fixture, p)
}

#[test]
fn the_recorded_response_yields_its_real_image_and_proves_luna_at_high_effort() {
    let (_d, _f, p) = replay(200, RECORDED.to_string(), None);
    let out = p.generate(&request(), &CallContext::new()).expect("the recorded response is accepted");
    assert_eq!(&out.image[..8], b"\x89PNG\r\n\x1a\n", "a real PNG");
    let img = image::load_from_memory(&out.image).unwrap();
    assert!(img.width() <= 64 && img.height() <= 64 && img.width() > 0);
    assert_eq!(out.model_echo.as_deref(), Some("gpt-6-luna"));
    assert!(out.notes[0].contains("gpt-6-luna at high reasoning effort"), "{:?}", out.notes);
    assert!(out.notes.iter().any(|n| n == "image background: transparent"), "{:?}", out.notes);
    assert!(out.notes.iter().any(|n| n == "image tool model (chosen by the backend): gpt-image-2-codex"), "{:?}", out.notes);
    assert!(out.notes.iter().any(|n| n == "image quality: medium"), "the backend chose medium although high was asked: {:?}", out.notes);
    assert!(out.notes.iter().any(|n| n.starts_with("revised prompt:")));
    assert_eq!(out.provider_request_id.as_deref(), Some("resp_recorded"));
    let usage = out.usage.expect("the backend reported usage");
    assert_eq!(usage["reasoning_effort"], "high", "the effort the image ran at is data on the candidate");
    assert_eq!(usage["backend_usage"]["total_tokens"], 2690);
    assert!(out.usage_unknown, "plan quota has no price, so spend stays unknown");
}

#[test]
fn the_fixture_carries_no_account_identifiers() {
    assert!(RECORDED.contains("\"safety_identifier\":\"user-REDACTED\""));
    assert!(!RECORDED.contains("resp_0") && !RECORDED.contains("ig_0"), "ids are replaced");
    assert!(!RECORDED.contains("access-") && !RECORDED.contains("refresh_token"));
}

#[test]
fn another_model_injected_into_the_recording_is_refused() {
    for (from, to, needle) in [
        ("\"model\":\"gpt-6-luna\"", "\"model\":\"gpt-6-astra\"", "instead of gpt-6-luna"),
        ("\"effort\":\"high\"", "\"effort\":\"medium\"", "instead of the asked high"),
        ("\"model\":\"gpt-6-luna\",", "", "did not report which model"),
        ("\"effort\":\"high\",", "", "did not report the reasoning effort"),
    ] {
        assert!(RECORDED.contains(from), "the recording contains {from}");
        let (_d, _f, p) = replay(200, RECORDED.replace(from, to), None);
        let e = p.generate(&request(), &CallContext::new()).unwrap_err();
        assert!(e.message.contains(needle), "{to}: {}", e.message);
        assert_eq!(e.retry, RetryClass::Terminal, "never retried, never replaced by another model");
    }
}

#[test]
fn a_stream_cut_before_the_image_is_a_malformed_response_to_reconcile() {
    let cut = RECORDED.find("response.output_item.done").expect("recorded image event");
    let (_d, _f, p) = replay(200, RECORDED[..cut].to_string(), None);
    let e = p.generate(&request(), &CallContext::new()).unwrap_err();
    assert_eq!((e.kind, e.retry), (ErrorKind::MalformedResponse, RetryClass::ReconcileFirst));
}

#[test]
fn a_backend_failure_injected_mid_stream_is_surfaced() {
    let at = RECORDED.find("data: {\"type\":\"response.image_generation_call.generating\"").expect("recorded generating event");
    let failed = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"You've hit your usage limit\"}}}\n\n";
    let body = format!("{}{}{}", &RECORDED[..at], failed, &RECORDED[at..]);
    let (_d, _f, p) = replay(200, body, None);
    let e = p.generate(&request(), &CallContext::new()).unwrap_err();
    assert!(e.message.contains("usage limit"), "{}", e.message);
}

#[test]
fn rejected_tokens_and_rate_limits_are_reported_plainly() {
    let (_d, _f, p) = replay(401, "{\"detail\":\"Unauthorized\"}".into(), None);
    let e = p.generate(&request(), &CallContext::new()).unwrap_err();
    assert_eq!((e.kind, e.retry), (ErrorKind::Auth, RetryClass::RequiresAuthorization));
    let (_d, _f, p) = replay(429, "{\"detail\":\"rate limited\"}".into(), None);
    // the route backs off between tries; this sleeper only records, so the test does not really wait
    struct NoSleep;
    impl imagegen_providers::types::Sleeper for NoSleep {
        fn sleep(&self, _d: std::time::Duration) {}
    }
    let ctx = CallContext { sleeper: Arc::new(NoSleep), ..CallContext::new() };
    assert_eq!(p.generate(&request(), &ctx).unwrap_err().kind, ErrorKind::RateLimited);
}

#[test]
fn an_expired_token_is_refreshed_before_the_recorded_call() {
    let (_d, f, p) = replay(200, RECORDED.to_string(), Some(1_000));
    f.respond(
        Method::Post,
        "/oauth/token",
        200,
        &[],
        br#"{"access_token":"access-synthetic-2","refresh_token":"refresh-synthetic-2","expires_in":3600}"#.to_vec(),
    );
    p.generate(&request(), &CallContext::new()).unwrap();
    let urls: Vec<String> = f.requests().iter().map(|r| r.url.clone()).collect();
    assert!(urls[0].ends_with("/oauth/token") && urls[1].ends_with(BACKEND_PATH), "{urls:?}");
}

/// RECORDED-REAL REPLAY of a refused image (2026-11-01, an officer-corps icon brief during the end-to-end run): the
/// image call ends with `status: failed` and `moderation_blocked`, then the backend answers in text.
const REFUSED: &str = include_str!("fixtures/recorded-real/moderation-blocked-2026-11-01.sse");

#[test]
fn a_moderation_refusal_is_reported_as_such_and_never_retried() {
    assert!(REFUSED.contains("moderation_blocked") && !REFUSED.contains("gAAAA"), "the fixture is the scrubbed refusal");
    let (_d, _f, p) = replay(200, REFUSED.to_string(), None);
    let e = p.generate(&request(), &CallContext::new()).unwrap_err();
    assert_eq!((e.kind, e.retry), (ErrorKind::ModerationBlocked, RetryClass::Terminal));
    assert!(e.message.contains("safety system"), "{}", e.message);
}

// ------------------------------------------------------------------ the 2026-11-05 effort refusals (recorded real)

/// RECORDED-REAL: the backend's answers of 2026-11-05 to gpt-6-luna at high and then at medium effort, captured by the
/// imagegen-docs session with `HOI4_IMAGEGEN_RECORD_RESPONSES` and scrubbed by `scripts/make_replay_fixture.py`.
/// Both echo the model and the asked effort in response.created, then the image tool fails with
/// `image_generation_user_error` / `invalid_value` / param `complexity` ("Complexity '<effort>' is not supported for
/// this image model.") and response.failed.
const HIGH_REFUSED: &str = include_str!("fixtures/recorded-real/effort-high-refused-2026-11-05.sse");
const MEDIUM_REFUSED: &str = include_str!("fixtures/recorded-real/effort-medium-refused-2026-11-05.sse");

fn no_sleep() -> CallContext {
    struct NoSleep;
    impl imagegen_providers::types::Sleeper for NoSleep {
        fn sleep(&self, _d: std::time::Duration) {}
    }
    CallContext { sleeper: Arc::new(NoSleep), ..CallContext::new() }
}

#[test]
fn the_recorded_effort_refusals_are_read_as_effort_refusals() {
    for (fixture, effort) in [(HIGH_REFUSED, "high"), (MEDIUM_REFUSED, "medium")] {
        assert!(fixture.contains("\"param\":\"complexity\"") && fixture.contains(&format!("Complexity '{effort}'")));
        assert!(provider_chatgpt_login::provider::is_effort_refusal(&format!("Complexity '{effort}' is not supported for this image model.")));
    }
}

/// The two recorded refusals, then an accepted low-effort answer. The low answer is the 2026-11-01 recording with its
/// effort changed to low (recorded-real replay with an injected value): no real low-effort response exists yet.
#[test]
fn after_the_recorded_refusals_of_high_and_medium_low_is_used_and_recorded() {
    let (_d, f, p) = replay(200, HIGH_REFUSED.to_string(), None);
    f.respond(Method::Post, BACKEND_PATH, 200, &[("content-type", "text/event-stream")], MEDIUM_REFUSED.to_string());
    f.respond(
        Method::Post,
        BACKEND_PATH,
        200,
        &[("content-type", "text/event-stream")],
        RECORDED.replace("\"effort\":\"high\"", "\"effort\":\"low\""),
    );
    let out = p.generate(&request(), &no_sleep()).unwrap();
    assert_eq!(out.usage.clone().unwrap()["reasoning_effort"], "low");
    assert_eq!(out.usage.unwrap()["refused_efforts"], serde_json::json!(["high", "medium"]));
    assert!(out.notes[0].contains("gpt-6-luna at low reasoning effort"), "{:?}", out.notes);
    let sent: Vec<serde_json::Value> = f.requests().iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect();
    assert_eq!(sent.iter().map(|b| b["reasoning"]["effort"].as_str().unwrap().to_string()).collect::<Vec<_>>(), ["high", "medium", "low"]);
}

#[test]
fn a_refusal_of_every_rung_is_one_terminal_error_and_nothing_goes_without_an_effort() {
    let (_d, f, p) = replay(200, HIGH_REFUSED.to_string(), None);
    f.respond(Method::Post, BACKEND_PATH, 200, &[("content-type", "text/event-stream")], MEDIUM_REFUSED.to_string());
    // injected: low refused, then the request without an effort refused as well (no real recording of these yet)
    let low = MEDIUM_REFUSED.replace("'medium'", "'low'").replace("\"effort\":\"medium\"", "\"effort\":\"low\"");
    f.respond(Method::Post, BACKEND_PATH, 200, &[("content-type", "text/event-stream")], low.clone());
    f.respond(Method::Post, BACKEND_PATH, 200, &[("content-type", "text/event-stream")], low);
    let e = p.generate(&request(), &no_sleep()).unwrap_err();
    assert_eq!((e.kind, e.retry), (ErrorKind::BadRequest, RetryClass::Terminal));
    assert!(e.message.contains("high: Complexity 'high'") && e.message.contains("backend default: Complexity"), "{}", e.message);
    assert_eq!(f.requests().len(), 4, "high, medium, low, then once without an effort");
    let last: serde_json::Value = serde_json::from_slice(&f.requests()[3].body).unwrap();
    assert!(last.get("reasoning").is_none());
}
