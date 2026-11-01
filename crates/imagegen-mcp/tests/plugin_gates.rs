//! The ChatGPT-login plugin's gates, seen through the engine.
//!
//! Without the cargo feature, a project that asks for the plugin gets nothing and is told why. With the feature
//! (run `cargo test -p imagegen-mcp --features unofficial-chatgpt-login`), the plugin registers only when the config
//! flag, the environment acknowledgement and a local mode all hold. No request is made in either case.

mod common;

use std::sync::Arc;

use common::temp_project;
use imagegen_core::config::MapEnv;
use imagegen_core::hashing::FixedClock;
use imagegen_mcp::{Engine, RunMode};
use imagegen_providers::transport::FixtureTransport;

fn open(config: &str, ack: bool, mode: RunMode) -> (tempfile::TempDir, Engine) {
    let dir = temp_project(config);
    let no_game = dir.path().join("no-game").display().to_string();
    let auth = dir.path().join("chatgpt-auth.json").display().to_string();
    let mut pairs = vec![("HOI4_GAME_DIR", no_game.as_str()), ("HOI4_IMAGEGEN_CHATGPT_AUTH", auth.as_str())];
    if ack {
        pairs.push(("HOI4_IMAGEGEN_ACCEPT_UNOFFICIAL", "1"));
    }
    let engine = Engine::open(dir.path(), Arc::new(MapEnv::from_pairs(&pairs)), Arc::new(FixtureTransport::new()), Arc::new(FixedClock(1_793_404_800)), mode).unwrap();
    (dir, engine)
}

const ENABLED: &str = "[providers.chatgpt_login]\nenabled = true\n";

#[cfg(not(feature = "unofficial-chatgpt-login"))]
#[test]
fn a_default_build_ignores_the_request_and_says_why() {
    let (_d, e) = open(ENABLED, true, RunMode::LocalStdio);
    assert!(!e.plugin.compiled && !e.plugin.enabled);
    assert_eq!(e.plugin.state, "not compiled in");
    assert!(e.plugin.reasons[0].contains("does not contain the plugin"), "{:?}", e.plugin.reasons);
    assert!(e.registry.get("chatgpt_login").is_none());
    let s = e.status();
    assert_eq!(s["chatgpt_login"]["compiled"], false);
    let err = e
        .plan(serde_json::from_value(serde_json::json!({"category": "focus_icon", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "chatgpt_login"})).unwrap())
        .unwrap_err();
    assert_eq!(err.code, "provider_not_found");
    // not asking for it: no noise
    let (_d2, e2) = open("", false, RunMode::LocalStdio);
    assert!(e2.plugin.reasons.is_empty());
}

#[cfg(feature = "unofficial-chatgpt-login")]
#[test]
fn every_gate_is_needed_when_the_feature_is_compiled_in() {
    // nothing asked, nothing acknowledged
    let (_d, e) = open("", false, RunMode::LocalStdio);
    assert!(e.plugin.compiled && !e.plugin.enabled);
    assert_eq!(e.plugin.reasons.len(), 2, "{:?}", e.plugin.reasons);
    assert!(e.registry.get("chatgpt_login").is_none());

    // the project file alone cannot switch it on
    let (_d, e) = open(ENABLED, false, RunMode::LocalStdio);
    assert!(!e.plugin.enabled);
    assert!(e.plugin.reasons.iter().any(|r| r.contains("HOI4_IMAGEGEN_ACCEPT_UNOFFICIAL")), "{:?}", e.plugin.reasons);
    assert!(e.registry.get("chatgpt_login").is_none());

    // the environment alone cannot either
    let (_d, e) = open("", true, RunMode::LocalStdio);
    assert!(!e.plugin.enabled && e.plugin.reasons.iter().any(|r| r.contains("enabled = true")));

    // a hosted mode is refused even with both
    let (_d, e) = open(ENABLED, true, RunMode::Hosted);
    assert!(!e.plugin.enabled && e.plugin.reasons.iter().any(|r| r.contains("hosted")), "{:?}", e.plugin.reasons);
    assert!(e.registry.get("chatgpt_login").is_none());

    // both gates and a local mode: registered, but not signed in, so not configured, and it never reports a price
    for mode in [RunMode::LocalStdio, RunMode::Cli] {
        let (_d, e) = open(ENABLED, true, mode);
        assert!(e.plugin.enabled, "{:?}", e.plugin.reasons);
        let p = e.registry.get("chatgpt_login").expect("registered");
        assert!(!p.is_configured(), "no credential file exists yet");
        assert_eq!(e.status()["chatgpt_login"]["enabled"], true);
    }
}
