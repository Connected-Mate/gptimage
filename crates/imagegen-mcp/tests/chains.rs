//! L2: every family's processing runs by itself and cannot be skipped.
//!
//! SYNTHETIC: provider replies come from the fixture transport, and the templates are small images drawn by these
//! tests (an advisor frame with an enclosed transparent opening, achievement backgrounds and a red cross), not
//! Paradox files. The tests that need the kit's real templates, its face model or the installed game say so and
//! skip when they are missing.

mod common;

use common::*;
use imagegen_core::chain::{recipe_of, Recipe};
use imagegen_core::dds;
use imagegen_core::family::Family;
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba};
use imagegen_core::plan::PlanRequest;
use imagegen_core::records::{kind, ProcessedRecord};
use imagegen_mcp::api::GroupFollowUp;
use imagegen_mcp::EngineError;
use serde_json::{json, Value};

fn code(e: &EngineError) -> &str {
    &e.code
}

// ------------------------------------------------------------------ synthetic inputs

/// A synthetic dossier frame: an opaque ring, a transparent opening rotated by a few degrees and a transparent
/// exterior outside the card outline (like the real template, whose corners are transparent).
fn advisor_template() -> Rgba {
    let mut t = Rgba::new(65, 67);
    let (cx, cy) = (24.5f64, 30.5f64);
    let rad = (-3.0f64).to_radians();
    let (c, s) = (rad.cos(), rad.sin());
    for y in 0..67u32 {
        for x in 0..65u32 {
            if !((3..62).contains(&x) && (3..65).contains(&y)) {
                continue;
            }
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            let (lx, ly) = (dx * c + dy * s, -dx * s + dy * c);
            t.set_px(x, y, if lx.abs() < 16.0 && ly.abs() < 20.0 { [0, 0, 0, 0] } else { [120, 100, 60, 255] });
        }
    }
    t
}

fn achievement_background(shade: u8) -> Rgba {
    let mut t = Rgba::filled(64, 64, [shade, shade, shade, 254]);
    for i in 0..64 {
        t.set_px(i, 0, [shade + 60, shade + 60, shade + 60, 255]);
        t.set_px(0, i, [shade + 60, shade + 60, shade + 60, 255]);
    }
    t
}

fn red_cross() -> Rgba {
    let mut o = Rgba::new(64, 64);
    for i in 8..56u32 {
        for d in 0..3u32 {
            o.set_px(i + d, i, [200, 40, 40, 255]);
            o.set_px(63 - i - d, i, [200, 40, 40, 255]);
        }
    }
    o
}

const ASSETS: &str = ".claude/skills/hoi4-feature-assets/assets/test_reference";

fn put(h: &Harness, rel: &str, img: &Rgba) {
    let path = h.root().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, encode_png(img).unwrap()).unwrap();
}

fn write_templates(h: &Harness) {
    put(h, &format!("{ASSETS}/portraits/advisors/advisor_template.png"), &advisor_template());
    put(h, &format!("{ASSETS}/icons/achievements/template/achievement_template.png"), &achievement_background(40));
    put(h, &format!("{ASSETS}/icons/achievements/template/achievement_template_grey.png"), &achievement_background(70));
    put(h, &format!("{ASSETS}/icons/achievements/template/overlay.png"), &red_cross());
}

fn chain_harness() -> Harness {
    let h = openai_harness();
    write_templates(&h);
    h
}

/// An opaque 928x816 picture with structure, for the families made from a photograph.
fn photo_png() -> Vec<u8> {
    let mut img = Rgba::new(928, 816);
    for y in 0..816u32 {
        for x in 0..928u32 {
            let v = ((x / 40 + y / 40) % 2) as u8 * 60;
            img.set_px(x, y, [60 + v + (x % 90) as u8, 90 + v + (y % 70) as u8, 140 + (x % 40) as u8, 255]);
        }
    }
    encode_png(&img).unwrap()
}

fn rubric_of(family: &str) -> String {
    imagegen_core::builtin::rubric_of(family.parse().unwrap())
}

/// Plan, generate with the fixture reply `png`, and return the candidate id.
fn generate(h: &Harness, family: &str, asset: &str, png: &[u8], key: &str) -> String {
    h.queue_generation(png);
    let fam: Family = family.parse().unwrap();
    let req: PlanRequest = serde_json::from_value(json!({
        "family": family, "feature_slug": "iron_pact", "asset": asset, "brief": "a test subject", "provider": "openai", "quality": "low"
    }))
    .unwrap();
    let follow = GroupFollowUp { approve: true, idempotency_key: Some(key.into()), approve_cost: false };
    let out = h.engine.group_request(fam.group(), req, follow).unwrap();
    out["generation"]["candidates"][0]["candidate_id"].as_str().unwrap().to_string()
}

fn process(h: &Harness, cand: &str, extra: Value) -> Result<Value, EngineError> {
    let mut v = json!({"candidate_id": cand});
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    h.engine.process(serde_json::from_value(v).unwrap())
}

fn names(p: &Value, key: &str) -> Vec<String> {
    p[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.get("op").map(|o| o["op"].as_str().unwrap()).or_else(|| s["name"].as_str()).unwrap().to_string())
        .collect()
}

fn approve(h: &Harness, id: &str, family: &str) -> String {
    let v = h.validate(id);
    assert_eq!(v["status"], "pass", "{v:#}");
    h.preview(id);
    h.approve(id, &rubric_of(family))["approval_id"].as_str().unwrap().to_string()
}

fn processed_record(h: &Harness, id: &str) -> ProcessedRecord {
    let e = h.engine.ledger.entries().unwrap().into_iter().find(|e| e.kind == kind::PROCESSED && e.artifact_id.as_deref() == Some(id)).unwrap();
    serde_json::from_value(e.data).unwrap()
}

fn png_of(h: &Harness, rel: &str) -> Rgba {
    decode(&h.read(rel), None, DecodeLimits::default()).unwrap().0
}

// ------------------------------------------------------------------ (a) the finishing runs without being asked

#[test]
fn every_family_runs_its_own_finishing_without_being_asked() {
    let h = chain_harness();
    // (family, asset, picture the provider returns, extra process arguments, steps that must have run, finishers, companions, size)
    let subject = synthetic_icon_png();
    let photo = photo_png();
    type Row<'a> = (&'a str, &'a str, &'a [u8], Value, Vec<&'a str>, Vec<&'a str>, Vec<&'a str>, [u32; 2]);
    let rows: Vec<Row> = vec![
        ("focus", "wall", &subject, json!({}), vec!["trim_to_subject", "fit_subject"], vec![], vec![], [100, 88]),
        ("news_event", "gazette", &photo, json!({}), vec!["flatten_on", "cover_resize", "grayscale"], vec![], vec![], [397, 153]),
        ("report_event", "briefing", &photo, json!({}), vec!["flatten_on", "report_card"], vec![], vec![], [210, 176]),
        (
            "faction_logo",
            "pact",
            &subject,
            json!({}),
            vec!["fit_subject", "selected_frame"],
            vec!["derived_companions"],
            vec!["miniature"],
            [200, 100],
        ),
        ("agency_logo", "bureau", &subject, json!({}), vec!["fit_subject", "selected_frame"], vec![], vec![], [234, 119]),
        ("flag", "IRP", &photo, json!({}), vec!["flatten_on"], vec!["flag_ladder"], vec!["medium", "small"], [82, 52]),
        (
            "achievement",
            "hold_the_line",
            &subject,
            json!({}),
            vec!["fit_subject"],
            vec!["achievement_triplet"],
            vec!["_grey", "_not_eligible"],
            [64, 64],
        ),
        ("advisor_card", "chief_advisor", &photo, json!({}), vec!["flatten_on"], vec!["advisor_card"], vec![], [65, 67]),
        (
            "leader_portrait",
            "marshal",
            &photo,
            json!({"portrait_crop": [100, 50, 600, 700]}),
            vec!["flatten_on"],
            vec!["extract_portrait"],
            vec![],
            [156, 210],
        ),
    ];
    for (i, (family, asset, png, extra, steps, finishers, companions, size)) in rows.into_iter().enumerate() {
        let cand = generate(&h, family, asset, png, &format!("k-chain-{i}"));
        // no steps are passed: the family's processing is the tool's own
        let p = process(&h, &cand, extra).unwrap_or_else(|e| panic!("{family}: {e}"));
        let ran = names(&p, "steps");
        for s in &steps {
            assert!(ran.iter().any(|r| r == s), "{family}: step {s} did not run: {ran:?}");
        }
        assert_eq!(names(&p, "finishers"), finishers, "{family}");
        let keys: Vec<String> = p["companions"].as_array().unwrap().iter().map(|c| c["key"].as_str().unwrap().to_string()).collect();
        assert_eq!(keys, companions, "{family}");
        assert_eq!(p["size"], json!(size), "{family}");
        assert_eq!(p["chain"]["complete"], true, "{family}: {:#}", p["chain"]);
        let recipe = recipe_of(h.engine.profiles.for_family(family.parse().unwrap()).unwrap());
        assert_eq!(p["recipe"], json!({"id": recipe.id(), "version": recipe.version()}), "{family}");
        // the record carries what ran, and the full validation of the result passes
        let id = p["artifact_id"].as_str().unwrap();
        let rec = processed_record(&h, id);
        assert_eq!(rec.recipe.as_ref().unwrap().id, recipe.id());
        assert_eq!(rec.companions.len(), companions.len());
        let v = h.validate(id);
        assert_eq!(v["status"], "pass", "{family}: {v:#}");
    }
}

#[test]
fn a_pre_step_runs_before_the_family_finishing_and_is_recorded_first() {
    let h = chain_harness();
    let cand = generate(&h, "news_event", "gazette", &photo_png(), "k-pre-1");
    let p = process(&h, &cand, json!({"pre_steps": [{"op": "crop_to_aspect", "width": 1, "height": 1}]})).unwrap();
    assert_eq!(names(&p, "steps"), vec!["crop_to_aspect", "flatten_on", "cover_resize", "grayscale"]);
    assert_eq!(p["chain"]["complete"], true);
    // the old parameter name still works, and means the same thing
    let cand = generate(&h, "news_event", "bulletin", &photo_png(), "k-pre-2");
    let p = h.engine.process(serde_json::from_value(json!({"candidate_id": cand, "steps": [{"op": "trim_to_subject"}]})).unwrap());
    // the photo is opaque, so there is nothing to trim to: the pre-step runs and reports its own error rather than being skipped
    assert!(p.is_ok() || p.unwrap_err().message.contains("subject"));
}

// ------------------------------------------------------------------ (b) nothing can skip or replace the finishing

#[test]
fn an_empty_step_list_or_a_step_that_is_not_a_pre_step_is_refused_and_nothing_is_written() {
    let h = chain_harness();
    let cand = generate(&h, "focus", "wall", &synthetic_icon_png(), "k-refuse-1");
    let before = h.engine.ledger.entries().unwrap().len();

    let e = process(&h, &cand, json!({"steps": []})).unwrap_err();
    assert_eq!(code(&e), "chain_steps_empty");
    assert!(e.message.contains("cannot be skipped"), "{}", e.message);
    assert_eq!(code(&process(&h, &cand, json!({"pre_steps": []})).unwrap_err()), "chain_steps_empty");

    for bad in [
        json!({"op": "grayscale"}),
        json!({"op": "resize", "width": 100, "height": 88}),
        json!({"op": "fit_subject", "width": 100, "height": 88, "margin": 0}),
        json!({"op": "flatten_on", "background": [0, 0, 0]}),
        json!({"op": "cover_resize", "width": 100, "height": 88}),
        json!({"op": "conform", "width": 928, "height": 816}),
        json!({"op": "report_card", "seed": 1}),
        json!({"op": "selected_frame", "frames": 2}),
        json!({"op": "disabled_frame", "frames": 2}),
    ] {
        let name = bad["op"].as_str().unwrap().to_string();
        // an allowed pre-step in front of it does not make it allowed
        let e = process(&h, &cand, json!({"steps": [{"op": "trim_to_subject"}, bad]})).unwrap_err();
        assert_eq!(code(&e), "chain_step_not_allowed", "{name}");
        assert!(e.message.contains(&name) && e.message.contains("cannot be replaced or left out"), "{}", e.message);
    }
    assert_eq!(h.engine.ledger.entries().unwrap().len(), before, "a refused request leaves no processed record");
    assert!(!h.exists("docs/assets/iron_pact/processed_png/wall-v1-p1.png"));

    // the same holds for the other recipes: no family can be processed without its finishing
    let cand = generate(&h, "report_event", "briefing", &photo_png(), "k-refuse-2");
    assert_eq!(code(&process(&h, &cand, json!({"steps": []})).unwrap_err()), "chain_steps_empty");
    assert_eq!(
        code(&process(&h, &cand, json!({"steps": [{"op": "cover_resize", "width": 210, "height": 176}]})).unwrap_err()),
        "chain_step_not_allowed"
    );
    // and the allowed one still goes through, with the card made after it
    let p = process(&h, &cand, json!({"steps": [{"op": "crop_to_aspect", "width": 5, "height": 4}]})).unwrap();
    assert_eq!(names(&p, "steps"), vec!["crop_to_aspect", "flatten_on", "report_card"]);
}

// ------------------------------------------------------------------ (c) a record without its recipe fails validation and the gate

#[test]
fn a_processed_artifact_without_its_mandatory_recipe_fails_validation_and_the_install_gate() {
    let h = chain_harness();
    let cand = generate(&h, "focus", "wall", &synthetic_icon_png(), "k-forged-1");
    let p = process(&h, &cand, json!({})).unwrap();
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let genuine = processed_record(&h, &id);
    assert_eq!(h.validate(&id)["status"], "pass");

    // a record whose chain stops after the trim, as an older version or a hand-made entry would leave it
    let forge = |suffix: u32, edit: &dyn Fn(&mut ProcessedRecord)| -> String {
        let mut rec = genuine.clone();
        rec.artifact_id = format!("iron_pact.wall-v1-p{suffix}");
        rec.run = suffix;
        edit(&mut rec);
        h.engine.ledger.append(kind::PROCESSED, Some(&rec.asset_id), Some(&rec.artifact_id), &rec).unwrap();
        rec.artifact_id
    };
    let no_fit = forge(7, &|r| {
        r.steps.retain(|s| s.op.name() != "fit_subject");
        r.recipe.as_mut().unwrap().finishers.clear();
    });
    let v = h.validate(&no_fit);
    assert_eq!(v["status"], "fail", "{v:#}");
    let findings = v["results"][0]["findings"].as_array().unwrap();
    assert!(findings.iter().any(|f| f["code"] == "GFX-CHAIN-INCOMPLETE" && f["message"].as_str().unwrap().contains("fit_subject")), "{findings:?}");

    let no_recipe = forge(8, &|r| r.recipe = None);
    let v = h.validate(&no_recipe);
    assert_eq!(v["status"], "fail");
    assert!(v["results"][0]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["code"] == "GFX-CHAIN-INCOMPLETE" && f["message"].as_str().unwrap().contains("no recipe")));
    let wrong_version = forge(9, &|r| r.recipe.as_mut().unwrap().version = 99);
    assert_eq!(h.validate(&wrong_version)["status"], "fail");

    // a failing validation cannot be approved, and the gate names the chain even when an approval id is made up
    h.preview(&no_fit);
    let answers: serde_json::Map<String, Value> =
        imagegen_core::rubric::questions("icon_v1").iter().map(|(k, _)| (k.to_string(), json!(true))).collect();
    let e = h
        .engine
        .review(serde_json::from_value(json!({"target": no_fit, "verdict": "approved", "reviewer": "t", "answers": answers})).unwrap())
        .unwrap_err();
    assert_eq!(code(&e), "review_needs_validation");
    let e = h.install(&no_fit, "apr-0000000000000000", json!({})).unwrap_err();
    assert!(e.message.contains("install_gate_chain_incomplete") && e.message.contains("fit_subject"), "{}", e.message);
    // a raw candidate has no chain at all
    let e = h.install(&cand, "apr-0000000000000000", json!({})).unwrap_err();
    assert!(e.message.contains("install_gate_chain_incomplete") && e.message.contains("not a processed artifact"), "{}", e.message);
    assert!(!h.exists("gfx/interface/goals/iron_pact/focus_wall.dds"));

    // the genuine artifact installs
    let approval = approve(&h, &id, "focus");
    assert!(h.install(&id, &approval, json!({})).is_ok());
}

// ------------------------------------------------------------------ (d) achievement: three files, one approval

#[test]
fn an_achievement_installs_three_dds_files_and_one_approval_covers_all_of_them() {
    let h = chain_harness();
    let cand = generate(&h, "achievement", "hold_the_line", &synthetic_icon_png(), "k-ach-1");
    let p = process(&h, &cand, json!({})).unwrap();
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let rec = processed_record(&h, &id);
    assert_eq!(rec.companions.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), vec!["_grey", "_not_eligible"]);
    assert!(rec.companions.iter().all(|c| h.exists(&c.path) && c.size == [64, 64]));
    let detail = &rec.recipe.as_ref().unwrap().finishers[0].detail;
    assert_eq!(detail["grey_source"], "grayscale of the subject layer");
    assert!(detail["not_eligible_source"].as_str().unwrap().contains("overlay.png"));
    // the three states look as the kit says: grey is the subject in grey, not-eligible adds the red cross
    let main = png_of(&h, &rec.path);
    let grey = png_of(&h, &rec.companions[0].path);
    let not_eligible = png_of(&h, &rec.companions[1].path);
    assert_ne!(main, grey);
    assert_ne!(grey, not_eligible);
    assert!(not_eligible.px(20, 20)[0] > 150 && not_eligible.px(20, 20)[1] < 80, "red cross at (20,20): {:?}", not_eligible.px(20, 20));
    let centre = grey.px(32, 32);
    assert!(centre[0] == centre[1] && centre[1] == centre[2], "the grey state has no colour: {centre:?}");

    // validation covers every companion: a state that is not 64x64 fails it
    let approval = approve(&h, &id, "achievement");
    let grey_path = h.root().join(&rec.companions[0].path);
    let original = std::fs::read(&grey_path).unwrap();
    std::fs::write(&grey_path, encode_png(&Rgba::filled(60, 64, [9, 9, 9, 255])).unwrap()).unwrap();
    let v = h.validate(&id);
    assert_eq!(v["status"], "fail", "{v:#}");
    assert!(v["results"][0]["findings"].as_array().unwrap().iter().any(|f| f["message"].as_str().unwrap().starts_with("[_grey]")), "{v:#}");
    // and the approval of the main file does not cover a changed companion
    let e = h.install(&id, &approval, json!({})).unwrap_err();
    assert!(e.message.contains("install_gate_validation_stale") || e.message.contains("install_gate_hash_changed"), "{}", e.message);
    assert!(!h.exists("gfx/achievements/hold_the_line.dds"), "nothing is written when the gate is closed");
    std::fs::write(&grey_path, &original).unwrap();
    assert_eq!(h.validate(&id)["status"], "pass", "restored, the same approval applies again");

    let out = h.install(&id, &approval, json!({})).unwrap();
    let files: Vec<&str> = out["final_files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(
        files,
        vec!["gfx/achievements/hold_the_line.dds", "gfx/achievements/hold_the_line_grey.dds", "gfx/achievements/hold_the_line_not_eligible.dds"]
    );
    for (rel, want) in [(files[0], &main), (files[1], &grey), (files[2], &not_eligible)] {
        let bytes = h.read(rel);
        assert!(dds::canonical_problems(&bytes).is_empty(), "{rel}");
        assert_eq!(dds::decode(&bytes).unwrap().pixels_sha256(), want.pixels_sha256(), "{rel} decodes to the processed pixels");
    }
    assert!(!h.exists("docs/assets/iron_pact/gfx_handoff.md"), "achievements have no .gfx sprite");
    assert_eq!(out["sprite"]["names"], json!([]));
}

// ------------------------------------------------------------------ (e) flag: three TGA files, bottom-left origin

#[test]
fn a_flag_installs_all_three_tga_sizes_bottom_left_and_validation_covers_each_size() {
    let h = chain_harness();
    let cand = generate(&h, "flag", "IRP_fascism", &photo_png(), "k-flag-1");
    let p = process(&h, &cand, json!({})).unwrap();
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let rec = processed_record(&h, &id);
    assert_eq!(rec.companions.iter().map(|c| (c.key.as_str(), c.size)).collect::<Vec<_>>(), vec![("medium", [41, 26]), ("small", [10, 7])]);

    // validation checks the small size on its own: a 10x8 small flag is refused
    let small = h.root().join(&rec.companions[1].path);
    let original = std::fs::read(&small).unwrap();
    std::fs::write(&small, encode_png(&Rgba::filled(10, 8, [200, 0, 0, 255])).unwrap()).unwrap();
    let v = h.validate(&id);
    assert_eq!(v["status"], "fail", "{v:#}");
    assert!(v["results"][0]["findings"].as_array().unwrap().iter().any(|f| f["message"].as_str().unwrap().starts_with("[small]")));
    std::fs::write(&small, &original).unwrap();

    let approval = approve(&h, &id, "flag");
    let out = h.install(&id, &approval, json!({})).unwrap();
    assert_eq!(out["final_files"].as_array().unwrap().len(), 3);
    for (rel, w, hgt, from) in [
        ("gfx/flags/IRP_fascism.tga", 82, 52, rec.path.clone()),
        ("gfx/flags/medium/IRP_fascism.tga", 41, 26, rec.companions[0].path.clone()),
        ("gfx/flags/small/IRP_fascism.tga", 10, 7, rec.companions[1].path.clone()),
    ] {
        let bytes = h.read(rel);
        let info = imagegen_core::tga::parse_header(&bytes).unwrap();
        assert_eq!((info.width, info.height, info.top_origin), (w, hgt, false), "{rel}: bottom-left origin");
        let decoded = imagegen_core::tga::decode(&bytes).unwrap();
        assert_eq!(decoded.pixels_sha256(), png_of(&h, &from).pixels_sha256(), "{rel} is the recorded image of that size");
    }
}

// ------------------------------------------------------------------ (f) advisor card

#[test]
fn an_advisor_card_is_refused_when_the_template_is_missing_and_built_from_it_when_present() {
    // no template anywhere in the project: refused before spending, naming where it looked
    let h = openai_harness();
    let req: PlanRequest = serde_json::from_value(
        json!({"family": "advisor_card", "feature_slug": "iron_pact", "asset": "chief_advisor", "brief": "x", "provider": "openai"}),
    )
    .unwrap();
    let e = h.engine.plan(req).unwrap_err();
    assert_eq!(code(&e), "templates_missing");
    assert!(e.message.contains(".claude/skills/*/assets/*/portraits/advisors/advisor_template.png"), "{}", e.message);
    // an imported portrait that is not a card needs the template too, and the refusal names it again
    put(&h, "incoming/portrait.png", &Rgba::filled(300, 400, [90, 80, 70, 255]));
    let e = h
        .engine
        .process(
            serde_json::from_value(
                json!({"import": {"path": "incoming/portrait.png", "family": "advisor_card", "feature_slug": "iron_pact", "asset": "chief_advisor"}}),
            )
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(code(&e), "templates_missing");
    assert!(e.message.contains("advisor_template.png"));
    assert!(!h.engine.ledger.entries().unwrap().iter().any(|e| e.kind == kind::PROCESSED), "nothing was processed");

    // with the template: the plan asks for a tall portrait, and the card is built with a clean alpha audit
    let h = chain_harness();
    let req: PlanRequest = serde_json::from_value(
        json!({"family": "advisor_card", "feature_slug": "iron_pact", "asset": "chief_advisor", "brief": "x", "provider": "openai"}),
    )
    .unwrap();
    let plan = h.engine.plan(req).unwrap();
    assert_eq!(plan.source_canvas, Some([156, 210]), "the model draws a portrait, not a 65x67 picture");
    assert_eq!(plan.generation.aspect, "26:35");
    assert!(!plan.generation.transparent);
    assert!(plan.prompt.text.contains("head-and-shoulders"));
    let cand = generate(&h, "advisor_card", "chief_advisor", &photo_png(), "k-adv-1");
    let p = process(&h, &cand, json!({})).unwrap();
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let rec = processed_record(&h, &id);
    let detail = &rec.recipe.as_ref().unwrap().finishers[0].detail;
    assert_eq!(detail["alpha_coverage"], json!({"opening_gap": 0, "inner_edge_gap": 0, "exterior_leak": 0}));
    let card = png_of(&h, &rec.path);
    assert_eq!((card.width, card.height), (65, 67));
    assert_eq!([card.alpha(0, 0), card.alpha(64, 0), card.alpha(0, 66), card.alpha(64, 66)], [0, 0, 0, 0], "transparent corners");
    let approval = approve(&h, &id, "advisor_card");
    h.install(&id, &approval, json!({})).unwrap();
    let installed = dds::decode(&h.read("gfx/interface/ideas/iron_pact/advisors/idea_chief_advisor.dds")).unwrap();
    assert_eq!(installed.pixels_sha256(), card.pixels_sha256());
}

#[test]
fn the_advisor_alpha_audit_fails_closed_on_a_template_without_an_opening() {
    let h = openai_harness();
    // a frame with no enclosed transparent opening cannot take a portrait: the card is refused, not approximated
    put(&h, &format!("{ASSETS}/portraits/advisors/advisor_template.png"), &Rgba::filled(65, 67, [120, 100, 60, 255]));
    let cand = generate(&h, "advisor_card", "chief_advisor", &photo_png(), "k-adv-bad");
    let e = process(&h, &cand, json!({})).unwrap_err();
    assert!(code(&e).starts_with("advisor_"), "{e}");
    assert!(!h.engine.ledger.entries().unwrap().iter().any(|e| e.kind == kind::PROCESSED));
}

// ------------------------------------------------------------------ (g) report event

#[test]
fn report_event_output_has_transparent_corners_and_passes_validation() {
    let h = chain_harness();
    let cand = generate(&h, "report_event", "briefing", &photo_png(), "k-rep-1");
    let p = process(&h, &cand, json!({})).unwrap();
    let id = p["artifact_id"].as_str().unwrap().to_string();
    assert_eq!(p["recipe"], json!({"id": "report_card", "version": 2}));
    let card = png_of(&h, p["path"].as_str().unwrap());
    assert_eq!((card.width, card.height), (210, 176));
    assert_eq!([card.alpha(0, 0), card.alpha(209, 0), card.alpha(0, 175), card.alpha(209, 175)], [0, 0, 0, 0]);
    assert_eq!(card.alpha(105, 88), 255);
    assert!(card.has_transparency(), "card alpha is never flattened");
    // the seed is derived from the candidate: the same candidate always makes the same card
    let seed = names(&p, "steps").iter().position(|n| n == "report_card").unwrap();
    assert_eq!(
        p["steps"][seed]["op"]["seed"],
        imagegen_core::chain::seed_from_sha(
            h.engine.ledger.entries().unwrap().iter().find(|e| e.kind == kind::GENERATED).unwrap().data["sha256"].as_str().unwrap()
        )
    );
    let again = process(&h, &cand, json!({})).unwrap();
    assert_eq!(again["pixels_sha256"], p["pixels_sha256"]);

    let v = h.validate(&id);
    assert_eq!(v["status"], "pass", "{v:#}");
    let checks: Vec<&str> = v["results"][0]["checks"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    assert!(checks.contains(&"card_corners") && !checks.contains(&"opaque"), "{checks:?}");
    let approval = approve(&h, &id, "report_event");
    h.install(&id, &approval, json!({})).unwrap();
    let dds_card = dds::decode(&h.read("gfx/event_pictures/iron_pact/report_event_briefing.dds")).unwrap();
    assert_eq!(dds_card.alpha(0, 0), 0);
    assert_eq!(dds_card.pixels_sha256(), card.pixels_sha256());
}

#[test]
fn imported_report_art_becomes_a_card_unless_it_already_is_one() {
    let h = chain_harness();
    let import = |name: &str, asset: &str| {
        h.engine
            .process(
                serde_json::from_value(json!({"import": {"path": name, "family": "report_event", "feature_slug": "iron_pact", "asset": asset}}))
                    .unwrap(),
            )
            .unwrap()
    };
    // an opaque photo of the card's own size is not a card, so it gets the treatment
    put(&h, "incoming/flat.png", &Rgba::filled(210, 176, [90, 80, 70, 255]));
    let p = import("incoming/flat.png", "flat");
    assert!(names(&p, "steps").contains(&"report_card".to_string()));
    let flat = png_of(&h, p["path"].as_str().unwrap());
    assert_eq!(flat.alpha(0, 0), 0);
    // a photo of any other size does too
    put(&h, "incoming/big.png", &Rgba::filled(640, 480, [90, 80, 70, 255]));
    assert!(names(&import("incoming/big.png", "big"), "steps").contains(&"report_card".to_string()));
    // a finished card (210x176, fully transparent corners, card alpha) is accepted unchanged
    let finished = imagegen_core::recipes::report_card::report_card(&Rgba::filled(300, 200, [90, 80, 70, 255]), &Default::default()).unwrap();
    put(&h, "incoming/card.png", &finished);
    let p = import("incoming/card.png", "card");
    assert_eq!(names(&p, "steps"), vec!["conform"]);
    assert_eq!(p["pixels_sha256"], finished.pixels_sha256());
    assert_eq!(p["chain"]["complete"], true);
}

// ------------------------------------------------------------------ (h) installs are all or nothing

#[test]
fn a_failure_while_writing_leaves_no_file_behind_and_no_ledger_entry() {
    let h = chain_harness();
    let (_, _, id) = h.make_processed("steel_gate", "k-atomic-1");
    let approval = h.approve_all(&id);
    // `interface` is a file, so the .gfx file of register mode cannot be written
    std::fs::write(h.root().join("interface"), b"not a folder").unwrap();
    let e = h.install(&id, &approval, json!({"mode": "register"})).unwrap_err();
    assert!(matches!(code(&e), "install_write_failed" | "io_error" | "path_rejected"), "{e}");
    assert!(!h.exists("gfx/interface/goals/iron_pact/focus_steel_gate.dds"), "the DDS was not left behind");
    assert!(!h.exists("gfx"), "not even the folders");
    assert!(!h.engine.ledger.entries().unwrap().iter().any(|e| e.kind == kind::INSTALLED));
    fn leftovers(dir: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                leftovers(&p, out);
            } else if p.file_name().unwrap().to_string_lossy().contains(".tmp-") {
                out.push(p.display().to_string());
            }
        }
    }
    let mut found = Vec::new();
    leftovers(h.root(), &mut found);
    assert!(found.is_empty(), "temporary files left: {found:?}");
    // once the obstacle is gone the same approval installs everything
    std::fs::remove_file(h.root().join("interface")).unwrap();
    let out = h.install(&id, &approval, json!({"mode": "register"})).unwrap();
    assert!(h.exists("interface/iron_pact_goals.gfx") && h.exists("gfx/interface/goals/iron_pact/focus_steel_gate.dds"));
    // the shine companion is the recipe's vanilla-exact block, with the rotation offset sprite.rs used to omit
    let gfx = String::from_utf8(h.read("interface/iron_pact_goals.gfx")).unwrap();
    assert!(gfx.contains("GFX_focus_steel_gate_shine") && gfx.contains("animationrotationoffset = { x = 0.0 y = 0.0 }"), "{gfx}");
    assert_eq!(out["final_files"].as_array().unwrap().len(), 1);
}

// ------------------------------------------------------------------ strips from two frame files

#[test]
fn a_strip_family_can_be_imported_as_two_frame_files() {
    let h = chain_harness();
    let frame = |shade: u8| {
        let mut f = Rgba::new(76, 42);
        for y in 5..37 {
            for x in 8..68 {
                f.set_px(x, y, [shade, 90, 120, 255]);
            }
        }
        f
    };
    put(&h, "incoming/f1.png", &frame(40));
    put(&h, "incoming/f2.png", &frame(200));
    let p = h
        .engine
        .process(serde_json::from_value(json!({"import": {"frames": ["incoming/f1.png", "incoming/f2.png"], "family": "counter_large", "feature_slug": "iron_pact", "asset": "inf"}})).unwrap())
        .unwrap();
    assert_eq!(p["size"], json!([152, 42]));
    assert_eq!(names(&p, "steps"), vec!["conform"]);
    assert_eq!(p["recipe"]["id"], "imported_art");
    let strip = png_of(&h, p["path"].as_str().unwrap());
    let parts = strip.split_frames(2).unwrap();
    assert_eq!(parts[0], frame(40));
    assert_eq!(parts[1], frame(200));
    assert_eq!(h.validate(p["artifact_id"].as_str().unwrap())["status"], "pass");

    // the wrong number of files, a wrong frame size, a single-frame family and both inputs at once are all refused
    let refuse = |import: Value| h.engine.process(serde_json::from_value(json!({"import": import})).unwrap()).unwrap_err();
    let base = |frames: Value, family: &str| json!({"frames": frames, "family": family, "feature_slug": "iron_pact", "asset": "x"});
    assert!(refuse(base(json!(["incoming/f1.png"]), "counter_large")).message.contains("2 frames"));
    put(&h, "incoming/small.png", &Rgba::filled(30, 12, [1, 2, 3, 255]));
    assert!(refuse(base(json!(["incoming/small.png", "incoming/small.png"]), "counter_large")).message.contains("76x42"));
    assert!(refuse(base(json!(["incoming/f1.png", "incoming/f2.png"]), "focus")).message.contains("single image"));
    let mut both = base(json!(["incoming/f1.png", "incoming/f2.png"]), "counter_large");
    both["path"] = json!("incoming/f1.png");
    assert!(refuse(both).message.contains("exactly one"));
    // counters are not generated, so a counter plan is refused
    let req: PlanRequest =
        serde_json::from_value(json!({"family": "counter_large", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai"})).unwrap();
    assert_eq!(code(&h.engine.plan(req).unwrap_err()), "family_import_only");
}

#[test]
fn logos_and_buildings_build_their_own_second_frame() {
    let h = chain_harness();
    let cand = generate(&h, "faction_logo", "pact", &synthetic_icon_png(), "k-strip-1");
    let p = process(&h, &cand, json!({})).unwrap();
    let rec = processed_record(&h, p["artifact_id"].as_str().unwrap());
    let strip = png_of(&h, &rec.path);
    let frames = strip.split_frames(2).unwrap();
    // frame 1 keeps room around the emblem for the glow of frame 2, and frame 2 leaves the emblem itself alone
    let b = frames[0].alpha_bbox(16).unwrap();
    assert!(b[0] >= 10 && b[1] >= 10 && 100 - b[2] >= 10 && 100 - b[3] >= 10, "{b:?}");
    for y in 0..100 {
        for x in 0..100 {
            if frames[0].alpha(x, y) == 255 {
                assert_eq!(frames[1].px(x, y), frames[0].px(x, y));
            }
        }
    }
    assert!(frames[1].alpha_bbox(16).unwrap()[0] < b[0], "frame 2 reaches beyond the emblem");
    // a building's second frame is the ruined landmark: vanilla draws it as separate art, so it is a second
    // candidate of the same asset, and processing without it says how to make it
    let cand = generate(&h, "building", "castle", &synthetic_icon_png(), "k-strip-2");
    let e = process(&h, &cand, json!({})).unwrap_err();
    assert_eq!(code(&e), "second_frame_needed");
    assert!(e.message.contains("hoi4.image_edit"), "{}", e.message);
    let ruined_png = {
        let mut img = imagegen_core::pixels::decode(&synthetic_icon_png(), None, imagegen_core::pixels::DecodeLimits::default()).unwrap().0;
        for p in img.data.chunks_exact_mut(4) {
            p[0] /= 3;
        }
        imagegen_core::pixels::encode_png(&img).unwrap()
    };
    let ruined = generate(&h, "building", "castle", &ruined_png, "k-strip-3");
    assert_ne!(ruined, cand);
    let p = process(&h, &cand, json!({"second_frame_candidate": ruined})).unwrap();
    let strip = png_of(&h, p["path"].as_str().unwrap());
    let f = strip.split_frames(2).unwrap();
    assert_eq!(f[0].alpha_bbox(16), f[1].alpha_bbox(16), "both frames are fitted the same way");
    assert_ne!(f[0], f[1]);
    assert_eq!(h.validate(p["artifact_id"].as_str().unwrap())["status"], "pass");
    // another asset's candidate cannot be frame 2, and a single-frame family takes no second frame
    let other = generate(&h, "building", "tower", &ruined_png, "k-strip-4");
    assert!(process(&h, &cand, json!({"second_frame_candidate": other})).unwrap_err().message.contains("another raw candidate"));
    let focus = generate(&h, "focus", "wall", &synthetic_icon_png(), "k-strip-5");
    assert!(process(&h, &focus, json!({"second_frame_candidate": ruined})).is_err());
    // a finished strip is accepted at its size and the acceptance is recorded
    put(&h, "incoming/strip.png", &strip);
    let p = h
        .engine
        .process(
            serde_json::from_value(
                json!({"import": {"path": "incoming/strip.png", "family": "building", "feature_slug": "iron_pact", "asset": "keep"}}),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(names(&p, "steps"), vec!["conform"]);
    assert_eq!(p["chain"]["complete"], true);
    assert_eq!(recipe_of(h.engine.profiles.for_family(Family::Building).unwrap()), Recipe::LandmarkStrip);
}

// ------------------------------------------------------------------ portraits

#[test]
fn portraits_need_a_face_model_or_a_manual_crop_and_the_plan_warns_about_likeness() {
    let h = chain_harness();
    let req: PlanRequest = serde_json::from_value(
        json!({"family": "commander_portrait", "feature_slug": "iron_pact", "asset": "general", "brief": "x", "provider": "openai"}),
    )
    .unwrap();
    let plan = h.engine.plan(req).unwrap();
    assert!(plan.warnings.iter().any(|w| w.contains("real people")), "{:?}", plan.warnings);
    assert!(plan.warnings.iter().any(|w| w.contains("YuNet")), "no model in this project: {:?}", plan.warnings);
    let cand = generate(&h, "commander_portrait", "general", &photo_png(), "k-por-1");
    // no model and no crop: refused with both ways out named
    let e = process(&h, &cand, json!({})).unwrap_err();
    assert_eq!(code(&e), "face_model_missing");
    assert!(e.message.contains("face_detection_yunet_2026may.onnx") && e.message.contains("portrait_crop"), "{}", e.message);
    // a manual crop is the other route: no model needed
    let p = process(&h, &cand, json!({"portrait_crop": [100, 50, 600, 700]})).unwrap();
    assert_eq!(p["size"], json!([156, 210]));
    let img = png_of(&h, p["path"].as_str().unwrap());
    assert!(img.data.chunks_exact(4).all(|px| px[3] == 255), "portraits are opaque");
    assert_eq!(p["finishers"][0]["detail"]["choice"], json!({"manual": true}));
    assert_eq!(h.validate(p["artifact_id"].as_str().unwrap())["status"], "pass");
    // a crop outside the picture is refused, and the crop belongs to portraits only
    assert_eq!(code(&process(&h, &cand, json!({"portrait_crop": [0, 0, 5000, 5000]})).unwrap_err()), "portrait_crop_failed");
    let icon = generate(&h, "focus", "wall", &synthetic_icon_png(), "k-por-2");
    assert_eq!(code(&process(&h, &icon, json!({"portrait_crop": [0, 0, 10, 10]})).unwrap_err()), "invalid_argument");
    // the model is found inside the project, from the skill's tools folder, or from the config
    std::fs::create_dir_all(h.root().join(".claude/skills/hoi4-feature-assets/tools/models")).unwrap();
    std::fs::write(h.root().join(".claude/skills/hoi4-feature-assets/tools/models/face_detection_yunet_2026may.onnx"), b"not a model").unwrap();
    let e = process(&h, &cand, json!({})).unwrap_err();
    assert_ne!(code(&e), "face_model_missing", "the file was found: {e}");
}

// ------------------------------------------------------------------ the kit's real files (skip without them)

fn kit_dir() -> Option<std::path::PathBuf> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(v) = std::env::var("HOI4_KIT_SKILL") {
        candidates.push(v.into());
    }
    for up in ["../../..", "../../../.."] {
        candidates.push(manifest.join(up).join("agentic-hoi4-modding/.claude/skills/hoi4-feature-assets"));
    }
    candidates.into_iter().find(|p| p.join("assets/vanilla_reference/portraits/advisors/advisor_template.png").is_file())
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::copy(from, to).unwrap();
}

#[test]
fn the_kits_real_templates_make_a_clean_advisor_card_and_a_matching_achievement_triplet() {
    let Some(kit) = kit_dir() else {
        eprintln!("SKIPPED real kit templates: the agentic-hoi4-modding checkout was not found (set HOI4_KIT_SKILL)");
        return;
    };
    let h = openai_harness();
    let dest = h.root().join(".claude/skills/hoi4-feature-assets/assets/vanilla_reference");
    let src = kit.join("assets/vanilla_reference");
    for rel in [
        "portraits/advisors/advisor_template.png",
        "icons/achievements/template/achievement_template.png",
        "icons/achievements/template/achievement_template_grey.png",
        "icons/achievements/template/overlay.png",
    ] {
        copy_tree(&src.join(rel), &dest.join(rel));
    }
    // a real card: any portrait-shaped picture goes into the real frame, and the audit must be clean
    let cand = generate(&h, "advisor_card", "chief_advisor", &photo_png(), "k-real-adv");
    let p = process(&h, &cand, json!({})).unwrap();
    let rec = processed_record(&h, p["artifact_id"].as_str().unwrap());
    assert_eq!(
        rec.recipe.as_ref().unwrap().finishers[0].detail["alpha_coverage"],
        json!({"opening_gap": 0, "inner_edge_gap": 0, "exterior_leak": 0})
    );
    assert_eq!(h.validate(p["artifact_id"].as_str().unwrap())["status"], "pass");
    // the real achievement templates are the pinned ones, and the states validate
    let cand = generate(&h, "achievement", "hold_the_line", &synthetic_icon_png(), "k-real-ach");
    let p = process(&h, &cand, json!({})).unwrap();
    let rec = processed_record(&h, p["artifact_id"].as_str().unwrap());
    let pinned = &rec.recipe.as_ref().unwrap().finishers[0].detail["pinned_hashes_match"];
    assert_eq!(*pinned, json!({"completed": true, "grey": true, "overlay": true}));
    assert_eq!(h.validate(p["artifact_id"].as_str().unwrap())["status"], "pass");
    // and the files the real tool installs are three real DDS
    let approval = approve(&h, p["artifact_id"].as_str().unwrap(), "achievement");
    h.install(p["artifact_id"].as_str().unwrap(), &approval, json!({})).unwrap();
    for suffix in ["", "_grey", "_not_eligible"] {
        assert!(dds::canonical_problems(&h.read(&format!("gfx/achievements/hold_the_line{suffix}.dds"))).is_empty());
    }
}

#[test]
fn the_kits_face_model_finds_a_face_in_a_vanilla_portrait() {
    let Some(kit) = kit_dir() else {
        eprintln!("SKIPPED real face model: the agentic-hoi4-modding checkout was not found (set HOI4_KIT_SKILL)");
        return;
    };
    let model = kit.join("tools/models/face_detection_yunet_2026may.onnx");
    let portrait_dir = kit.join("assets/vanilla_reference/portraits/leaders");
    let Some(first) = std::fs::read_dir(&portrait_dir)
        .ok()
        .and_then(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "png").unwrap_or(false)).min())
    else {
        eprintln!("SKIPPED real face model: no vanilla leader portrait in the kit");
        return;
    };
    if !model.is_file() {
        eprintln!("SKIPPED real face model: {} is missing", model.display());
        return;
    }
    let h = openai_harness();
    copy_tree(&model, &h.root().join(".claude/skills/hoi4-feature-assets/tools/models/face_detection_yunet_2026may.onnx"));
    // a vanilla portrait enlarged to the size of a generated master (read at test time, never stored in the repository)
    let small = decode(&std::fs::read(&first).unwrap(), None, DecodeLimits::default()).unwrap().0;
    let master = imagegen_core::resize::resize_lanczos3(&small, small.width * 4, small.height * 4).unwrap();
    assert!(h.engine.face_model_available());
    let cand = generate(&h, "leader_portrait", "marshal", &encode_png(&master).unwrap(), "k-real-face");
    match process(&h, &cand, json!({})) {
        Ok(p) => {
            assert_eq!(p["size"], json!([156, 210]));
            let detail = &p["finishers"][0]["detail"];
            assert!(detail["choice"]["detected_face"].is_array(), "{detail:#}");
            assert!(detail["model"]["sha256"].is_string());
            assert_eq!(h.validate(p["artifact_id"].as_str().unwrap())["status"], "pass");
        }
        // painted portraits are not guaranteed to be found by a photo-trained detector: the refusal must then be the clear one
        Err(e) => {
            eprintln!("the detector did not find exactly one face in {}: {e}", first.display());
            assert_eq!(code(&e), "portrait_crop_failed");
            assert!(e.message.contains("portrait_crop"));
        }
    }
}

// ------------------------------------------------------------------ (f) the names and companions the game looks up

/// An imported 76x42 emblem or 30x12-frame counter, drawn as a filled shape.
fn shape(w: u32, h: u32, frames: u32, rgb: [u8; 3]) -> Rgba {
    let mut img = Rgba::new(w * frames, h);
    for f in 0..frames {
        for y in h / 6..h - h / 6 {
            for x in w / 4..w - w / 4 {
                img.set_px(f * w + x, y, [rgb[0], rgb[1], rgb[2], 255]);
            }
        }
    }
    img
}

fn import_and_install(h: &Harness, family: &str, asset: &str, img: &Rgba) -> Value {
    let rel = format!("incoming/{asset}.png");
    put(h, &rel, img);
    let p = h
        .engine
        .process(serde_json::from_value(json!({"import": {"path": rel, "family": family, "feature_slug": "iron_pact", "asset": asset}})).unwrap())
        .unwrap_or_else(|e| panic!("{family}: {e}"));
    assert_eq!(p["chain"]["complete"], true, "{family}: {:#}", p["chain"]);
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let approval = approve(h, &id, family);
    h.install(&id, &approval, json!({"mode": "register"})).unwrap_or_else(|e| panic!("{family}: {e}"))
}

#[test]
fn a_division_emblem_takes_the_next_free_symbol_number_with_its_small_symbol() {
    let h = chain_harness();
    // the mod already registers symbol 140 for something else
    std::fs::create_dir_all(h.root().join("interface")).unwrap();
    std::fs::write(
        h.root().join("interface/old_emblems.gfx"),
        "spriteTypes = {\n\tspriteType = { name = \"GFX_div_templ_140_large\" texturefile = \"gfx/x.dds\" }\n}\n",
    )
    .unwrap();
    let out = import_and_install(&h, "division_emblem", "wolf", &shape(76, 42, 1, [70, 110, 80]));
    assert_eq!(out["division_template_number"], 141, "one past the highest of vanilla (121) and the mod (140)");
    assert_eq!(out["sprite"]["names"], json!(["GFX_div_templ_141_large", "GFX_div_templ_141_small"]));
    let files: Vec<&str> = out["final_files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(
        files,
        vec![
            "gfx/interface/counters/division_templates_large/iron_pact/custom_wolf.dds",
            "gfx/interface/counters/division_templates_small/iron_pact/custom_wolf.dds"
        ]
    );
    let small = dds::decode(&h.read(files[1])).unwrap();
    assert_eq!((small.width, small.height), (30, 12));
    let gfx = String::from_utf8(h.read("interface/iron_pact_counters.gfx")).unwrap();
    assert!(gfx.contains("name = \"GFX_div_templ_141_small\"") && gfx.contains(files[1]), "{gfx}");
    assert!(out["script"].as_str().unwrap().contains("symbol 141"), "{}", out["script"]);
    // the number is in the asset record, and the next emblem takes the one after it
    let record: Value = serde_json::from_slice(&h.read(out["record_path"].as_str().unwrap())).unwrap();
    assert_eq!(record["last_install"]["division_template_number"], 141);
    let next = import_and_install(&h, "division_emblem", "bear", &shape(76, 42, 1, [120, 80, 60]));
    assert_eq!(next["division_template_number"], 142);
}

#[test]
fn an_on_map_counter_registers_the_vanilla_name_and_its_text_icon() {
    let h = chain_harness();
    let out = import_and_install(&h, "counter_small", "mountain_rangers", &shape(30, 12, 2, [190, 190, 190]));
    assert_eq!(out["sprite"]["names"], json!(["GFX_unit_mountain_rangers_icon_medium_white", "GFX_unit_mountain_rangers_icon_small"]));
    let gfx = String::from_utf8(h.read("interface/iron_pact_counters.gfx")).unwrap();
    assert_eq!(gfx.matches("noOfFrames = 2").count(), 2, "{gfx}");
    let icon = dds::decode(&h.read("gfx/texticons/iron_pact/unit_mountain_rangers_icon_small.dds")).unwrap();
    assert_eq!((icon.width, icon.height), (60, 12));
    let px = icon.px(15, 6);
    assert!(px[1] > px[0] && px[0] == px[2], "frame 1 of the text icon is tinted green: {px:?}");
    assert_eq!(icon.px(45, 6), [190, 190, 190, 255], "frame 2 is kept");
    assert!(out["script"].as_str().unwrap().contains("name the sub-unit mountain_rangers"));
    // the battalion icon is GFX_unit_<key>_icon_medium
    let out = import_and_install(&h, "counter_large", "mountain_rangers", &shape(76, 42, 2, [90, 120, 90]));
    assert_eq!(out["sprite"]["names"], json!(["GFX_unit_mountain_rangers_icon_medium"]));
}
