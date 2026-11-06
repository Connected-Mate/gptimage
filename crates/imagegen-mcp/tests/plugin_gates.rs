//! The ChatGPT-login provider under decision D-12a, seen through the engine.
//!
//! It is part of every build and registers in local modes without any flag, config entry or environment
//! acknowledgement. Signed in, it is the default provider. In hosted mode it is never registered and can never be
//! selected, whatever the project file asks for. No request is made.

mod common;

use std::sync::Arc;

use common::temp_project;
use imagegen_core::config::MapEnv;
use imagegen_core::hashing::FixedClock;
use imagegen_mcp::{Engine, RunMode};
use imagegen_providers::transport::FixtureTransport;
use provider_chatgpt_login::store::{CredentialStore, Credentials};

fn open(config: &str, mode: RunMode, signed_in: bool, extra: &[(&str, &str)]) -> (tempfile::TempDir, Engine) {
    let dir = temp_project(config);
    let no_game = dir.path().join("no-game").display().to_string();
    let auth = dir.path().join("chatgpt-auth.json");
    if signed_in {
        // SYNTHETIC credentials
        CredentialStore::at(&auth)
            .save(&Credentials {
                access: "access-synthetic".into(),
                refresh: None,
                account_id: Some("acct".into()),
                expires_ms: None,
                last_refresh: "t".into(),
            })
            .unwrap();
    }
    let auth = auth.display().to_string();
    let mut pairs = vec![("HOI4_GAME_DIR", no_game.as_str()), ("HOI4_IMAGEGEN_CHATGPT_AUTH", auth.as_str())];
    pairs.extend_from_slice(extra);
    let engine =
        Engine::open(dir.path(), Arc::new(MapEnv::from_pairs(&pairs)), Arc::new(FixtureTransport::new()), Arc::new(FixedClock(1_793_404_800)), mode)
            .unwrap();
    (dir, engine)
}

fn plan(e: &Engine, provider: Option<&str>) -> Result<serde_json::Value, imagegen_mcp::EngineError> {
    let mut p = serde_json::json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "an iron wall"});
    if let Some(id) = provider {
        p["provider"] = serde_json::Value::String(id.to_string());
    }
    e.plan(serde_json::from_value(p).unwrap()).map(|v| serde_json::to_value(v).unwrap())
}

#[test]
fn local_builds_register_it_without_any_flag() {
    for mode in [RunMode::LocalStdio, RunMode::Cli] {
        let (_d, e) = open("", mode, false, &[]);
        assert!(e.plugin.compiled && e.plugin.enabled, "{:?}", e.plugin.reasons);
        assert!(!e.plugin.signed_in);
        assert!(e.plugin.reasons[0].contains("hoi4-imagegen login"));
        assert!(e.registry.get("chatgpt_login").is_some());
        assert_eq!(e.status()["chatgpt_login"]["model"], format!("gpt-6-luna at {}", provider_chatgpt_login::EFFORT_POLICY));
    }
}

#[test]
fn signed_in_it_is_the_default_provider_and_an_explicit_request_still_wins() {
    let (_d, e) = open("", RunMode::LocalStdio, true, &[("OPENAI_API_KEY", "sk-test-synthetic")]);
    assert!(e.plugin.signed_in);
    let p = plan(&e, None).unwrap();
    assert_eq!(p["provider"], "chatgpt_login", "{p}");
    assert_eq!(p["model"], "gpt-6-luna");
    let p = plan(&e, Some("openai")).unwrap();
    assert_eq!(p["provider"], "openai");
}

#[test]
fn not_signed_in_the_next_configured_provider_is_used_and_none_means_a_sign_in_hint() {
    let (_d, e) = open("", RunMode::LocalStdio, false, &[("OPENAI_API_KEY", "sk-test-synthetic")]);
    assert_eq!(plan(&e, None).unwrap()["provider"], "openai");
    let (_d, e) = open("", RunMode::LocalStdio, false, &[]);
    let err = plan(&e, None).unwrap_err();
    assert!(err.message.contains("hoi4-imagegen login"), "{}", err.message);
}

#[test]
fn the_login_route_refuses_any_other_model() {
    let (_d, e) = open("", RunMode::LocalStdio, true, &[]);
    let mut p =
        serde_json::json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "chatgpt_login", "model": "gpt-6-astra"});
    let r = e.plan(serde_json::from_value(p.clone()).unwrap());
    assert!(r.is_err(), "a different model must be refused at plan time");
    p["model"] = "gpt-6-luna".into();
    assert!(e.plan(serde_json::from_value(p).unwrap()).is_ok());
}

#[test]
fn hosted_mode_can_never_select_it_even_signed_in_and_asked_for() {
    assert!(imagegen_mcp::plugin::refuses_hosted(RunMode::Hosted));
    assert!(!imagegen_mcp::plugin::refuses_hosted(RunMode::LocalStdio));
    let (_d, e) = open("[defaults]\nprovider = \"chatgpt_login\"\n", RunMode::Hosted, true, &[]);
    assert!(!e.plugin.enabled && e.plugin.reasons[0].contains("hosted"), "{:?}", e.plugin.reasons);
    assert!(e.registry.get("chatgpt_login").is_none());
    let err = plan(&e, Some("chatgpt_login")).unwrap_err();
    assert_eq!(err.code, "provider_not_found");
    assert!(err.message.contains("hosted"), "{}", err.message);
    let err = plan(&e, None).unwrap_err();
    assert!(!err.message.is_empty());
}

#[test]
fn the_login_tool_reports_an_existing_sign_in_and_is_refused_hosted() {
    // signed in: nothing is opened and no port is bound
    let (_d, e) = open("", RunMode::LocalStdio, true, &[]);
    let v = e.login().unwrap();
    assert_eq!(v["status"], "already_signed_in");
    assert!(v["state"].as_str().unwrap().contains("signed in"));
    // hosted: refused before anything else, signed in or not
    for signed_in in [true, false] {
        let (_d, e) = open("", RunMode::Hosted, signed_in, &[]);
        let err = e.login().unwrap_err();
        assert_eq!(err.code, "login_unavailable");
        assert!(err.message.contains("D-12a"), "{}", err.message);
    }
    // the status line names the tool that signs in
    let (_d, e) = open("", RunMode::LocalStdio, false, &[]);
    assert!(e.status()["chatgpt_login"]["state"].as_str().unwrap().contains("hoi4.image_login"));
}

/// W5-8: `[providers.chatgpt_login] concurrency` reaches the provider and `hoi4.image_status`, the environment variable
/// wins over it, and the route block shows a fall back to one at a time. No request is made.
#[test]
fn the_configured_concurrency_and_a_fall_back_show_in_the_status_route_block() {
    let (_d, e) = open("", RunMode::LocalStdio, false, &[]);
    let r = &e.status()["chatgpt_login"]["route"];
    assert_eq!((r["concurrency"]["configured"].as_u64(), r["concurrency"]["effective"].as_u64()), (Some(3), Some(3)), "{r}");
    assert!(r["fallback"].is_null());

    let (d, e) = open("[providers.chatgpt_login]\nconcurrency = 2\n", RunMode::LocalStdio, false, &[]);
    assert_eq!(e.status()["chatgpt_login"]["route"]["concurrency"]["configured"], 2);
    let (_d2, e2) = open("[providers.chatgpt_login]\nconcurrency = 2\n", RunMode::LocalStdio, false, &[("HOI4_IMAGEGEN_CHATGPT_CONCURRENCY", "9")]);
    assert_eq!(e2.status()["chatgpt_login"]["route"]["concurrency"]["configured"], 6, "the variable wins and is clamped");

    // SYNTHETIC state: a 429 recorded on this machine a minute ago
    let route = provider_chatgpt_login::route::Route::new(CredentialStore::at(d.path().join("chatgpt-auth.json")).state_dir());
    route.update(|s| s.record_fallback(1_793_404_800 - 60, "rate_limit", "http_429")).unwrap();
    let r = &e.status()["chatgpt_login"]["route"];
    assert_eq!(r["concurrency"]["effective"], 1);
    assert_eq!(r["fallback"]["code"], "http_429");
    assert!(r["fallback"]["message"]
        .as_str()
        .unwrap()
        .starts_with("the ChatGPT-login route dropped to one request at a time on this machine after a rate limit"));

    // hosted mode has no route block
    let (_d, e) = open("", RunMode::Hosted, false, &[]);
    assert!(e.status()["chatgpt_login"].get("route").is_none());
}

/// "Unable to verify model access": the occurrences, the slot pause and the fall back they cause show in
/// `hoi4.image_status`. SYNTHETIC state; the status reads it only.
#[test]
fn model_access_answers_show_in_the_status_route_block() {
    let (d, e) = open("", RunMode::LocalStdio, false, &[]);
    assert!(e.status()["chatgpt_login"]["route"]["model_access_unverified"].is_null());
    let route = provider_chatgpt_login::route::Route::new(CredentialStore::at(d.path().join("chatgpt-auth.json")).state_dir());
    route
        .update(|s| {
            let _ = s.record_model_access(1, 1_793_404_800 - 30);
        })
        .unwrap();
    let r = &e.status()["chatgpt_login"]["route"];
    assert_eq!(r["model_access_unverified"]["count"], 1);
    assert_eq!(r["model_access_unverified"]["slot_pauses"][0]["slot"], 1);
    assert!(r["fallback"].is_null());
    route
        .update(|s| {
            let _ = s.record_model_access(2, 1_793_404_800 - 10);
        })
        .unwrap();
    let r = &e.status()["chatgpt_login"]["route"];
    assert_eq!(r["model_access_unverified"]["count"], 2);
    assert_eq!((r["fallback"]["reason"].as_str(), r["concurrency"]["effective"].as_u64()), (Some("model_access_unverified"), Some(1)));
}
