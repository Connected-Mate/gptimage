//! Shared test harness: a temporary mod project, an engine wired to a fixture transport, and synthetic images.
//!
//! SYNTHETIC: every provider reply here was written for the tests. Nothing in this file is evidence of what a
//! real provider returns.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use imagegen_core::config::MapEnv;
use imagegen_core::hashing::FixedClock;
use imagegen_core::pixels::{encode_png, Rgba};
use imagegen_core::rubric;
use imagegen_mcp::{Engine, RunMode};
use imagegen_providers::transport::{FixtureTransport, Method};
use serde_json::{json, Value};

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub engine: Engine,
    pub fixture: Arc<FixtureTransport>,
}

pub fn repo_work_dir() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
    std::fs::create_dir_all(&p).unwrap();
    p
}

pub fn temp_project(config: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("p").tempdir_in(repo_work_dir()).unwrap();
    if !config.is_empty() {
        std::fs::create_dir_all(dir.path().join(".hoi4-imagegen")).unwrap();
        std::fs::write(dir.path().join(".hoi4-imagegen/config.toml"), config).unwrap();
    }
    dir
}

pub fn harness(config: &str, env: &[(&str, &str)]) -> Harness {
    let dir = temp_project(config);
    let fixture = Arc::new(FixtureTransport::new());
    // never discover a real game install: tests must not depend on, or read, the developer's game folder
    let no_game = dir.path().join("no-game").display().to_string();
    let mut pairs: Vec<(&str, &str)> = env.to_vec();
    pairs.push(("HOI4_GAME_DIR", &no_game));
    // never touch the developer's real ChatGPT sign-in: the provider gets a private file store per test
    let auth = dir.path().join("chatgpt-auth.json").display().to_string();
    if !pairs.iter().any(|(k, _)| *k == "HOI4_IMAGEGEN_CHATGPT_AUTH") {
        pairs.push(("HOI4_IMAGEGEN_CHATGPT_AUTH", &auth));
    }
    let engine =
        Engine::open(dir.path(), Arc::new(MapEnv::from_pairs(&pairs)), fixture.clone(), Arc::new(FixedClock(1_793_404_800)), RunMode::Cli).unwrap();
    Harness { dir, engine, fixture }
}

pub fn openai_harness() -> Harness {
    harness("[defaults]\nprovider = \"openai\"\n", &[("OPENAI_API_KEY", "sk-test-synthetic-000")])
}

/// A transparent 928x816 image with an off-centre painted subject. Synthetic.
pub fn synthetic_icon_png() -> Vec<u8> {
    let mut img = Rgba::new(928, 816);
    for y in 140..700 {
        for x in 90..560 {
            let dx = (f64::from(x) - 325.0) / 235.0;
            let dy = (f64::from(y) - 420.0) / 280.0;
            if dx * dx + dy * dy < 1.0 {
                img.set_px(x, y, [150 + (x % 60) as u8, 90 + (y % 50) as u8, 50, 255]);
            }
        }
    }
    encode_png(&img).unwrap()
}

pub fn opaque_png() -> Vec<u8> {
    encode_png(&Rgba::filled(928, 816, [40, 90, 160, 255])).unwrap()
}

/// OpenAI-shaped success body around a PNG. Synthetic.
pub fn openai_ok(png: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "created": 1,
        "data": [{"b64_json": base64::engine::general_purpose::STANDARD.encode(png)}],
        "output_format": "png",
        "usage": {"input_tokens": 120, "input_tokens_details": {"text_tokens": 120, "image_tokens": 0}, "output_tokens": 1056}
    }))
    .unwrap()
}

impl Harness {
    pub fn queue_generation(&self, png: &[u8]) {
        self.fixture.clear_route(Method::Post, "/v1/images/generations");
        self.fixture.respond(Method::Post, "/v1/images/generations", 200, &[("x-request-id", "req_synthetic")], openai_ok(png));
    }

    pub fn queue_edit(&self, png: &[u8]) {
        self.fixture.clear_route(Method::Post, "/v1/images/edits");
        self.fixture.respond(Method::Post, "/v1/images/edits", 200, &[("x-request-id", "req_synthetic_edit")], openai_ok(png));
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn read(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.root().join(rel).exists()
    }

    pub fn plan_focus(&self, asset: &str) -> Value {
        let req = serde_json::from_value(json!({
            "family": "focus", "feature_slug": "iron_pact", "asset": asset,
            "brief": "a riveted steel gate under a red banner", "provider": "openai", "quality": "low"
        }))
        .unwrap();
        serde_json::to_value(self.engine.plan(req).unwrap()).unwrap()
    }

    /// plan -> generate -> process, returning (plan, candidate id, processed id).
    pub fn make_processed(&self, asset: &str, key: &str) -> (Value, String, String) {
        self.queue_generation(&synthetic_icon_png());
        let plan = self.plan_focus(asset);
        let gen = self.engine.generate(serde_json::from_value(json!({"plan_id": plan["plan_id"], "idempotency_key": key})).unwrap()).unwrap();
        let cand = gen["candidates"][0]["candidate_id"].as_str().unwrap().to_string();
        let proc = self.engine.process(serde_json::from_value(json!({"candidate_id": cand})).unwrap()).unwrap();
        let pid = proc["artifact_id"].as_str().unwrap().to_string();
        (plan, cand, pid)
    }

    pub fn validate(&self, id: &str) -> Value {
        self.engine.validate(serde_json::from_value(json!({"targets": [id]})).unwrap()).unwrap()
    }

    pub fn preview(&self, id: &str) -> Value {
        self.engine.preview(serde_json::from_value(json!({"target": id})).unwrap()).unwrap()
    }

    pub fn approve(&self, id: &str, rubric_id: &str) -> Value {
        let answers: serde_json::Map<String, Value> = rubric::questions(rubric_id).iter().map(|(k, _)| (k.to_string(), json!(true))).collect();
        self.engine
            .review(serde_json::from_value(json!({"target": id, "verdict": "approved", "reviewer": "test reviewer", "answers": answers})).unwrap())
            .unwrap()
    }

    /// validate, preview and approve in one go; returns the approval id.
    pub fn approve_all(&self, id: &str) -> String {
        assert_eq!(self.validate(id)["status"], "pass", "{:#}", self.validate(id));
        self.preview(id);
        self.approve(id, "icon_v1")["approval_id"].as_str().unwrap().to_string()
    }

    pub fn install(&self, id: &str, approval: &str, extra: Value) -> Result<Value, imagegen_mcp::EngineError> {
        let mut v = json!({"target": id, "approval_id": approval});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        self.engine.install(serde_json::from_value(v).unwrap())
    }
}
