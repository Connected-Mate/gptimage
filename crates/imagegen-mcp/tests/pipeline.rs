//! L2: the whole pipeline into a temporary mod, with a synthetic provider transport.
//!
//! SYNTHETIC: provider replies are written for these tests. They prove this tool's behaviour (placement,
//! versions, ledger, gates, refusals), not what any provider returns.

mod common;

use common::*;
use imagegen_core::dds;
use imagegen_core::hashing::sha256_hex;
use imagegen_core::pixels::{decode, DecodeLimits, Rgba};
use imagegen_providers::transport::{Method, TransportError};
use serde_json::{json, Value};

fn code(e: &imagegen_mcp::EngineError) -> &str {
    &e.code
}

#[test]
fn focus_icon_goes_from_plan_to_installed_dds_with_a_shine_sprite() {
    let h = openai_harness();
    let (plan, cand, processed) = h.make_processed("steel_gate", "k-steel-1");

    // plan facts
    assert_eq!(plan["provider"], "openai");
    assert_eq!(plan["model"], "gpt-image-2.6-flare");
    assert_eq!(plan["canvas"], json!([100, 88]));
    assert_eq!(plan["paths"]["final_files"], json!(["gfx/interface/goals/iron_pact/focus_steel_gate.dds"]));
    assert!(plan["estimate"]["basis"].as_str().unwrap().contains("2026-10-31"));
    assert!(plan["prompt"]["text"].as_str().unwrap().contains("riveted steel gate"));

    // the raw candidate is stored unchanged and named -v1
    assert_eq!(cand, "iron_pact.steel_gate-v1");
    let raw = h.read("docs/assets/iron_pact/source_png/steel_gate-v1.png");
    assert_eq!(raw, synthetic_icon_png(), "the provider output is stored byte for byte");
    assert!(h.exists("docs/assets/iron_pact/prompts/steel_gate-v1.md"));
    assert_eq!(processed, "iron_pact.steel_gate-v1-p1");

    let approval = h.approve_all(&processed);
    let out = h.install(&processed, &approval, json!({})).unwrap();

    // the DDS is canonical and decodes to the processed pixels
    let dds_bytes = h.read("gfx/interface/goals/iron_pact/focus_steel_gate.dds");
    assert!(dds::canonical_problems(&dds_bytes).is_empty());
    let info = dds::parse_header(&dds_bytes).unwrap();
    assert_eq!((info.width, info.height, info.mip_levels), (100, 88, 1));
    let decoded = dds::decode(&dds_bytes).unwrap();
    assert_eq!(decoded.pixels_sha256(), out["pixels_sha256"].as_str().unwrap());
    let processed_png = h.read("docs/assets/iron_pact/processed_png/steel_gate-v1-p1.png");
    let (processed_img, _) = decode(&processed_png, None, DecodeLimits::default()).unwrap();
    assert_eq!(processed_img.pixels_sha256(), decoded.pixels_sha256());
    let b = decoded.alpha_bbox(16).unwrap();
    assert!(b[0] >= 2 && b[1] >= 2 && 100 - b[2] >= 2 && 88 - b[3] >= 2, "margins {b:?}");

    // handoff snippet, with the shine companion and the in-game finding that the shine is required
    let handoff = String::from_utf8(h.read("docs/assets/iron_pact/gfx_handoff.md")).unwrap();
    assert!(handoff.contains("GFX_focus_steel_gate") && handoff.contains("GFX_focus_steel_gate_shine"));
    assert!(handoff.contains("interface/iron_pact_goals.gfx") && handoff.contains("required"));
    assert!(!h.exists("interface/iron_pact_goals.gfx"), "handoff mode must not touch .gfx files");

    // provenance
    assert!(h.exists(".hoi4-imagegen/records/iron_pact.steel_gate.json"));
    let chain = h.engine.ledger_query(serde_json::from_value(json!({"artifact_id": processed, "lineage": true})).unwrap()).unwrap();
    assert_eq!(chain["chain"]["ok"], true);
    assert_eq!(chain["lineage"], json!([processed, cand]));
    let entries = h.engine.ledger.entries().unwrap();
    let kinds: Vec<&str> = entries.iter().map(|e| e.kind.as_str()).collect();
    for k in ["plan_created", "job_intent", "generated", "job_ended", "processed", "validated", "previewed", "reviewed", "installed"] {
        assert!(kinds.contains(&k), "ledger lacks {k}: {kinds:?}");
    }
}

#[test]
fn install_refuses_until_validation_review_and_hashes_are_in_order() {
    let h = openai_harness();
    let (_plan, _cand, processed) = h.make_processed("steel_gate", "k-gate-1");

    // nothing recorded yet
    let e = h.install(&processed, "apr-0000000000000000", json!({})).unwrap_err();
    assert_eq!(code(&e), "install_gate_validation_missing");
    assert!(e.message.contains("install_gate_review_missing"), "all reasons are listed: {}", e.message);

    // validated but not reviewed
    assert_eq!(h.validate(&processed)["status"], "pass");
    let e = h.install(&processed, "apr-0000000000000000", json!({})).unwrap_err();
    assert_eq!(code(&e), "install_gate_review_missing");

    // approving without a preview is refused
    let answers: serde_json::Map<String, Value> =
        imagegen_core::rubric::questions("icon_v3").iter().map(|(k, _)| (k.to_string(), json!(true))).collect();
    let plain = |verdict: &str, answers: Value| {
        h.engine.review(serde_json::from_value(json!({"target": processed, "verdict": verdict, "reviewer": "t", "answers": answers})).unwrap())
    };
    // the harness has no installed game, so every approval below records an override of the vanilla check
    let review = |verdict: &str, answers: Value| {
        h.engine.review(
            serde_json::from_value(json!({"target": processed, "verdict": verdict, "reviewer": "t", "answers": answers, "override_vanilla_check": true, "override_reason": "synthetic test art, no installed game"}))
                .unwrap(),
        )
    };
    assert_eq!(review("approved", json!(answers)).unwrap_err().code, "review_needs_preview");
    h.preview(&processed);
    // blanket true answers alone no longer approve: without the vanilla side-by-side sheet and check it is refused
    let e = plain("approved", json!(answers)).unwrap_err();
    assert_eq!(e.code, "review_needs_vanilla_preview", "{e}");
    assert_eq!(e.details.as_ref().unwrap()["vanilla_check"]["status"], "unavailable");
    // an override needs a reason
    let e = h
        .engine
        .review(
            serde_json::from_value(
                json!({"target": processed, "verdict": "approved", "reviewer": "t", "answers": answers, "override_vanilla_check": true}),
            )
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(e.code, "invalid_argument");
    // an incomplete rubric is refused
    let mut partial = answers.clone();
    partial.insert("vanilla_form".into(), json!(false));
    partial.remove("no_text_or_watermark");
    let e = review("approved", json!(partial)).unwrap_err();
    assert_eq!(e.code, "review_incomplete");
    assert!(e.message.contains("vanilla_form") && e.message.contains("no_text_or_watermark"));
    // the subject question is required, and the refusal and the reply quote the brief it is answered against
    let mut no_subject = answers.clone();
    no_subject.remove("shows_brief_subject");
    let e = review("approved", json!(no_subject)).unwrap_err();
    assert_eq!(e.code, "review_incomplete");
    assert!(e.message.contains("shows_brief_subject"), "{e}");
    assert_eq!(e.details.as_ref().unwrap()["subject"]["brief"], "a riveted steel gate under a red banner");
    // a rejection issues no approval
    let rej = review("rejected", json!(partial)).unwrap();
    assert!(rej["approval_id"].is_null());
    let e = h.install(&processed, "apr-0000000000000000", json!({})).unwrap_err();
    assert_eq!(code(&e), "install_gate_approval_mismatch", "a rejection issues no approval id, so no id can match");

    // a real approval opens the gate...
    let approved = review("approved", json!(answers)).unwrap();
    assert_eq!(approved["subject"]["answer"], true);
    // the override and its reason are in the reply and the ledger
    assert_eq!(approved["vanilla_override"]["reason"], "synthetic test art, no installed game");
    assert_eq!(approved["vanilla_override"]["status_overridden"], "unavailable");
    let rec = h.engine.ledger.entries().unwrap().into_iter().rev().find(|e| e.kind == imagegen_core::records::kind::REVIEWED).unwrap();
    assert_eq!(rec.data["vanilla_override"]["reason"], "synthetic test art, no installed game");
    assert_eq!(rec.data["vanilla_check"]["status"], "unavailable");
    let approval = approved["approval_id"].as_str().unwrap().to_string();
    // ...but only with its own id
    assert_eq!(code(&h.install(&processed, "apr-ffffffffffffffff", json!({})).unwrap_err()), "install_gate_approval_mismatch");

    // changing one pixel of the processed file withdraws the approval
    let path = h.root().join("docs/assets/iron_pact/processed_png/steel_gate-v1-p1.png");
    let original = std::fs::read(&path).unwrap();
    let (mut img, _) = decode(&original, None, DecodeLimits::default()).unwrap();
    img.set_px(50, 44, [1, 2, 3, 255]);
    std::fs::write(&path, imagegen_core::pixels::encode_png(&img).unwrap()).unwrap();
    let e = h.install(&processed, &approval, json!({})).unwrap_err();
    assert!(e.message.contains("install_gate_hash_changed"), "{e}");
    assert!(!h.exists("gfx/interface/goals/iron_pact/focus_steel_gate.dds"), "nothing may be written when the gate is closed");
    std::fs::write(&path, &original).unwrap();

    // editing the raw candidate is caught too
    let raw_path = h.root().join("docs/assets/iron_pact/source_png/steel_gate-v1.png");
    let raw = std::fs::read(&raw_path).unwrap();
    std::fs::write(&raw_path, b"tampered").unwrap();
    let e = h.install(&processed, &approval, json!({})).unwrap_err();
    assert!(e.message.contains("raw candidate"), "{e}");
    std::fs::write(&raw_path, raw).unwrap();

    // and so is an edited plan
    let plan_file = std::fs::read_dir(h.root().join(".hoi4-imagegen/plans")).unwrap().next().unwrap().unwrap().path();
    let plan_text = std::fs::read_to_string(&plan_file).unwrap();
    std::fs::write(&plan_file, plan_text.replace("riveted steel gate", "something else")).unwrap();
    let e = h.install(&processed, &approval, json!({})).unwrap_err();
    assert!(e.message.contains("install_gate_plan_changed"), "{e}");
    std::fs::write(&plan_file, plan_text).unwrap();

    // everything restored: the gate opens
    assert!(h.install(&processed, &approval, json!({})).is_ok());
}

#[test]
fn installed_files_are_never_overwritten_unless_this_tool_made_them() {
    let h = openai_harness();
    let (_p, _c, processed) = h.make_processed("steel_gate", "k-own-1");
    let approval = h.approve_all(&processed);
    h.install(&processed, &approval, json!({})).unwrap();
    let first = h.read("gfx/interface/goals/iron_pact/focus_steel_gate.dds");

    // second install: stop by default
    let e = h.install(&processed, &approval, json!({})).unwrap_err();
    assert_eq!(code(&e), "final_path_exists");
    assert_eq!(h.read("gfx/interface/goals/iron_pact/focus_steel_gate.dds"), first);

    // replace is allowed for a file the ledger shows this tool installed
    let out = h.install(&processed, &approval, json!({"on_exists": "replace"})).unwrap();
    assert_eq!(out["replaced"][0]["path"], "gfx/interface/goals/iron_pact/focus_steel_gate.dds");
    assert_eq!(out["replaced"][0]["previous_sha256"], sha256_hex(&first));

    // a file this tool did not make is never replaced, even on request
    std::fs::create_dir_all(h.root().join("gfx/interface/goals/iron_pact")).unwrap();
    let (_p2, _c2, processed2) = h.make_processed("other_gate", "k-own-2");
    std::fs::write(h.root().join("gfx/interface/goals/iron_pact/focus_other_gate.dds"), b"someone else's art").unwrap();
    let approval2 = h.approve_all(&processed2);
    let e = h.install(&processed2, &approval2, json!({"on_exists": "replace"})).unwrap_err();
    assert_eq!(code(&e), "final_path_not_owned");
    assert_eq!(h.read("gfx/interface/goals/iron_pact/focus_other_gate.dds"), b"someone else's art");

    // a file this tool installed but that was edited since is not replaced either
    std::fs::write(h.root().join("gfx/interface/goals/iron_pact/focus_steel_gate.dds"), b"hand edited").unwrap();
    let e = h.install(&processed, &approval, json!({"on_exists": "replace"})).unwrap_err();
    assert_eq!(code(&e), "final_path_not_owned");
}

#[test]
fn versions_count_up_and_nothing_is_overwritten() {
    let h = openai_harness();
    let (_p, c1, _) = h.make_processed("steel_gate", "k-v-1");
    let first = h.read("docs/assets/iron_pact/source_png/steel_gate-v1.png");
    h.queue_generation(&opaque_png());
    let plan = h.plan_focus("steel_gate");
    let g2 = h.engine.generate(serde_json::from_value(json!({"plan_id": plan["plan_id"], "idempotency_key": "k-v-2"})).unwrap()).unwrap();
    assert_eq!(c1, "iron_pact.steel_gate-v1");
    assert_eq!(g2["candidates"][0]["candidate_id"], "iron_pact.steel_gate-v2");
    assert_eq!(h.read("docs/assets/iron_pact/source_png/steel_gate-v1.png"), first, "v1 is untouched");
    assert_eq!(h.read("docs/assets/iron_pact/source_png/steel_gate-v2.png"), opaque_png());
    // processing twice gives p1 and p2, not an overwrite
    let again = h.engine.process(serde_json::from_value(json!({"candidate_id": "iron_pact.steel_gate-v1"})).unwrap()).unwrap();
    assert_eq!(again["artifact_id"], "iron_pact.steel_gate-v1-p2");
    assert!(h.exists("docs/assets/iron_pact/processed_png/steel_gate-v1-p1.png"));
}

#[test]
fn idempotency_replays_without_paying_and_a_reused_key_is_caught() {
    let h = openai_harness();
    h.queue_generation(&synthetic_icon_png());
    let plan = h.plan_focus("steel_gate");
    let run = |key: &str| h.engine.generate(serde_json::from_value(json!({"plan_id": plan["plan_id"], "idempotency_key": key})).unwrap());
    let a = run("same-key").unwrap();
    let b = run("same-key").unwrap();
    assert_eq!(a["candidates"][0]["candidate_id"], b["candidates"][0]["candidate_id"]);
    assert_eq!(b["candidates"][0]["replayed"], true);
    let posts = h.fixture.requests().iter().filter(|r| r.method == Method::Post).count();
    assert_eq!(posts, 1, "a replay must not call the provider again");
    assert!(!h.exists("docs/assets/iron_pact/source_png/steel_gate-v2.png"));

    // the same key for a different request is a caller bug
    let other = h.plan_focus("different_gate");
    let e = h.engine.generate(serde_json::from_value(json!({"plan_id": other["plan_id"], "idempotency_key": "same-key"})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "idempotency_key_reused");
    // bad keys
    assert_eq!(code(&run("has space").unwrap_err()), "invalid_argument");
}

#[test]
fn cost_above_the_project_limit_needs_confirmation() {
    let h = harness("[defaults]\nprovider = \"openai\"\n[policy]\nmax_usd_per_call = 0.001\n", &[("OPENAI_API_KEY", "sk-test-synthetic-000")]);
    h.queue_generation(&synthetic_icon_png());
    let plan = h.plan_focus("steel_gate");
    assert!(plan["approvals"].as_array().unwrap().iter().any(|a| a.as_str().unwrap().contains("per-call limit")));
    let e = h.engine.generate(serde_json::from_value(json!({"plan_id": plan["plan_id"], "idempotency_key": "k-cost"})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "requires_authorization");
    assert!(h.fixture.requests().is_empty(), "no request before the confirmation");
    let ok =
        h.engine.generate(serde_json::from_value(json!({"plan_id": plan["plan_id"], "idempotency_key": "k-cost", "approve_cost": true})).unwrap());
    assert!(ok.is_ok());
}

#[test]
fn plan_budget_is_enforced_before_any_spend() {
    let h = openai_harness();
    let req = serde_json::from_value(json!({"family": "focus", "feature_slug": "iron_pact", "asset": "g", "brief": "x", "provider": "openai", "quality": "high", "model": "gpt-image-2", "budget_usd": 0.01})).unwrap();
    let e = h.engine.plan(req).unwrap_err();
    assert_eq!(code(&e), "budget_exceeded");
}

#[test]
fn policy_gates_follow_the_maintainer_decisions() {
    let h = harness(
        "[defaults]\nprovider = \"openai\"\n[policy]\nforbidden_families = [\"idea_spirit\"]\n",
        &[("OPENAI_API_KEY", "sk-test-synthetic-000")],
    );
    let plan = |category: &str, asset: &str| {
        h.engine.plan(
            serde_json::from_value(json!({"family": category, "feature_slug": "f", "asset": asset, "brief": "x", "provider": "openai"})).unwrap(),
        )
    };
    assert_eq!(code(&plan("idea_spirit", "a").unwrap_err()), "family_forbidden_by_project");
    // every family is generated end to end, the counters and the division symbol too
    assert!(plan("counter_large", "a").is_ok());
    assert!(plan("division_emblem", "a").is_ok());
    // the advisor card needs the mod's frame template: refused before anything is spent, naming the paths it looked for
    let e = plan("advisor_card", "a").unwrap_err();
    assert_eq!(code(&e), "templates_missing");
    assert!(e.message.contains("advisor_template.png") && e.message.contains(".claude/skills/*/assets/*"), "{}", e.message);
    // portraits are generated now, with a likeness warning instead of a refusal
    let p = plan("leader_portrait", "a").unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("real people")), "{:?}", p.warnings);
    assert!(plan("focus", "a").unwrap().warnings.iter().all(|w| !w.contains("real people")));
    assert!(plan("focus", "a").is_ok());
    assert_eq!(code(&plan("gui_texture", "a").unwrap_err()), "invalid_argument", "gui_texture needs a mod-defined canvas");
    assert!(h
        .engine
        .plan(
            serde_json::from_value(
                json!({"family": "gui_texture", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai", "canvas": [120, 90]})
            )
            .unwrap()
        )
        .is_ok());
    assert_eq!(code(&plan("focus", "Bad Name").unwrap_err()), "invalid_argument");
    // a canvas cannot be overridden per request for a fixed-canvas category
    let e = h
        .engine
        .plan(
            serde_json::from_value(
                json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai", "canvas": [94, 86]}),
            )
            .unwrap(),
        )
        .unwrap_err();
    assert!(e.message.contains("fixed canvas"));
}

#[test]
fn vanilla_art_as_a_reference_needs_no_flag_and_its_origin_is_recorded() {
    let shelf = ".claude/skills/hoi4-feature-assets/assets/vanilla_reference/icons/national_focus/a.png";
    let plan_with_ref = |h: &Harness| {
        std::fs::create_dir_all(h.root().join(".claude/skills/hoi4-feature-assets/assets/vanilla_reference/icons/national_focus")).unwrap();
        std::fs::write(h.root().join(shelf), synthetic_icon_png()).unwrap();
        h.engine.plan(
            serde_json::from_value(json!({
                "family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai",
                "references": [{"path": shelf, "role": "style_anchor"}]
            }))
            .unwrap(),
        )
    };
    let plan = plan_with_ref(&openai_harness()).unwrap();
    assert_eq!(plan.references[0].origin, imagegen_core::refs::RefOrigin::KitShelf);
    assert!(!plan.approvals.iter().any(|a| a.contains("Paradox")), "no approval is needed for vanilla references");
    // the old key is read, ignored and reported as deprecated
    let on = harness(
        "[defaults]
provider = \"openai\"
[policy]
allow_vanilla_references = false
",
        &[("OPENAI_API_KEY", "sk-test-synthetic-000")],
    );
    let plan = plan_with_ref(&on).unwrap();
    assert_eq!(plan.references[0].origin, imagegen_core::refs::RefOrigin::KitShelf, "false no longer blocks anything");
    assert!(on.engine.status()["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("deprecated")));
}

#[test]
fn a_changed_reference_invalidates_the_plan_before_spending() {
    let h = openai_harness();
    std::fs::create_dir_all(h.root().join("references")).unwrap();
    std::fs::write(h.root().join("references/style.png"), synthetic_icon_png()).unwrap();
    let plan = h
        .engine
        .plan(serde_json::from_value(json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai", "references": [{"path": "references/style.png", "role": "style_anchor"}]})).unwrap())
        .unwrap();
    std::fs::write(h.root().join("references/style.png"), opaque_png()).unwrap();
    let e = h.engine.generate(serde_json::from_value(json!({"plan_id": plan.plan_id, "idempotency_key": "k-ref"})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "reference_changed");
    assert!(h.fixture.requests().is_empty());
}

#[test]
fn transparency_is_proven_from_pixels_never_assumed() {
    let h = openai_harness();
    h.queue_generation(&opaque_png());
    let plan = h.plan_focus("steel_gate");
    let cand = h.engine.generate(serde_json::from_value(json!({"plan_id": plan["plan_id"], "idempotency_key": "k-opaque"})).unwrap()).unwrap();
    assert_eq!(cand["candidates"][0]["alpha"]["has_transparency"], false);
    let id = cand["candidates"][0]["candidate_id"].as_str().unwrap();
    let e = h.engine.process(serde_json::from_value(json!({"candidate_id": id})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "no_transparency_in_source");

    // pre-steps may key a flat background, and the result is still validated
    let mut flat = Rgba::filled(928, 816, [255, 0, 255, 255]);
    for y in 200..600 {
        for x in 200..600 {
            flat.set_px(x, y, [90, 120, 60, 255]);
        }
    }
    h.queue_generation(&imagegen_core::pixels::encode_png(&flat).unwrap());
    let plan2 = h.plan_focus("keyed_gate");
    let g = h.engine.generate(serde_json::from_value(json!({"plan_id": plan2["plan_id"], "idempotency_key": "k-key"})).unwrap()).unwrap();
    let cid = g["candidates"][0]["candidate_id"].as_str().unwrap();
    let steps = json!([{"op": "chroma_key", "key": [255, 0, 255], "tolerance": 10}]);
    let p = h.engine.process(serde_json::from_value(json!({"candidate_id": cid, "pre_steps": steps})).unwrap()).unwrap();
    assert_eq!(p["quick_check"]["status"], "pass", "{:#}", p["quick_check"]);
    let ops: Vec<&str> = p["steps"].as_array().unwrap().iter().map(|s| s["op"]["op"].as_str().unwrap()).collect();
    assert_eq!(ops, vec!["chroma_key", "trim_to_subject", "fit_subject"], "the pre-step, then the family's own processing");
}

#[test]
fn fake_checkerboards_are_rejected_at_validation() {
    let h = openai_harness();
    // an "icon" that carries a drawn transparency checkerboard as its background (synthetic)
    let mut img = Rgba::new(100, 88);
    for y in 0..88u32 {
        for x in 0..100u32 {
            let v = if (x / 8 + y / 8) % 2 == 0 { 255 } else { 204 };
            img.set_px(x, y, [v, v, v, 255]);
        }
    }
    for y in 30..58 {
        for x in 30..70 {
            img.set_px(x, y, [160, 40, 40, 255]);
        }
    }
    std::fs::create_dir_all(h.root().join("incoming")).unwrap();
    std::fs::write(h.root().join("incoming/fake.png"), imagegen_core::pixels::encode_png(&img).unwrap()).unwrap();
    // it already has an accepted size, so the importer takes it as finished art; the checks then name the problem
    let out = h
        .engine
        .process(
            serde_json::from_value(json!({"import": {"path": "incoming/fake.png", "family": "focus", "feature_slug": "f", "asset": "fake"}}))
                .unwrap(),
        )
        .unwrap();
    let codes_of = |v: &Value| -> Vec<String> { v.as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap().to_string()).collect() };
    let quick = codes_of(&out["quick_check"]["findings"]);
    assert!(
        quick.contains(&"GFX-FAKE-CHECKERBOARD".to_string())
            && quick.contains(&"GFX-OPAQUE-CANVAS".to_string())
            && quick.contains(&"GFX-NO-ALPHA".to_string()),
        "{quick:?}"
    );
    let id = out["artifact_id"].as_str().unwrap().to_string();
    let v = h.validate(&id);
    assert_eq!(v["status"], "fail");
    // the failed validation blocks approval and install
    h.preview(&id);
    let answers: serde_json::Map<String, Value> =
        imagegen_core::rubric::questions("icon_v3").iter().map(|(k, _)| (k.to_string(), json!(true))).collect();
    let e = h
        .engine
        .review(serde_json::from_value(json!({"target": id, "verdict": "approved", "reviewer": "t", "answers": answers, "override_vanilla_check": true, "override_reason": "synthetic test art, no installed game"})).unwrap())
        .unwrap_err();
    assert_eq!(e.code, "review_needs_validation");
    assert_eq!(code(&h.install(&id, "apr-0000000000000000", json!({})).unwrap_err()), "install_gate_validation_failed");
    // validating a loose file with the same flaws gives the same findings
    let loose = h.engine.validate(serde_json::from_value(json!({"targets": ["incoming/fake.png"], "family": "focus"})).unwrap()).unwrap();
    assert!(codes_of(&loose["results"][0]["findings"]).contains(&"GFX-FAKE-CHECKERBOARD".to_string()));
}

#[test]
fn provider_failures_leave_a_clean_trail_and_uncertain_calls_block_blind_retries() {
    let h = openai_harness();
    h.fixture.respond(Method::Post, "/v1/images/generations", 429, &[], r#"{"error":{"message":"slow down"}}"#);
    let plan = h.plan_focus("steel_gate");
    let run = |key: &str| h.engine.generate(serde_json::from_value(json!({"plan_id": plan["plan_id"], "idempotency_key": key})).unwrap());
    let e = run("k-rl").unwrap_err();
    assert_eq!((code(&e), e.retry), ("provider_rate_limited", imagegen_core::RetryClass::SafeSameInputs));
    assert!(!h.exists("docs/assets/iron_pact/source_png/steel_gate-v1.png"), "a failed call leaves no candidate");
    let ended = h.engine.ledger.entries().unwrap().into_iter().rfind(|e| e.kind == "job_ended").unwrap();
    assert_eq!(ended.data["outcome"], "failed");
    // a failed (not uncertain) attempt may be retried with the same key
    h.fixture.clear_route(Method::Post, "/v1/images/generations");
    h.queue_generation(&synthetic_icon_png());
    assert!(run("k-rl").is_ok());

    // a connection that dies after the request left is uncertain: do not blindly pay twice
    let h2 = openai_harness();
    h2.fixture.fail(Method::Post, "/v1/images/generations", TransportError::MaybeSent("connection reset".into()));
    let plan2 = h2.plan_focus("steel_gate");
    let run2 = |key: &str| h2.engine.generate(serde_json::from_value(json!({"plan_id": plan2["plan_id"], "idempotency_key": key})).unwrap());
    let e = run2("k-un").unwrap_err();
    assert_eq!(e.retry, imagegen_core::RetryClass::ReconcileFirst);
    // the intent entry is closed as "uncertain", so the same key is allowed again only after a human decision:
    let ended = h2.engine.ledger.entries().unwrap().into_iter().rfind(|e| e.kind == "job_ended").unwrap();
    assert_eq!(ended.data["outcome"], "uncertain");

    // a crash between the intent and any result leaves an unfinished intent that blocks the same key
    let h3 = openai_harness();
    let plan3 = h3.plan_focus("steel_gate");
    h3.engine
        .ledger
        .append(
            "job_intent",
            Some("iron_pact.steel_gate"),
            None,
            json!({"job_id": "job-crashed", "idempotency_key": "k-crash", "asset_id": "iron_pact.steel_gate"}),
        )
        .unwrap();
    let e = h3.engine.generate(serde_json::from_value(json!({"plan_id": plan3["plan_id"], "idempotency_key": "k-crash"})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "uncertain_previous_attempt");
    assert!(h3.fixture.requests().is_empty());
    // a synchronous provider cannot reconcile; the report says so plainly
    let c = h3.engine.cancel(serde_json::from_value(json!({"job_id": "job-crashed", "reconcile": true})).unwrap()).unwrap();
    assert_eq!(c["status"], "unknown_outcome");
}

#[test]
fn a_missing_key_fails_before_anything_is_sent() {
    let h = harness("[defaults]\nprovider = \"openai\"\n", &[]);
    let plan = h.plan_focus("steel_gate");
    assert!(plan["approvals"].as_array().unwrap().iter().any(|a| a.as_str().unwrap().contains("OPENAI_API_KEY")));
    let e = h.engine.generate(serde_json::from_value(json!({"plan_id": plan["plan_id"], "idempotency_key": "k"})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "provider_not_configured");
    assert!(h.fixture.requests().is_empty());
}

#[test]
fn retired_models_are_refused_at_plan_time() {
    let h = openai_harness();
    let e = h
        .engine
        .plan(
            serde_json::from_value(
                json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai", "model": "gpt-image-1"}),
            )
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(code(&e), "provider_model_retired");
    assert!(e.message.contains("2026-10-23"));
}

#[test]
fn edits_create_a_child_candidate_that_keeps_the_original_plan() {
    let h = openai_harness();
    let (plan, cand, _processed) = h.make_processed("steel_gate", "k-edit-0");
    h.queue_edit(&synthetic_icon_png());
    let out = h
        .engine
        .edit(serde_json::from_value(json!({"candidate_id": cand, "instruction": "make the banner blue", "idempotency_key": "k-edit-1"})).unwrap())
        .unwrap();
    let child = &out["candidates"][0];
    assert_eq!(child["candidate_id"], "iron_pact.steel_gate-v2");
    assert_eq!(child["parent"], cand);
    assert_eq!(child["plan_id"], plan["plan_id"], "the edit inherits the plan so the install gate still checks it");
    let sent = h.fixture.requests().into_iter().find(|r| r.url.ends_with("/v1/images/edits")).unwrap();
    let body: Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(body["model"], "gpt-image-2.6-sunburst", "edits use the editing model");
    assert!(body["prompt"].as_str().unwrap().contains("make the banner blue"));
    assert_eq!(body["images"].as_array().unwrap().len(), 1);
    let lineage = h.engine.ledger.lineage("iron_pact.steel_gate-v2").unwrap();
    assert_eq!(lineage, vec!["iron_pact.steel_gate-v2", "iron_pact.steel_gate-v1"]);
    // an unreadable mask is refused before anything is sent
    let before = h.fixture.requests().len();
    std::fs::write(h.root().join("mask.png"), vec![0u8; 4]).unwrap();
    assert!(h
        .engine
        .edit(serde_json::from_value(json!({"candidate_id": cand, "instruction": "x", "mask": "mask.png", "idempotency_key": "k-edit-2"})).unwrap())
        .is_err());
    assert_eq!(h.fixture.requests().len(), before);
}

#[test]
fn finished_art_from_another_tool_can_be_imported_validated_and_installed() {
    let h = openai_harness();
    // a finished 65x67 advisor card with rounded transparent corners, made elsewhere (synthetic)
    let mut art = Rgba::new(65, 67);
    for y in 0..67 {
        for x in 0..65 {
            let corner = !(3..=61).contains(&x) && !(3..=63).contains(&y);
            art.set_px(x, y, if corner { [0, 0, 0, 0] } else { [(x * 3) as u8, (y * 3) as u8, 120, 255] });
        }
    }
    std::fs::create_dir_all(h.root().join("incoming")).unwrap();
    std::fs::write(h.root().join("incoming/advisor.dds"), dds::write_a8r8g8b8(&art)).unwrap();
    let p = h
        .engine
        .process(
            serde_json::from_value(
                json!({"import": {"path": "incoming/advisor.dds", "family": "advisor_card", "feature_slug": "iron_pact", "asset": "chief_advisor"}}),
            )
            .unwrap(),
        )
        .unwrap();
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let ops: Vec<&str> = p["steps"].as_array().unwrap().iter().map(|s| s["op"]["op"].as_str().unwrap()).collect();
    assert_eq!(ops, vec!["conform"], "finished art of the right size is accepted as it is, and the acceptance is recorded");
    assert_eq!(art.pixels_sha256(), p["pixels_sha256"], "no pixel changed");
    assert_eq!(h.validate(&id)["status"], "pass");
    h.preview(&id);
    let approval = h.approve(&id, "portrait_v3")["approval_id"].as_str().unwrap().to_string();
    let out = h.install(&id, &approval, json!({})).unwrap();
    let installed = dds::decode(&h.read("gfx/interface/ideas/iron_pact/advisors/idea_chief_advisor.dds")).unwrap();
    assert_eq!(installed.pixels_sha256(), art.pixels_sha256(), "imported art arrives pixel for pixel");
    assert_eq!(out["final_files"][0]["pixels_sha256"], art.pixels_sha256());
    // the original import source is untouched and the copy is under the ledger
    assert!(h.exists("incoming/advisor.dds") && h.exists("docs/assets/iron_pact/source_png/chief_advisor-v1.dds"));
}

#[test]
fn flag_sets_install_three_tga_sizes() {
    let h = openai_harness();
    let mut flag = Rgba::filled(164, 104, [200, 30, 30, 255]);
    for y in 0..104 {
        for x in 0..54 {
            flag.set_px(x, y, [250, 250, 250, 255]);
        }
    }
    std::fs::create_dir_all(h.root().join("incoming")).unwrap();
    std::fs::write(h.root().join("incoming/flag.png"), imagegen_core::pixels::encode_png(&flag).unwrap()).unwrap();
    let p = h
        .engine
        .process(
            serde_json::from_value(json!({"import": {"path": "incoming/flag.png", "family": "flag", "feature_slug": "iron_pact", "asset": "IRP"}}))
                .unwrap(),
        )
        .unwrap();
    let id = p["artifact_id"].as_str().unwrap().to_string();
    assert_eq!(p["size"], json!([82, 52]));
    assert_eq!(h.validate(&id)["status"], "pass");
    h.preview(&id);
    let approval = h.approve(&id, "flag_v3")["approval_id"].as_str().unwrap().to_string();
    h.install(&id, &approval, json!({})).unwrap();
    for (rel, w, hgt) in [("gfx/flags/IRP.tga", 82, 52), ("gfx/flags/medium/IRP.tga", 41, 26), ("gfx/flags/small/IRP.tga", 10, 7)] {
        let info = imagegen_core::tga::parse_header(&h.read(rel)).unwrap();
        assert_eq!((info.width, info.height, info.top_origin), (w, hgt, false), "{rel}");
    }
    assert!(!h.exists("docs/assets/iron_pact/gfx_handoff.md"), "flags have no sprite, so there is no handoff");
}

#[test]
fn register_mode_inserts_sprites_into_an_existing_gfx_file_and_keeps_the_rest() {
    let h = openai_harness();
    let (_p, _c, processed) = h.make_processed("steel_gate", "k-reg-1");
    std::fs::create_dir_all(h.root().join("interface")).unwrap();
    let existing = "# iron pact goals\nspriteTypes = {\n\t# keep this\n\tspriteType = {\n\t\tname = \"GFX_focus_older\"\n\t\ttexturefile = \"gfx/interface/goals/iron_pact/goal_older.dds\"\n\t}\n}\n";
    std::fs::write(h.root().join("interface/iron_pact_goals.gfx"), existing).unwrap();
    let approval = h.approve_all(&processed);
    let out = h.install(&processed, &approval, json!({"mode": "register"})).unwrap();
    let after = String::from_utf8(h.read("interface/iron_pact_goals.gfx")).unwrap();
    assert!(after.starts_with("# iron pact goals\nspriteTypes = {\n\t# keep this\n"));
    assert!(after.contains("GFX_focus_steel_gate_shine") && after.contains("GFX_focus_older"));
    assert!(out["replaced"].as_array().unwrap().iter().any(|r| r["path"] == "interface/iron_pact_goals.gfx"));
    assert!(!h.exists("docs/assets/iron_pact/gfx_handoff.md"));
    // registering the same sprite twice is refused before any file is written
    let (_p2, _c2, processed2) = h.make_processed("steel_gate", "k-reg-2");
    let approval2 = h.approve_all(&processed2);
    let e = h.install(&processed2, &approval2, json!({"mode": "register", "on_exists": "replace"})).unwrap_err();
    assert!(e.code == "already_exists" || e.message.contains("already defined"), "{e}");
    assert_eq!(String::from_utf8(h.read("interface/iron_pact_goals.gfx")).unwrap(), after);
}

#[test]
fn validating_a_folder_reports_every_file_and_flags_missing_provenance() {
    let h = openai_harness();
    std::fs::create_dir_all(h.root().join("gfx/interface/goals/old")).unwrap();
    let good = {
        let mut img = Rgba::new(100, 88);
        for y in 4..84 {
            for x in 6..94 {
                img.set_px(x, y, [100, 80, 60, 255]);
            }
        }
        img
    };
    std::fs::write(h.root().join("gfx/interface/goals/old/focus_a.dds"), dds::write_a8r8g8b8(&good)).unwrap();
    std::fs::write(h.root().join("gfx/interface/goals/old/focus_b.dds"), dds::write_a8r8g8b8(&Rgba::new(94, 86))).unwrap();
    std::fs::write(h.root().join("gfx/interface/goals/old/focus_c.dds"), b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 1\n")
        .unwrap();
    let v = h.engine.validate(serde_json::from_value(json!({"targets": ["gfx/interface/goals/old/"], "family": "focus"})).unwrap()).unwrap();
    assert_eq!(v["checked"], 3);
    assert_eq!(v["failed"], 2, "{v:#}");
    let a = &v["results"][0];
    assert_eq!(a["target"], "gfx/interface/goals/old/focus_a.dds");
    assert!(a["findings"].as_array().unwrap().iter().any(|f| f["code"] == "GFX-PROVENANCE-MISSING"));
    assert!(v["results"][2]["findings"].as_array().unwrap().iter().any(|f| f["code"] == "GFX-LFS-POINTER"));
    // validating a path without a profile is an error, not a guess
    assert!(h.engine.validate(serde_json::from_value(json!({"targets": ["gfx/interface/goals/old/focus_a.dds"]})).unwrap()).is_err());
}

#[test]
fn status_and_profiles_describe_the_project_without_leaking_keys() {
    let h = openai_harness();
    let s = h.engine.status();
    let text = s.to_string();
    assert!(!text.contains("sk-test-synthetic-000"), "the key value must never appear in status");
    let openai = s["providers"].as_array().unwrap().iter().find(|p| p["id"] == "openai").expect("openai listed").clone();
    assert_eq!(openai["configured"], true);
    assert_eq!(s["policy"]["allow_vanilla_references"]["configured"], Value::Null, "the deprecated key is unset");
    assert!(s["policy"]["allow_vanilla_references"]["effect"].as_str().unwrap().contains("ignored"));
    assert_eq!(s["chatgpt_login"]["compiled"], true);
    assert_eq!(s["chatgpt_login"]["signed_in"], false);
    let p = h.engine.families_info(serde_json::from_value(json!({"family": "focus"})).unwrap()).unwrap();
    assert_eq!(p["family"]["canvas"], json!([100, 88]));
    assert!(p["family"]["rubric_questions"].as_array().unwrap().len() >= 5);
    assert_eq!(p["family"]["example_final_files"][0], "gfx/interface/goals/example_feature/focus_example_asset.dds");
    let cal = h.engine.families_info(serde_json::from_value(json!({"calibrate": true})).unwrap()).unwrap();
    assert!(cal["calibration"]["available"].is_boolean(), "calibration reports availability honestly");
}

#[test]
fn reference_and_style_management() {
    let h = openai_harness();
    std::fs::create_dir_all(h.root().join("references")).unwrap();
    std::fs::write(h.root().join("references/a.png"), synthetic_icon_png()).unwrap();
    let call = |v: Value| h.engine.references(serde_json::from_value(v).unwrap());
    let added = call(json!({"action": "add", "path": "references/a.png", "role": "style_anchor", "note": "own work, free to use"})).unwrap();
    assert_eq!(added["recorded"], true);
    assert!(call(json!({"action": "add", "path": "references/a.png"})).is_err(), "a rights note is required");
    let listed = call(json!({"action": "list"})).unwrap();
    assert_eq!(listed["files"][0]["path"], "references/a.png");
    let saved = call(json!({"action": "style_save", "style": {
        "name": "iron_oil", "description": "aged oil", "prompt_fragments": ["visible brush strokes"], "references": [{"path": "references/a.png", "sha256": "", "note": "n"}], "approved_by": "me"
    }})).unwrap();
    assert_eq!(saved["style"]["references"][0]["sha256"], sha256_hex(&synthetic_icon_png()), "hashes are pinned for you");
    assert_eq!(call(json!({"action": "style_list"})).unwrap()["styles"], json!(["iron_oil"]));
    // a plan with the style includes its reference and fails if the file later changes
    let plan = h
        .engine
        .plan(
            serde_json::from_value(
                json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai", "style": "iron_oil"}),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(plan.references.len(), 1);
    assert!(plan.prompt.text.contains("visible brush strokes"));
    std::fs::write(h.root().join("references/a.png"), opaque_png()).unwrap();
    let e = h
        .engine
        .plan(
            serde_json::from_value(
                json!({"family": "focus", "feature_slug": "f", "asset": "b", "brief": "x", "provider": "openai", "style": "iron_oil"}),
            )
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(e.code, "style_reference_changed");
}

#[test]
fn the_ledger_chain_survives_a_whole_session_and_detects_tampering() {
    let h = openai_harness();
    let (_p, _c, processed) = h.make_processed("steel_gate", "k-chain");
    let approval = h.approve_all(&processed);
    h.install(&processed, &approval, json!({})).unwrap();
    let q = h.engine.ledger_query(serde_json::from_value(json!({"asset_id": "iron_pact.steel_gate", "limit": 100})).unwrap()).unwrap();
    assert_eq!(q["chain"]["ok"], true);
    let path = h.root().join(".hoi4-imagegen/ledger.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replacen("steel_gate", "steel_gat3", 1)).unwrap();
    let q = h.engine.ledger_query(serde_json::from_value(json!({})).unwrap()).unwrap();
    assert_eq!(q["chain"]["ok"], false);
}

// ---- 0.3.2: retiring an uncertain idempotency key ----

fn crashed_harness(key: &str) -> (Harness, serde_json::Value) {
    let h = openai_harness();
    let plan = h.plan_focus("steel_gate");
    h.engine
        .ledger
        .append(
            "job_intent",
            Some("iron_pact.steel_gate"),
            None,
            json!({"job_id": "job-crashed", "idempotency_key": key, "asset_id": "iron_pact.steel_gate"}),
        )
        .unwrap();
    (h, plan)
}

fn gen_with(h: &Harness, plan: &serde_json::Value, key: &str) -> Result<serde_json::Value, imagegen_mcp::EngineError> {
    h.engine.generate(serde_json::from_value(json!({"plan_id": plan["plan_id"], "idempotency_key": key})).unwrap())
}

#[test]
fn abandon_retires_an_uncertain_key_and_the_same_key_then_proceeds() {
    let (h, plan) = crashed_harness("k-killed");
    assert_eq!(code(&gen_with(&h, &plan, "k-killed").unwrap_err()), "uncertain_previous_attempt");
    // reconcile cannot clear it for a synchronous provider
    let r = h.engine.cancel(serde_json::from_value(json!({"idempotency_key": "k-killed", "reconcile": true})).unwrap()).unwrap();
    assert_eq!(r["status"], "unknown_outcome");
    assert!(r["note"].as_str().unwrap().contains("abandon"), "the note points to the way out: {r}");
    assert_eq!(code(&gen_with(&h, &plan, "k-killed").unwrap_err()), "uncertain_previous_attempt");

    let c = h.engine.cancel(serde_json::from_value(json!({"idempotency_key": "k-killed", "abandon": true})).unwrap()).unwrap();
    assert_eq!(c["status"], "abandoned", "{c}");
    assert_eq!(c["job_ids"], json!(["job-crashed"]));
    assert!(c["warning"].as_str().unwrap().contains("may have been charged"));
    // the ledger records it with the time and the warning
    let ended = h.engine.ledger.entries().unwrap().into_iter().rfind(|e| e.kind == "job_ended").unwrap();
    assert_eq!(ended.data["outcome"], "abandoned");
    assert_eq!(ended.data["job_id"], "job-crashed");
    assert_eq!(ended.data["idempotency_key"], "k-killed");
    assert!(ended.data["abandoned_at"].as_str().unwrap().starts_with("20"), "{}", ended.data);
    assert!(ended.data["warning"].as_str().unwrap().contains("may have been charged"));
    // no provider call was made by retiring the key
    assert!(h.fixture.requests().is_empty());

    // the same key now proceeds
    h.queue_generation(&synthetic_icon_png());
    let g = gen_with(&h, &plan, "k-killed").unwrap();
    assert!(g["candidates"][0]["candidate_id"].is_string(), "{g}");
    // and a second abandon of the now-completed key is refused
    let again = h.engine.cancel(serde_json::from_value(json!({"idempotency_key": "k-killed", "abandon": true})).unwrap()).unwrap_err();
    assert_eq!(code(&again), "attempt_completed");
}

#[test]
fn abandon_by_job_id_works_too() {
    let (h, plan) = crashed_harness("k-by-id");
    let c = h.engine.cancel(serde_json::from_value(json!({"job_id": "job-crashed", "abandon": true})).unwrap()).unwrap();
    assert_eq!(c["status"], "abandoned");
    assert_eq!(c["idempotency_key"], "k-by-id");
    h.queue_generation(&synthetic_icon_png());
    assert!(gen_with(&h, &plan, "k-by-id").is_ok());
}

#[test]
fn abandon_never_applies_to_a_completed_attempt() {
    let h = openai_harness();
    let plan = h.plan_focus("steel_gate");
    h.queue_generation(&synthetic_icon_png());
    let g = gen_with(&h, &plan, "k-done").unwrap();
    let cid = g["candidates"][0]["candidate_id"].as_str().unwrap().to_string();
    let e = h.engine.cancel(serde_json::from_value(json!({"idempotency_key": "k-done", "abandon": true})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "attempt_completed");
    assert!(e.message.contains(&cid), "{}", e.message);
    // the ledger gained no abandonment, and the key still replays the same candidate
    assert!(!h.engine.ledger.entries().unwrap().iter().any(|x| x.data["outcome"] == "abandoned"));
    let replay = gen_with(&h, &plan, "k-done").unwrap();
    assert_eq!(replay["candidates"][0]["candidate_id"], json!(cid));
    assert_eq!(replay["candidates"][0]["replayed"], true);
}

#[test]
fn abandon_refuses_a_job_recorded_as_completed_by_job_id() {
    let (h, _plan) = crashed_harness("k-rec");
    h.engine.ledger.append("job_ended", Some("iron_pact.steel_gate"), None, json!({"job_id": "job-crashed", "outcome": "recovered"})).unwrap();
    let e = h.engine.cancel(serde_json::from_value(json!({"job_id": "job-crashed", "abandon": true})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "attempt_completed");
}

#[test]
fn abandon_reports_when_there_is_nothing_to_retire() {
    let h = openai_harness();
    let plan = h.plan_focus("steel_gate");
    // unknown key
    let e = h.engine.cancel(serde_json::from_value(json!({"idempotency_key": "k-never", "abandon": true})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "not_found");
    // an attempt that failed cleanly is already closed
    h.fixture.fail(Method::Post, "/v1/images/generations", TransportError::NotSent("no network".into()));
    let _ = gen_with(&h, &plan, "k-failed");
    let c = h.engine.cancel(serde_json::from_value(json!({"idempotency_key": "k-failed", "abandon": true})).unwrap()).unwrap();
    assert_eq!(c["status"], "nothing_to_abandon", "{c}");
    // abandon cannot be combined with reconcile, and needs an id
    let e = h.engine.cancel(serde_json::from_value(json!({"idempotency_key": "k", "abandon": true, "reconcile": true})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "invalid_argument");
    let e = h.engine.cancel(serde_json::from_value(json!({"abandon": true})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "invalid_argument");
}

#[test]
fn a_second_crash_after_an_abandon_blocks_the_key_again() {
    // the retry reuses the job id, so the earlier end entry must not hide the new unfinished intent
    let (h, plan) = crashed_harness("k-twice");
    h.engine.cancel(serde_json::from_value(json!({"idempotency_key": "k-twice", "abandon": true})).unwrap()).unwrap();
    h.engine
        .ledger
        .append(
            "job_intent",
            Some("iron_pact.steel_gate"),
            None,
            json!({"job_id": "job-crashed", "idempotency_key": "k-twice", "asset_id": "iron_pact.steel_gate"}),
        )
        .unwrap();
    assert_eq!(code(&gen_with(&h, &plan, "k-twice").unwrap_err()), "uncertain_previous_attempt");
    let c = h.engine.cancel(serde_json::from_value(json!({"idempotency_key": "k-twice", "abandon": true})).unwrap()).unwrap();
    assert_eq!(c["status"], "abandoned");
}
