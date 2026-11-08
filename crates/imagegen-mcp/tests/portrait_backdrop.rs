//! The review refuses a portrait on another family's backdrop (`review_portrait_backdrop`).
//!
//! The measured vanilla-match check cannot refuse a dark backdrop for leaders, because vanilla has a few special dark
//! ones, and the validation's backdrop check only warns. The review blocks it unless the art is a deliberate, recorded
//! exception.
//!
//! SYNTHETIC: the portraits are drawn by the test (a dark figure on a flat backdrop) and imported at vanilla's 156x210.

mod common;

use common::{openai_harness, Harness};
use imagegen_core::pixels::{encode_png, Rgba};
use serde_json::{json, Value};

/// A head-and-shoulders silhouette on a flat backdrop of `rgb`.
fn portrait(rgb: [u8; 3]) -> Rgba {
    let mut img = Rgba::filled(156, 210, [rgb[0], rgb[1], rgb[2], 255]);
    for y in 40..210u32 {
        for x in 0..156u32 {
            let head = (f64::from(x) - 78.0).hypot(f64::from(y) - 80.0) < 32.0;
            let shoulders = y > 120 && (f64::from(x) - 78.0).abs() < 20.0 + f64::from(y - 120) * 0.7;
            if head || shoulders {
                img.set_px(x, y, [70 + (x % 9) as u8, 60, 55 + (y % 7) as u8, 255]);
            }
        }
    }
    img
}

fn import(h: &Harness, name: &str, img: &Rgba) -> String {
    let rel = format!("art/{name}.png");
    let p = h.root().join(&rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, encode_png(img).unwrap()).unwrap();
    let params = json!({"import": {"path": rel, "family": "leader_portrait", "feature_slug": "council", "asset": name}});
    let v = h.engine.process(serde_json::from_value(params).unwrap()).unwrap();
    v["artifact_id"].as_str().unwrap().to_string()
}

fn approve(h: &Harness, id: &str, overridden: bool) -> Result<Value, imagegen_mcp::EngineError> {
    let answers: serde_json::Map<String, Value> =
        imagegen_core::rubric::questions("portrait_v3").iter().map(|(k, _)| (k.to_string(), json!(true))).collect();
    let mut p = json!({"target": id, "verdict": "approved", "reviewer": "t", "answers": answers});
    if overridden {
        p["override_vanilla_check"] = json!(true);
        p["override_reason"] = json!("a regency council portrait, dark backdrop on purpose");
    }
    h.engine.review(serde_json::from_value(p).unwrap())
}

#[test]
fn a_dark_leader_backdrop_blocks_the_approval_unless_it_is_a_recorded_exception() {
    let h = openai_harness();
    let dark = import(&h, "dark", &portrait([30, 28, 26]));
    let v = h.validate(&dark);
    assert_eq!(v["status"], "pass", "validation only warns: {v:#}");
    assert!(v.to_string().contains("GFX-PORTRAIT-BACKDROP"));
    h.preview(&dark);
    let e = approve(&h, &dark, false).unwrap_err();
    assert_eq!(e.code, "review_portrait_backdrop", "{e}");
    assert!(e.details.unwrap()["backdrop"][0]["code"] == "GFX-PORTRAIT-BACKDROP");
    // a deliberate exception passes with the recorded override and its reason
    let ok = approve(&h, &dark, true).unwrap();
    assert!(ok["approval_id"].is_string(), "{ok:#}");

    // vanilla's pale cream backdrop gets past the backdrop gate (here the next refusal is the vanilla preview, as the
    // harness has no installed game)
    let cream = import(&h, "cream", &portrait([229, 226, 209]));
    assert_eq!(h.validate(&cream)["status"], "pass");
    h.preview(&cream);
    assert_ne!(approve(&h, &cream, false).unwrap_err().code, "review_portrait_backdrop");
}
