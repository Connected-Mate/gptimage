//! `model_reference`: the input picture of an image-to-3D tool (Meshy), one whole subject on a plain light grey
//! backdrop, planned with the caller's canvas and never installed.
//!
//! SYNTHETIC: the pictures are drawn by the test (a dark figure on grey), and nothing is generated: plans are free and
//! processing runs on imported files.

mod common;

use common::openai_harness;
use imagegen_core::pixels::{encode_png, Rgba};
use serde_json::{json, Value};

fn request(extra: Value) -> imagegen_core::plan::PlanRequest {
    let mut v = json!({
        "family": "model_reference", "feature_slug": "zin", "asset": "knight", "brief": "an armoured knight with a tower shield", "provider": "openai", "quality": "low"
    });
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    serde_json::from_value(v).unwrap()
}

/// A dark upright figure on a light grey backdrop; `edge` lets it run off the bottom of the picture.
fn figure(w: u32, h: u32, edge: bool) -> Rgba {
    let mut img = Rgba::filled(w, h, [200, 200, 200, 255]);
    let bottom = if edge { h } else { h * 9 / 10 };
    for y in h / 10..bottom {
        for x in w * 3 / 10..w * 7 / 10 {
            img.set_px(x, y, [60 + (x % 7) as u8 * 4, 55, 50 + (y % 5) as u8 * 3, 255]);
        }
    }
    img
}

fn import(h: &common::Harness, name: &str, img: &Rgba) -> Value {
    let rel = format!("art/{name}.png");
    let p = h.root().join(&rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, encode_png(img).unwrap()).unwrap();
    let params = json!({"import": {"path": rel, "family": "model_reference", "feature_slug": "zin", "asset": name}, "canvas": [512, 768]});
    h.engine.process(serde_json::from_value(params).unwrap()).unwrap()
}

#[test]
fn a_model_reference_is_planned_with_the_callers_canvas_and_no_interface_wording() {
    let h = openai_harness();
    let err = h.engine.plan(request(json!({}))).unwrap_err();
    assert!(err.message.contains("canvas"), "the caller sets the canvas: {err}");
    let plan = h.engine.plan(request(json!({"canvas": [1024, 1536]}))).unwrap();
    assert_eq!(plan.canvas, [1024, 1536]);
    let text = plan.prompt.text.to_lowercase();
    assert!(text.contains("shown whole") && text.contains("plain, flat light grey backdrop") && text.contains("no film grain"), "{text}");
    for wrong in ["interface material", "complete scene", "film grain, painterly", "transparent background"] {
        assert!(!text.contains(wrong), "the prompt must not ask for {wrong:?}: {text}");
    }
    assert!(plan.prompt.text.contains("an armoured knight with a tower shield"));
}

#[test]
fn the_plain_backdrop_check_passes_a_whole_subject_and_install_refuses() {
    let h = openai_harness();
    let ok = import(&h, "knight", &figure(512, 768, false));
    let id = ok["artifact_id"].as_str().unwrap().to_string();
    let v = h.validate(&id);
    assert_eq!(v["status"], "pass", "{v:#}");
    let cropped = import(&h, "cropped", &figure(512, 768, true));
    let v = h.validate(cropped["artifact_id"].as_str().unwrap());
    assert_eq!(v["status"], "fail");
    assert!(v.to_string().contains("GFX-MODEL-BACKDROP"), "a subject that runs off the edge is refused: {v:#}");
    let err = h.install(&id, "apr-0000000000000000", json!({})).unwrap_err();
    assert_eq!(err.code, "install_not_game_art");
    // the review asks the 3D-input questions, not vanilla's
    let fam = h.engine.families_info(serde_json::from_value(json!({"family": "model_reference"})).unwrap()).unwrap();
    let keys: Vec<&str> = fam["family"]["rubric_questions"].as_array().unwrap().iter().map(|q| q["key"].as_str().unwrap()).collect();
    assert!(keys.contains(&"whole_subject") && keys.contains(&"plain_backdrop") && !keys.contains(&"vanilla_style"), "{keys:?}");
}

/// The figure on real alpha: a clear surround and a subject at `subject_alpha` (a generated cut-out came back at 252).
fn cutout(w: u32, h: u32, edge: bool, subject_alpha: u8) -> Rgba {
    let mut img = figure(w, h, edge);
    for y in 0..h {
        for x in 0..w {
            let mut p = img.px(x, y);
            p[3] = if p == [200, 200, 200, 255] { 0 } else { subject_alpha };
            img.set_px(x, y, p);
        }
    }
    img
}

fn import_with(h: &common::Harness, name: &str, img: &Rgba, backdrop: &str) -> Result<Value, imagegen_mcp::EngineError> {
    let rel = format!("art/{name}.png");
    let p = h.root().join(&rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, encode_png(img).unwrap()).unwrap();
    let params = json!({"import": {"path": rel, "family": "model_reference", "feature_slug": "zin", "asset": name, "backdrop": backdrop}, "canvas": [512, 768]});
    h.engine.process(serde_json::from_value(params).unwrap())
}

#[test]
fn the_prompt_asks_for_a_front_facing_believable_faithful_subject_without_naming_a_kind_of_subject() {
    let h = openai_harness();
    let plan = h.engine.plan(request(json!({"canvas": [1024, 1536]}))).unwrap();
    let text = plan.prompt.text.to_lowercase();
    for want in [
        "straight from the front",
        "slight a-pose",
        "backdrop showing under each arm",
        "the legs a little apart",
        "the tips of anything held, worn or trailing included",
        "gripped in a closed hand",
        "believable anatomy",
        "neither a cartoon or toy",
        "uncanny cg",
        "not a human body given its traits",
        "never shrunk and never comically oversized",
        "invent nothing",
        "leave out effects, scenery",
    ] {
        assert!(text.contains(want), "the prompt must ask for {want:?}: {text}");
    }
    assert!(!text.contains("a three-quarter front view with"), "the old three-quarter rule is gone: {text}");
    // general: no creature or subject kinds in the family's own rules
    let rules = imagegen_core::builtin::profiles().into_iter().find(|p| p.family == imagegen_core::Family::ModelReference).unwrap();
    for word in ["dragon", "knight", "soldier", "horse", "tank", "demon", "angel"] {
        assert!(!rules.imagegen.prompt.rules.join(" ").to_lowercase().contains(word), "{word}");
    }
}

#[test]
fn a_transparent_backdrop_asks_for_alpha_and_carries_through_processing_and_validation() {
    let h = openai_harness();
    let plan = h.engine.plan(request(json!({"canvas": [1024, 1536], "backdrop": "transparent"}))).unwrap();
    assert!(plan.generation.transparent, "the provider is asked for real alpha");
    assert_eq!(plan.profile.backdrop, Some(imagegen_core::profile::Backdrop::Transparent));
    let text = plan.prompt.text.to_lowercase();
    assert!(text.contains("real transparent background (alpha) around the subject"), "{text}");
    assert!(!text.contains("light grey backdrop"), "{text}");
    // grey stays the default and leaves the plan as it was
    let grey = h.engine.plan(request(json!({"canvas": [1024, 1536], "backdrop": "grey"}))).unwrap();
    let plain = h.engine.plan(request(json!({"canvas": [1024, 1536]}))).unwrap();
    assert!(!grey.generation.transparent && grey.profile.backdrop.is_none() && grey.profile.hash == plain.profile.hash);
    // backdrop is only for model references, and transparent is not how a model reference asks
    let focus =
        json!({"family": "focus", "feature_slug": "zin", "asset": "wall", "brief": "a wall", "provider": "openai", "backdrop": "transparent"});
    assert!(h.engine.plan(serde_json::from_value(focus).unwrap()).unwrap_err().message.contains("backdrop is for model_reference"));
    assert!(h.engine.plan(request(json!({"canvas": [1024, 1536], "transparent": true}))).is_err());

    // a subject at alpha 252 is made solid and passes; the clear surround stays clear
    let ok = import_with(&h, "knight", &cutout(512, 768, false, 252), "transparent").unwrap();
    let steps: Vec<String> =
        ok["steps"].as_array().unwrap().iter().map(|s| s["op"]["op"].as_str().or(s["name"].as_str()).unwrap().to_string()).collect();
    assert_eq!(steps[0], "solid_alpha", "{ok:#}");
    let id = ok["artifact_id"].as_str().unwrap().to_string();
    let v = h.validate(&id);
    assert_eq!(v["status"], "pass", "{v:#}");
    assert!(v.to_string().contains("alpha_border") && !v.to_string().contains("plain_backdrop"), "{v:#}");
    let fam = h.engine.families_info(serde_json::from_value(json!({"family": "model_reference"})).unwrap()).unwrap();
    let keys: Vec<&str> = fam["family"]["rubric_questions"].as_array().unwrap().iter().map(|q| q["key"].as_str().unwrap()).collect();
    for k in ["pose_ready", "believable_anatomy", "feature_scale", "faithful_to_reference", "plain_backdrop"] {
        assert!(keys.contains(&k), "{k}: {keys:?}");
    }
    // a subject on alpha that runs off the edge fails the border check
    let cropped = import_with(&h, "cropped", &cutout(512, 768, true, 255), "transparent").unwrap();
    let v = h.validate(cropped["artifact_id"].as_str().unwrap());
    assert_eq!(v["status"], "fail");
    assert!(v.to_string().contains("GFX-MODEL-ALPHA"), "{v:#}");
    // a thin tip that reaches into the border ring stays under the error share, and is named in a warning
    let mut tip = cutout(512, 768, false, 255);
    for y in 3..80 {
        tip.set_px(150, y, [120, 90, 40, 255]);
    }
    let tipped = import_with(&h, "tipped", &tip, "transparent").unwrap();
    let v = h.validate(tipped["artifact_id"].as_str().unwrap());
    assert_eq!(v["status"], "pass", "a warning only: {v:#}");
    assert!(v.to_string().contains("within 3 px of the edge"), "{v:#}");
    // a grey fallback has no alpha to give
    let err = import_with(&h, "grey", &figure(512, 768, false), "transparent").unwrap_err();
    assert_eq!(err.code, "no_transparency_in_source", "{err}");
}
