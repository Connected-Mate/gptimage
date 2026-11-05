//! The review's vanilla gate through the engine: the side-by-side preview, the measured vanilla-match check and the
//! recorded override.
//!
//! SYNTHETIC: the "installed game" here is a folder the test fills with images it makes. No Paradox art is involved,
//! and provider replies come from the fixture transport.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{repo_work_dir, synthetic_icon_png, temp_project, Harness};
use imagegen_core::config::MapEnv;
use imagegen_core::hashing::FixedClock;
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba};
use imagegen_core::records::kind;
use imagegen_mcp::{Engine, RunMode};
use imagegen_providers::transport::FixtureTransport;
use serde_json::{json, Value};

/// A harness whose game folder is `game` and whose cache folder is inside the project's temp folder.
fn harness_with_game(game: &Path, cache: &Path) -> Harness {
    let dir = temp_project("[defaults]\nprovider = \"openai\"\n");
    let fixture = Arc::new(FixtureTransport::new());
    let auth = dir.path().join("chatgpt-auth.json").display().to_string();
    let env = MapEnv::from_pairs(&[
        ("OPENAI_API_KEY", "sk-test-synthetic-000"),
        ("HOI4_GAME_DIR", &game.display().to_string()),
        ("HOI4_IMAGEGEN_CHATGPT_AUTH", &auth),
        ("HOI4_IMAGEGEN_CACHE_DIR", &cache.display().to_string()),
    ]);
    let engine = Engine::open(dir.path(), Arc::new(env), fixture.clone(), Arc::new(FixedClock(1_793_404_800)), RunMode::Cli).unwrap();
    Harness { dir, engine, fixture }
}

/// A fake game whose vanilla focus icons are `images` (100x88 PNGs in gfx/interface/goals).
fn fake_game(images: &[Rgba]) -> tempfile::TempDir {
    let game = tempfile::Builder::new().prefix("g").tempdir_in(repo_work_dir()).unwrap();
    std::fs::create_dir_all(game.path().join("interface")).unwrap();
    let goals = game.path().join("gfx/interface/goals");
    std::fs::create_dir_all(&goals).unwrap();
    for (i, img) in images.iter().enumerate() {
        std::fs::write(goals.join(format!("focus_fake_{i:02}.png")), encode_png(img).unwrap()).unwrap();
    }
    game
}

/// Small vanilla-sized discs: the "vanilla" of the mismatch case, far smaller on the canvas than the candidate.
fn small_disc(i: u32) -> Rgba {
    let mut img = Rgba::new(100, 88);
    let r = 14.0 + f64::from(i % 3);
    for y in 0..88u32 {
        for x in 0..100u32 {
            let (dx, dy) = (f64::from(x) + 0.5 - 50.0, f64::from(y) + 0.5 - 44.0);
            if dx * dx + dy * dy < r * r {
                img.set_px(x, y, [170, 140 + (i * 3) as u8, 70, 255]);
            }
        }
    }
    img
}

fn tinted(img: &Rgba, d: u8) -> Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        p[0] = p[0].saturating_add(d);
    }
    out
}

fn answers() -> Value {
    imagegen_core::rubric::questions("icon_v3").iter().map(|(k, _)| (k.to_string(), json!(true))).collect::<serde_json::Map<_, _>>().into()
}

/// The processed candidate of a fresh harness, as pixels, made the same way in every harness.
fn processed_pixels(h: &Harness, id: &str) -> Rgba {
    let rel = h
        .engine
        .ledger
        .entries()
        .unwrap()
        .into_iter()
        .rev()
        .find(|e| e.artifact_id.as_deref() == Some(id) && e.kind == kind::PROCESSED)
        .unwrap()
        .data["path"]
        .as_str()
        .unwrap()
        .to_string();
    decode(&h.read(&rel), Some(&rel), DecodeLimits::default()).unwrap().0
}

#[test]
fn a_candidate_that_measures_like_vanilla_is_approved_after_the_side_by_side_preview() {
    // first make the candidate without a game, to learn its pixels; the fake vanilla is then made from them
    let probe_cache = tempfile::Builder::new().prefix("c").tempdir_in(repo_work_dir()).unwrap();
    let empty = fake_game(&[]);
    let probe = harness_with_game(empty.path(), probe_cache.path());
    let (_, _, pid) = probe.make_processed("steel_gate", "k-probe");
    let pixels = processed_pixels(&probe, &pid);

    let game = fake_game(&(0..6).map(|i| tinted(&pixels, i * 4)).collect::<Vec<_>>());
    let cache = tempfile::Builder::new().prefix("c").tempdir_in(repo_work_dir()).unwrap();
    let h = harness_with_game(game.path(), cache.path());
    let (_, _, id) = h.make_processed("steel_gate", "k-gate");
    assert_eq!(h.validate(&id)["status"], "pass");

    let pr = h.preview(&id);
    assert_eq!(pr["vanilla_check"]["status"], "pass", "{:#}", pr["vanilla_check"]);
    let sheet = &pr["vanilla_sheet"];
    assert_eq!(sheet["references"].as_array().unwrap().len(), 6, "{sheet:#}");
    // the vanilla sheet is written outside the mod, in the cache folder
    let path = sheet["path"].as_str().unwrap();
    assert!(
        Path::new(path).starts_with(cache.path()) || path.replace('\\', "/").starts_with(&cache.path().display().to_string().replace('\\', "/")),
        "{path}"
    );
    assert!(Path::new(path).is_file());
    let in_mod: Vec<_> = walk(h.root()).into_iter().filter(|p| p.contains("vanilla")).collect();
    assert!(in_mod.is_empty(), "no vanilla sheet inside the mod: {in_mod:?}");
    // the preview returns both sheets
    let (_, images) = h.engine.preview_with_image(&serde_json::from_value(json!({"target": id})).unwrap()).unwrap();
    assert_eq!(images.len(), 2);

    let rev = h
        .engine
        .review(serde_json::from_value(json!({"target": id, "verdict": "approved", "reviewer": "t", "answers": answers()})).unwrap())
        .unwrap();
    assert!(rev["approval_id"].as_str().unwrap().starts_with("apr-"));
    assert_eq!(rev["vanilla_check"]["status"], "pass");
    assert!(rev["vanilla_override"].is_null());
}

#[test]
fn a_candidate_unlike_vanilla_is_refused_and_an_override_is_recorded() {
    let game = fake_game(&(0..8).map(small_disc).collect::<Vec<_>>());
    let cache = tempfile::Builder::new().prefix("c").tempdir_in(repo_work_dir()).unwrap();
    let h = harness_with_game(game.path(), cache.path());
    h.queue_generation(&synthetic_icon_png());
    let (_, _, id) = h.make_processed("steel_gate", "k-gate");
    assert_eq!(h.validate(&id)["status"], "pass");
    let pr = h.preview(&id);
    assert_eq!(pr["vanilla_check"]["status"], "fail");

    // every rubric answer true is not enough: the measurement refuses it and names the features
    let e = h
        .engine
        .review(serde_json::from_value(json!({"target": id, "verdict": "approved", "reviewer": "t", "answers": answers()})).unwrap())
        .unwrap_err();
    assert_eq!(e.code, "review_vanilla_mismatch", "{e}");
    assert!(e.message.contains("subject_span"), "{}", e.message);
    assert!(!e.details.as_ref().unwrap()["vanilla_check"]["outside"].as_array().unwrap().is_empty());

    // a deliberate mod style passes only with a recorded override
    let reason = "the mod's focus icons are deliberately large frameless emblems";
    let rev = h
        .engine
        .review(
            serde_json::from_value(json!({"target": id, "verdict": "approved", "reviewer": "t", "answers": answers(), "override_vanilla_check": true, "override_reason": reason}))
                .unwrap(),
        )
        .unwrap();
    assert!(rev["approval_id"].as_str().unwrap().starts_with("apr-"));
    assert_eq!(rev["vanilla_override"]["reason"], reason);
    assert_eq!(rev["vanilla_override"]["status_overridden"], "fail");
    let rec = h.engine.ledger.entries().unwrap().into_iter().rev().find(|e| e.kind == kind::REVIEWED).unwrap();
    assert_eq!(rec.data["vanilla_override"]["reason"], reason);
    assert_eq!(rec.data["vanilla_check"]["status"], "fail");

    // a non-approval records the check too, and never needs an override
    let rev = h
        .engine
        .review(serde_json::from_value(json!({"target": id, "verdict": "changes_requested", "reviewer": "t", "answers": answers()})).unwrap())
        .unwrap();
    assert!(rev["approval_id"].is_null());
    assert_eq!(rev["vanilla_check"]["status"], "fail");
}

fn walk(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.display().to_string());
            }
        }
    }
    out
}
