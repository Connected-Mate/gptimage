//! `hoi4.image_audit` and the pre-install reuse refusal, on a small synthetic mod.
//!
//! SYNTHETIC: the mod, its textures and the provider are written by the tests. Nothing here says what a real
//! mod looks like; the real-mod run is recorded separately (docs/testing.md).

mod common;

use common::*;
use imagegen_core::dds;
use imagegen_core::pixels::Rgba;
use serde_json::{json, Value};

fn write(h: &Harness, rel: &str, bytes: &[u8]) {
    let path = h.root().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn dds_of(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
    dds::write_a8r8g8b8(&Rgba::filled(w, h, px))
}

/// A mod where one sprite serves a focus and a decision, a report card has opaque corners, and a texture is missing.
fn synthetic_mod(h: &Harness) {
    write(
        h,
        "interface/mymod_goals.gfx",
        b"spriteTypes = {\n\tspriteType = { name = \"GFX_shared_icon\" texturefile = \"gfx/interface/goals/shared.dds\" }\n\tspriteType = { name = \"GFX_shared_icon_shine\" texturefile = \"gfx/interface/goals/shared.dds\" }\n\tspriteType = { name = \"GFX_report_card\" texturefile = \"gfx/event_pictures/card.dds\" }\n\tspriteType = { name = \"GFX_ghost\" texturefile = \"gfx/interface/goals/ghost.dds\" }\n}\n",
    );
    write(h, "gfx/interface/goals/shared.dds", &dds_of(100, 88, [200, 10, 10, 255]));
    write(h, "gfx/event_pictures/card.dds", &dds_of(210, 176, [10, 20, 30, 255]));
    write(
        h,
        "common/national_focus/mymod.txt",
        b"focus_tree = {\n\tid = mymod_tree\n\tfocus = {\n\t\tid = MYM_one\n\t\ticon = GFX_shared_icon\n\t}\n\tfocus = {\n\t\tid = MYM_two\n\t\ticon = GFX_ghost\n\t}\n}\n",
    );
    write(h, "common/decisions/mymod.txt", b"mymod_decisions = {\n\tMYM_decide = {\n\t\ticon = GFX_shared_icon\n\t}\n}\n");
    write(h, "events/mymod.txt", b"add_namespace = mym\ncountry_event = {\n\tid = mym.1\n\ttitle = mym.1.t\n\tpicture = GFX_report_card\n}\n");
}

fn count(v: &Value, code: &str) -> u64 {
    v["findings"][code]["count"].as_u64().unwrap_or_else(|| panic!("no group {code} in {v:#}"))
}

#[test]
fn audit_reports_reuse_with_every_use_and_the_checks_of_this_tool() {
    let h = openai_harness();
    synthetic_mod(&h);
    let v = h.engine.audit(serde_json::from_value(json!({})).unwrap()).unwrap();
    assert_eq!(v["schema"], "hoi4-imagegen-audit/1");
    // uses are counted per family
    assert_eq!(v["uses_by_family"]["focus"]["uses"], 2);
    assert_eq!(v["uses_by_family"]["decision"]["uses"], 1);
    assert_eq!(v["uses_by_family"]["report_event"]["uses"], 1);
    // reuse: a warning, with every use of the sprite
    assert_eq!(count(&v, "GFX-FAMILY-REUSE"), 1, "{v:#}");
    let reuse = &v["findings"]["GFX-FAMILY-REUSE"];
    assert_eq!(reuse["severity"], "warning");
    let f = &reuse["findings"][0];
    assert_eq!(f["subject"], "GFX_shared_icon");
    assert_eq!(f["families"], json!(["decision", "focus"]));
    assert_eq!(f["uses"].as_array().unwrap().len(), 2);
    assert!(f["uses"].as_array().unwrap().iter().any(|u| u["path"] == "common/decisions/mymod.txt" && u["family"] == "decision"));
    // the decision's 100x88 texture is not a decision size; the focus's is fine
    assert_eq!(count(&v, "GFX-CANVAS-SIZE"), 1, "{v:#}");
    let c = &v["findings"]["GFX-CANVAS-SIZE"]["findings"][0];
    assert_eq!(c["families"], json!(["decision"]));
    assert_eq!(c["evidence"]["size"], json!([100, 88]));
    // the report card is opaque in the corners: the one check that decodes pixels
    assert_eq!(count(&v, "GFX-CARD-CORNERS"), 1);
    assert_eq!(v["alpha_checked"], 1);
    assert_eq!(v["findings"]["GFX-CARD-CORNERS"]["findings"][0]["evidence"]["corner_alpha"], json!([255, 255, 255, 255]));
    // a sprite whose texture file does not exist
    assert_eq!(count(&v, "GFX-TEXTURE-MISSING"), 1, "{v:#}");
    // the focus sprites have a shine companion only for the shared one
    assert_eq!(count(&v, "GFX-FOCUS-SHINE-MISSING"), 1);
    assert_eq!(v["findings"]["GFX-FOCUS-SHINE-MISSING"]["findings"][0]["subject"], "GFX_ghost");
    // nothing was written
    assert!(!h.exists(".hoi4-imagegen/audit-allowlist.json"));
}

#[test]
fn the_allowlist_silences_intentional_aliases_and_filters_work() {
    let h = openai_harness();
    synthetic_mod(&h);
    write(
        &h,
        ".hoi4-imagegen/audit-allowlist.json",
        br#"{"aliases":[{"sprite":"GFX_shared_icon","families":["focus","decision"],"reason":"same art on purpose"}],"no_shine":["GFX_ghost"]}"#,
    );
    // the default allowlist is found without being named
    let v = h.engine.audit(serde_json::from_value(json!({})).unwrap()).unwrap();
    assert_eq!(count(&v, "GFX-FAMILY-REUSE"), 0, "{v:#}");
    assert_eq!(count(&v, "GFX-FOCUS-SHINE-MISSING"), 0);
    assert_eq!(v["allowlist"]["aliases"], 1);
    // a family filter keeps only that family's uses and findings
    let v = h.engine.audit(serde_json::from_value(json!({"families": ["report_event"]})).unwrap()).unwrap();
    assert_eq!(v["uses_by_family"].as_object().unwrap().len(), 1);
    assert_eq!(count(&v, "GFX-CARD-CORNERS"), 1);
    assert_eq!(count(&v, "GFX-CANVAS-SIZE"), 0);
    // a named allowlist that does not exist is an error, and so is a bad one
    assert!(h.engine.audit(serde_json::from_value(json!({"allowlist": "nope.json"})).unwrap()).is_err());
    write(&h, "bad.json", br#"{"aliases":[{"sprite":"GFX_a","families":["not_a_family"]}]}"#);
    assert!(h.engine.audit(serde_json::from_value(json!({"allowlist": "bad.json"})).unwrap()).is_err());
    // a path outside the project is refused
    assert!(h.engine.audit(serde_json::from_value(json!({"allowlist": "../x.json"})).unwrap()).is_err());
}

#[test]
fn limit_cuts_the_listing_not_the_counts() {
    let h = openai_harness();
    synthetic_mod(&h);
    let v = h.engine.audit(serde_json::from_value(json!({"limit": 1})).unwrap()).unwrap();
    assert_eq!(v["findings"]["GFX-FAMILY-REUSE"]["shown"], 1);
    assert!(h.engine.audit(serde_json::from_value(json!({"limit": 0})).unwrap()).is_err());
    let all = h.engine.audit(serde_json::from_value(json!({"limit": 1000})).unwrap()).unwrap();
    assert_eq!(all["truncated"], false);
}

#[test]
fn install_refuses_a_sprite_the_mod_already_uses_for_another_family() {
    let h = openai_harness();
    let (_p, _c, processed) = h.make_processed("steel_gate", "k-audit-1");
    // the mod already uses the sprite name this focus icon would get, in a decision
    write(&h, "common/decisions/mine.txt", b"mine = {\n\tMINE_a = {\n\t\ticon = GFX_focus_steel_gate\n\t}\n}\n");
    let approval = h.approve_all(&processed);
    let e = h.install(&processed, &approval, json!({"mode": "handoff"})).unwrap_err();
    assert_eq!(e.code, "GFX-FAMILY-REUSE", "{e}");
    assert!(e.message.contains("decision") && e.message.contains("common/decisions/mine.txt:3"), "{e}");
    assert!(!h.exists("gfx/interface/goals/iron_pact/goal_steel_gate.dds"), "nothing was written");
    // an allowlist alias for exactly this sprite lets it through
    write(
        &h,
        ".hoi4-imagegen/audit-allowlist.json",
        br#"{"aliases":[{"sprite":"GFX_focus_steel_gate","families":["decision","focus"],"reason":"test"}]}"#,
    );
    let ok = h.install(&processed, &approval, json!({"mode": "handoff"})).unwrap();
    assert_eq!(ok["reuse_trace"], "checked");
}

#[test]
fn install_of_the_same_family_is_not_reuse() {
    let h = openai_harness();
    let (_p, _c, processed) = h.make_processed("steel_gate", "k-audit-2");
    write(&h, "common/national_focus/mine.txt", b"focus_tree = {\n\tid = t\n\tfocus = {\n\t\tid = A\n\t\ticon = GFX_focus_steel_gate\n\t}\n}\n");
    let approval = h.approve_all(&processed);
    let ok = h.install(&processed, &approval, json!({"mode": "handoff"})).unwrap();
    assert_eq!(ok["reuse_trace"], "checked");
}
