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
