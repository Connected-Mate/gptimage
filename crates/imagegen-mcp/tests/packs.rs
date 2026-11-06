//! Style packs: a mod's house style across families, applied to plans, processing, validation and the review.
//!
//! SYNTHETIC: every image here is drawn by the tests (flat discs, bars, strips and gradients), and nothing is
//! generated: plans are free, and processing runs on imported synthetic files. No game file and no file of any mod is
//! read or copied.

mod common;

use std::path::Path;

use common::{harness, openai_harness, repo_work_dir, Harness};
use imagegen_core::family::Family;
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba};
use imagegen_core::profile::ProfileSet;
use imagegen_core::rubric;
use serde_json::{json, Value};

const TEAL: [u8; 4] = [76, 198, 192, 255];

fn write(h: &Harness, rel: &str, img: &Rgba) {
    let p = h.root().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, encode_png(img).unwrap()).unwrap();
}

fn disc(w: u32, h: u32, r: f64, colour: [u8; 4]) -> Rgba {
    let mut img = Rgba::new(w, h);
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (f64::from(x) + 0.5 - cx, f64::from(y) + 0.5 - cy);
            if dx * dx + dy * dy < r * r {
                img.set_px(x, y, colour);
            }
        }
    }
    img
}

/// A flat teal ring with a bar across it, the terminal pictogram style, drawn at `size`.
fn pictogram(size: u32, colour: [u8; 4]) -> Rgba {
    let mut img = Rgba::new(size, size);
    let c = f64::from(size) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (f64::from(x) + 0.5 - c, f64::from(y) + 0.5 - c);
            let r = (dx * dx + dy * dy).sqrt() / f64::from(size);
            if (r > 0.30 && r < 0.42) || (dy.abs() < 0.04 * f64::from(size) && dx.abs() < 0.30 * f64::from(size)) {
                img.set_px(x, y, colour);
            }
        }
    }
    img
}

fn gradient(w: u32, h: u32) -> Rgba {
    let mut img = Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let v = ((x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8);
            img.set_px(x, y, [v.0, v.1, 90, 255]);
        }
    }
    img
}

fn refs(h: &Harness, folder: &str, n: u32) -> Vec<Value> {
    (0..n)
        .map(|i| {
            let rel = format!("{folder}/ref_{i}.png");
            let shade = (i * 6) as u8;
            write(h, &rel, &disc(40, 40, 14.0 + f64::from(i), [76 + shade, 198 - shade, 192, 255]));
            json!({"path": rel, "sha256": "", "note": "synthetic test reference"})
        })
        .collect()
}

fn references(h: &Harness, params: Value) -> Result<Value, imagegen_mcp::EngineError> {
    h.engine.references(serde_json::from_value(params).unwrap())
}

fn save_pack(h: &Harness, pack: Value) -> Value {
    references(h, json!({"action": "pack_save", "pack": pack, "replace": true})).unwrap()
}

fn request(family: &str, extra: Value) -> imagegen_core::plan::PlanRequest {
    let mut v =
        json!({"family": family, "feature_slug": "house", "asset": "test_asset", "brief": "a test subject", "provider": "openai", "quality": "low"});
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    serde_json::from_value(v).unwrap()
}

fn import(h: &Harness, family: &str, img: &Rgba, extra: Value) -> Result<Value, imagegen_mcp::EngineError> {
    let rel = format!("art/{family}_{}.png", img.pixels_sha256().get(..8).unwrap());
    write(h, &rel, img);
    let mut spec = json!({"path": rel, "family": family, "feature_slug": "house", "asset": "test_asset"});
    let mut params = json!({});
    for (k, v) in extra.as_object().unwrap() {
        if k == "style_pack" {
            spec[k] = v.clone();
        } else {
            params[k] = v.clone();
        }
    }
    params["import"] = spec;
    h.engine.process(serde_json::from_value(params).unwrap())
}

fn pixels(h: &Harness, rel: &str) -> Rgba {
    decode(&h.read(rel), Some(rel), DecodeLimits::default()).unwrap().0
}

fn validated(h: &Harness, id: &str) -> Value {
    h.validate(id)["results"][0].clone()
}

fn decision_pack(h: &Harness) -> Value {
    json!({
        "name": "house",
        "description": "a teal terminal house style",
        "families": {
            "decision": {
                "look": "flat teal pictogram on transparent, crisp pixel edges",
                "form": ["A flat {w}x{h} pictogram drawn in thin teal lines, with no frame and no shading."],
                "negative": ["painted texture", "a frame"],
                "references": refs(h, "art/decision_refs", 5)
            }
        }
    })
}

// ------------------------------------------------------------------ the pack object

#[test]
fn a_pack_is_saved_pinned_listed_read_and_validated() {
    let h = openai_harness();
    let saved = save_pack(&h, decision_pack(&h));
    assert_eq!(saved["saved"], ".hoi4-imagegen/packs/house/pack.toml");
    let pinned = saved["pack"]["families"]["decision"]["references"][0]["sha256"].as_str().unwrap();
    assert_eq!(pinned.len(), 64, "the hash is filled in on save");
    let text = String::from_utf8(h.read(".hoi4-imagegen/packs/house/pack.toml")).unwrap();
    assert!(text.contains("[families.decision]") && text.contains("look = \"flat teal pictogram"), "{text}");

    assert_eq!(references(&h, json!({"action": "pack_list"})).unwrap()["packs"], json!(["house"]));
    let got = references(&h, json!({"action": "pack_get", "name": "house"})).unwrap();
    assert_eq!(got["pack"]["families"]["decision"]["look"], "flat teal pictogram on transparent, crisp pixel edges");
    let check = references(&h, json!({"action": "pack_validate", "name": "house"})).unwrap();
    assert_eq!(check["ok"], true, "{check}");
    let set = &check["families"]["decision"]["sets"][0];
    assert_eq!(set["references"], 5);
    assert_eq!(set["self_check"]["of"], 5, "{set}");
    assert!(h.engine.ledger.entries().unwrap().iter().any(|e| e.kind == "pack_saved"));

    // a switch that does not apply to the family, and a reference outside the project, are refused
    let mut bad = decision_pack(&h);
    bad["families"]["decision"]["processing"] = json!({"keep_colours": true});
    assert!(references(&h, json!({"action": "pack_save", "pack": bad, "replace": true}))
        .unwrap_err()
        .message
        .contains("keep_colours does not apply"));
    let mut bad = decision_pack(&h);
    bad["families"]["decision"]["references"][0]["path"] = json!("../outside.png");
    assert!(references(&h, json!({"action": "pack_save", "pack": bad, "replace": true})).is_err());
}

#[test]
fn a_changed_pack_reference_is_refused_at_plan_time() {
    let h = openai_harness();
    save_pack(&h, decision_pack(&h));
    h.engine.plan(request("decision", json!({"style_pack": "house"}))).unwrap();
    write(&h, "art/decision_refs/ref_2.png", &disc(40, 40, 9.0, TEAL));
    let err = h.engine.plan(request("decision", json!({"style_pack": "house"}))).unwrap_err();
    assert_eq!(err.code, "pack_reference_changed", "{err:?}");
    let check = references(&h, json!({"action": "pack_validate", "name": "house"})).unwrap();
    assert_eq!(check["ok"], false);
    assert!(check["problems"][0].as_str().unwrap().contains("ref_2.png"), "{check}");
}

// ------------------------------------------------------------------ plans

#[test]
fn a_plan_with_a_pack_sends_its_references_and_states_its_look_and_form() {
    let h = openai_harness();
    save_pack(&h, decision_pack(&h));
    let base = h.engine.plan(request("decision", json!({}))).unwrap();
    let plan = h.engine.plan(request("decision", json!({"style_pack": "house"}))).unwrap();
    let text = &plan.prompt.text;
    // the pack's references go to the provider as style anchors, and nothing of vanilla goes with them
    let sent: Vec<&str> = plan.references.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(sent.len(), 5, "{sent:?}");
    assert!(sent.iter().all(|p| p.starts_with("art/decision_refs/")));
    assert!(plan.references.iter().all(|r| r.role == imagegen_core::refs::RefRole::StyleAnchor));
    assert!(plan.style_sources.as_ref().unwrap().chosen.iter().all(|c| !c.sent_to_provider));
    let sel = plan.style_selection.as_ref().unwrap();
    assert_eq!(sel.form_precedence, imagegen_core::plan::FormPrecedence::CallerReferences);
    assert_eq!(sel.contributions[0].tier, 0);
    assert!(sel.contributions[0].source.contains("style pack 'house'"));
    // the look line is in the style, the pack's form replaces the vanilla form and negatives
    assert!(text.contains("Style:\n") && text.contains("- flat teal pictogram on transparent, crisp pixel edges\n"), "{text}");
    assert!(text.contains("A flat 33x32 pictogram drawn in thin teal lines"), "{text}");
    let vanilla = ProfileSet::builtin().for_family(Family::Decision).unwrap().imagegen.prompt.clone();
    for rule in &vanilla.rules {
        let head: String = rule.split('{').next().unwrap().chars().take(40).collect();
        assert!(base.prompt.text.contains(&head), "the plan without the pack states the vanilla rule: {head}");
        assert!(!text.contains(&head), "the pack's form replaces the vanilla rule: {head}");
    }
    assert!(!text.contains(&format!("Avoid: {}", vanilla.negative.join("; "))));
    assert!(text.contains("Avoid: painted texture; a frame."), "{text}");
    assert!(text.contains("The style anchor images were chosen for this image"));
    // the plan reports the pack, why and which entry applied
    let rep = plan.style_pack.as_ref().unwrap();
    assert_eq!((rep.name.as_str(), rep.why.as_str(), rep.covered), ("house", "explicit", true));
    assert_eq!(rep.entry.as_deref(), Some("decision"));
    assert!(rep.form_override && rep.negative_override);
    assert_eq!(rep.references.iter().filter(|r| r.sent_to_provider).count(), 5);
    assert_eq!(plan.profile.style_pack.as_deref(), Some("house"));
    assert_ne!(plan.profile.hash, base.profile.hash);
    assert!(plan.warnings.iter().any(|w| w.contains("style pack 'house'")));
}

#[test]
fn the_callers_own_style_anchors_win_over_the_pack() {
    let h = openai_harness();
    save_pack(&h, decision_pack(&h));
    write(&h, "references/mine.png", &disc(64, 64, 20.0, [200, 40, 40, 255]));
    let plan = h
        .engine
        .plan(request("decision", json!({"style_pack": "house", "references": [{"path": "references/mine.png", "role": "style_anchor"}]})))
        .unwrap();
    let sent: Vec<&str> = plan.references.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(sent, vec!["references/mine.png"], "only the caller's anchor goes");
    let rep = plan.style_pack.as_ref().unwrap();
    assert!(rep.references.iter().all(|r| !r.sent_to_provider));
    assert!(rep.detail.contains("own style anchors win"), "{}", rep.detail);
    assert!(plan.prompt.text.contains("flat teal pictogram"), "the look still applies");
}

#[test]
fn a_gui_texture_pack_is_keyed_by_texture_kind() {
    let h = openai_harness();
    let panel_refs = refs(&h, "art/panel_refs", 3);
    let pack = json!({
        "name": "house",
        "families": {"gui_texture": {
            "look": "near-black navy, 1 px teal lines",
            "kinds": {"panel": {"look": "a navy panel with a 1 px teal border", "form": ["A flat navy panel filling {w}x{h}."], "references": panel_refs}}
        }}
    });
    save_pack(&h, pack);
    let panel = h.engine.plan(request("gui_texture", json!({"style_pack": "house", "texture_kind": "panel", "canvas": [200, 100]}))).unwrap();
    let rep = panel.style_pack.as_ref().unwrap();
    assert_eq!(rep.entry.as_deref(), Some("gui_texture.panel"));
    assert_eq!(panel.references.len(), 3);
    assert!(panel.prompt.text.contains("a navy panel with a 1 px teal border") && panel.prompt.text.contains("A flat navy panel filling 200x100."));
    assert!(!panel.prompt.text.contains("The Hearts of Iron IV panel form"), "the pack's form replaces the kind's vanilla form");
    let button = h.engine.plan(request("gui_texture", json!({"style_pack": "house", "texture_kind": "button", "canvas": [90, 40]}))).unwrap();
    let rep = button.style_pack.as_ref().unwrap();
    assert_eq!(rep.entry.as_deref(), Some("gui_texture"), "a kind the pack does not key falls back to the family entry");
    assert!(button.prompt.text.contains("near-black navy, 1 px teal lines"));
    assert!(button.prompt.text.contains("The Hearts of Iron IV button form"), "without a pack form the kind's vanilla form stays");
    assert!(button.references.is_empty());
}

#[test]
fn the_project_default_pack_applies_none_turns_it_off_and_uncovered_families_keep_the_tiers() {
    let h = harness("[defaults]\nprovider = \"openai\"\nstyle_pack = \"house\"\n", &[("OPENAI_API_KEY", "sk-test-synthetic-000")]);
    // the default names a pack that does not exist yet: a plan says so and how to plan without it
    let err = h.engine.plan(request("decision", json!({}))).unwrap_err();
    assert!(err.message.contains("style_pack: \"none\""), "{err:?}");
    save_pack(&h, decision_pack(&h));
    let plan = h.engine.plan(request("decision", json!({}))).unwrap();
    assert_eq!(plan.style_pack.as_ref().unwrap().why, "project_default");
    assert_eq!(plan.references.len(), 5);
    let off = h.engine.plan(request("decision", json!({"style_pack": "none"}))).unwrap();
    assert!(off.style_pack.is_none() && off.references.is_empty() && off.profile.style_pack.is_none());
    let focus = h.engine.plan(request("focus", json!({}))).unwrap();
    let rep = focus.style_pack.as_ref().unwrap();
    assert!(!rep.covered && rep.entry.is_none(), "focus is not in the pack");
    assert!(focus.profile.style_pack.is_none());
    assert!(!focus.prompt.text.contains("flat teal pictogram"));
}

// ------------------------------------------------------------------ processing switches

#[test]
fn keep_colours_keeps_a_counters_own_colours_and_draws_the_nato_box_as_an_outline() {
    let h = openai_harness();
    // a finished teal strip: without the switch it is not vanilla's material and is drawn again in green
    let mut strip = Rgba::new(152, 42);
    for y in 15..30 {
        for x in 20..56 {
            strip.set_px(x, y, TEAL);
        }
    }
    for x in 90..136 {
        strip.set_px(x, 11, TEAL);
        strip.set_px(x, 33, TEAL);
    }
    let off = import(&h, "counter_large", &strip, json!({"counter_symbol": "artillery"})).unwrap();
    let off_names: Vec<&str> = off["steps"].as_array().unwrap().iter().map(|s| s["op"]["op"].as_str().unwrap()).collect();
    assert!(off_names.contains(&"unit_glyph"), "{off_names:?}");
    let green = pixels(&h, off["path"].as_str().unwrap());
    let f1 = green.split_frames(2).unwrap().remove(0);
    let opaque: Vec<[u8; 4]> = f1.data.chunks_exact(4).filter(|p| p[3] == 255).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    assert!(opaque.iter().all(|p| p[0].abs_diff(p[2]) <= 2), "vanilla's material has red equal to blue");

    save_pack(
        &h,
        json!({"name": "house", "families": {"counter_large": {"look": "teal glyph", "form": ["A flat teal glyph."], "processing": {"keep_colours": true}}}}),
    );
    let on = import(&h, "counter_large", &strip, json!({"style_pack": "house", "counter_symbol": "artillery"})).unwrap();
    assert_eq!(on["recipe"]["id"], "kept_unit_art");
    let kept = pixels(&h, on["path"].as_str().unwrap());
    assert_eq!(kept, strip, "finished art at the accepted size is kept as it is");
    let v = validated(&h, on["artifact_id"].as_str().unwrap());
    assert_eq!(v["status"], "pass", "{v}");
    assert!(!v["checks"].as_array().unwrap().contains(&json!("unit_material")));

    // a large source in its own colours: fitted into vanilla's glyph box, frame 2 the NATO box outlined in its colour
    let mut src = Rgba::new(600, 300);
    for y in 80..220 {
        for x in 60..540 {
            src.set_px(x, y, [233, 120, 110, 255]);
        }
    }
    let made = import(&h, "counter_large", &src, json!({"style_pack": "house", "counter_symbol": "artillery"})).unwrap();
    let names: Vec<&str> = made["steps"].as_array().unwrap().iter().map(|s| s["op"]["op"].as_str().unwrap()).collect();
    assert!(names.contains(&"fit_in_box") && names.contains(&"nato_outline") && !names.contains(&"unit_glyph"), "{names:?}");
    let out = pixels(&h, made["path"].as_str().unwrap());
    assert_eq!((out.width, out.height), (152, 42));
    let b = out.split_frames(2).unwrap();
    assert_eq!(b[0].px(38, 21), [233, 120, 110, 255], "frame 1 keeps the art's salmon");
    assert_eq!(b[1].px(14, 20), [233, 120, 110, 255], "the box border is drawn in the glyph's colour");
    assert_eq!(b[1].px(20, 28)[3], 0, "the box has no fill");
    assert!(b[1].px(37, 22)[3] > 0, "the artillery dot is drawn");
    assert_eq!(validated(&h, made["artifact_id"].as_str().unwrap())["status"], "pass");
}

#[test]
fn upright_print_keeps_the_art_upright_at_the_packs_canvas() {
    let h = openai_harness();
    let photo = gradient(700, 820);
    let off = import(&h, "report_event", &photo, json!({})).unwrap();
    let card = pixels(&h, off["path"].as_str().unwrap());
    assert_eq!((card.width, card.height), (210, 176));
    assert_eq!(card.alpha(0, 0), 0, "vanilla's card is tilted with transparent corners");

    save_pack(
        &h,
        json!({"name": "house", "families": {"report_event": {
            "look": "black and white photo on a slightly tilted white print", "form": ["A photograph on a white paper print."],
            "canvas": [175, 204], "processing": {"upright_print": true}
        }}}),
    );
    let on = import(&h, "report_event", &photo, json!({"style_pack": "house"})).unwrap();
    assert_eq!(on["recipe"]["id"], "upright_print");
    let print = pixels(&h, on["path"].as_str().unwrap());
    assert_eq!((print.width, print.height), (175, 204));
    let want = imagegen_core::process::apply(&photo, &imagegen_core::process::Op::CoverResize { width: 175, height: 204 }).unwrap();
    assert_eq!(print, want, "no tilt, tone, mask or shadow is added");
    let v = validated(&h, on["artifact_id"].as_str().unwrap());
    assert_eq!(v["status"], "pass", "{v}");
    assert!(!v["checks"].as_array().unwrap().contains(&json!("card_corners")));

    // a print drawn on transparency keeps its own alpha
    let mut cut = Rgba::new(500, 600);
    for y in 40..560 {
        for x in 30..470 {
            cut.set_px(x, y, [230, 230, 225, 255]);
        }
    }
    let alpha = import(&h, "report_event", &cut, json!({"style_pack": "house"})).unwrap();
    let names: Vec<&str> = alpha["steps"].as_array().unwrap().iter().map(|s| s["op"]["op"].as_str().unwrap()).collect();
    assert!(names.contains(&"fit_subject"), "{names:?}");
    assert_eq!(validated(&h, alpha["artifact_id"].as_str().unwrap())["status"], "pass");
}

#[test]
fn a_pack_canvas_makes_a_tall_news_strip() {
    let h = openai_harness();
    save_pack(&h, json!({"name": "house", "families": {"news_event": {"look": "tall black and white press photograph", "canvas": [161, 420]}}}));
    let plan = h.engine.plan(request("news_event", json!({"style_pack": "house"}))).unwrap();
    assert_eq!(plan.canvas, [161, 420]);
    assert!(plan.prompt.text.contains("161x420"));
    assert!(plan.warnings.iter().any(|w| w.contains("differs from the vanilla-measured default 397x153")));
    let made = import(&h, "news_event", &gradient(600, 1560), json!({"style_pack": "house"})).unwrap();
    let out = pixels(&h, made["path"].as_str().unwrap());
    assert_eq!((out.width, out.height), (161, 420));
    assert!(out.data.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2]), "news stays grayscale");
    assert_eq!(validated(&h, made["artifact_id"].as_str().unwrap())["status"], "pass");
    let off = import(&h, "news_event", &gradient(600, 1560), json!({"style_pack": "none"})).unwrap();
    let out = pixels(&h, off["path"].as_str().unwrap());
    assert_eq!((out.width, out.height), (397, 153));
}

#[test]
fn fill_canvas_and_a_form_override_replace_the_technology_side_view() {
    let h = openai_harness();
    let panel = Rgba::filled(1000, 490, [10, 20, 24, 255]);
    let err = import(&h, "technology", &panel, json!({})).unwrap_err();
    assert_eq!(err.code, "no_transparency_in_source", "without the switch an opaque panel is refused");
    save_pack(
        &h,
        json!({"name": "house", "families": {"technology": {
            "look": "teal line-art blueprint on a dark navy panel",
            "form": ["A blueprint panel that fills the whole {w}x{h} canvas: the equipment drawn in thin teal lines on dark navy."],
            "canvas": [239, 117], "processing": {"fill_canvas": true}
        }}}),
    );
    let plan = h.engine.plan(request("technology", json!({"style_pack": "house"}))).unwrap();
    assert!(!plan.generation.transparent, "a panel is asked for as an opaque picture");
    assert!(!plan.prompt.text.contains("seen exactly from the side"), "the painted side-view rule is gone");
    assert!(plan.prompt.text.contains("A blueprint panel that fills the whole 239x117 canvas"));
    let on = import(&h, "technology", &panel, json!({"style_pack": "house"})).unwrap();
    assert_eq!(on["recipe"]["id"], "filled_icon");
    let out = pixels(&h, on["path"].as_str().unwrap());
    assert_eq!((out.width, out.height), (239, 117));
    let v = validated(&h, on["artifact_id"].as_str().unwrap());
    assert_eq!(v["status"], "pass", "{v}");
    for gone in ["margin", "fill", "opaque_canvas", "side_view"] {
        assert!(!v["checks"].as_array().unwrap().contains(&json!(gone)), "{gone}");
    }
}

#[test]
fn crisp_adds_no_colour_outside_the_arts_palette() {
    let h = openai_harness();
    let off_palette = |img: &Rgba| img.data.chunks_exact(4).filter(|p| p[3] > 0 && (0..3).any(|c| p[c].abs_diff(TEAL[c]) > 6)).count();
    let src = pictogram(640, TEAL);
    let off = import(&h, "decision", &src, json!({})).unwrap();
    let lanczos = pixels(&h, off["path"].as_str().unwrap());
    save_pack(&h, json!({"name": "house", "families": {"decision": {"look": "flat teal pictogram", "processing": {"crisp": true}}}}));
    let on = import(&h, "decision", &src, json!({"style_pack": "house"})).unwrap();
    let area = pixels(&h, on["path"].as_str().unwrap());
    eprintln!("off-palette pixels: lanczos {} of 33x32, area {}", off_palette(&lanczos), off_palette(&area));
    assert!(off_palette(&lanczos) > 0, "the vanilla fit rings at hard edges");
    assert_eq!(off_palette(&area), 0, "the crisp fit keeps every colour on the art's own palette");
    let names: Vec<&str> = on["steps"].as_array().unwrap().iter().map(|s| s["op"]["op"].as_str().unwrap()).collect();
    assert!(names.contains(&"area_fit") && !names.contains(&"fit_subject") && !names.contains(&"defringe"), "{names:?}");
    let v = validated(&h, on["artifact_id"].as_str().unwrap());
    assert_eq!(v["status"], "pass", "{v}");
    assert!(!v["checks"].as_array().unwrap().contains(&json!("fringe")));
    // green line art, which the vanilla chain reads as key colour: its defringe finds no clean interior and refuses
    // the image; the crisp chain runs no defringe and keeps it
    let green = pictogram(640, [60, 220, 90, 255]);
    let off = import(&h, "decision", &green, json!({"style_pack": "none"})).unwrap_err();
    assert!(off.message.contains("no uncontaminated opaque interior"), "{off:?}");
    let on = import(&h, "decision", &green, json!({"style_pack": "house"})).unwrap();
    assert!(on["steps"].as_array().unwrap().iter().all(|s| s["op"]["op"] != "defringe"));
    assert_eq!(validated(&h, on["artifact_id"].as_str().unwrap())["status"], "pass");
}

// ------------------------------------------------------------------ the review against the pack's references

/// A harness whose side-by-side sheets go to a temporary cache folder, removed with it.
fn cache_harness() -> (Harness, tempfile::TempDir) {
    let cache = tempfile::Builder::new().prefix("c").tempdir_in(repo_work_dir()).unwrap();
    let path = cache.path().display().to_string();
    (harness("[defaults]\nprovider = \"openai\"\n", &[("OPENAI_API_KEY", "sk-test-synthetic-000"), ("HOI4_IMAGEGEN_CACHE_DIR", &path)]), cache)
}

fn review(h: &Harness, id: &str, extra: Value) -> Result<Value, imagegen_mcp::EngineError> {
    let rubric_id = ProfileSet::builtin().for_family(Family::Decision).unwrap().imagegen.rubric.clone();
    let answers: serde_json::Map<String, Value> = rubric::questions(&rubric_id).iter().map(|(k, _)| (k.to_string(), json!(true))).collect();
    let mut p = json!({"target": id, "verdict": "approved", "reviewer": "test reviewer", "answers": answers});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    h.engine.review(serde_json::from_value(p).unwrap())
}

#[test]
fn the_review_measures_pack_art_against_the_packs_references() {
    let (h, _cache) = cache_harness();
    save_pack(&h, decision_pack(&h));
    // like the pack's references: a teal disc
    let like = import(&h, "decision", &disc(640, 640, 250.0, TEAL), json!({"style_pack": "house"})).unwrap();
    let id = like["artifact_id"].as_str().unwrap();
    assert_eq!(validated(&h, id)["status"], "pass");
    let pv = h.preview(id);
    assert_eq!(pv["vanilla_check"]["reference_set"]["kind"], "style_pack", "{}", pv["vanilla_check"]);
    assert_eq!(pv["vanilla_sheet"]["references"].as_array().unwrap().len(), 5, "the side-by-side sheet shows the pack's references");
    let ok = review(&h, id, json!({})).unwrap();
    assert!(ok["approval_id"].as_str().unwrap().starts_with("apr-"), "{ok}");
    let set = &ok["vanilla_check"]["reference_set"];
    assert_eq!(
        (set["kind"].as_str(), set["pack"].as_str(), set["entry"].as_str(), set["rule"].as_str()),
        (Some("style_pack"), Some("house"), Some("decision"), Some("pack_match_v2"))
    );
    assert_eq!(ok["vanilla_check"]["status"], "pass");

    // unlike them: a thin grey bar fails the pack's ranges; an override with a reason is still possible and recorded
    let mut bar = Rgba::new(640, 640);
    for y in 300..340 {
        for x in 20..620 {
            bar.set_px(x, y, [128, 128, 128, 255]);
        }
    }
    let unlike = import(&h, "decision", &bar, json!({"style_pack": "house"})).unwrap();
    let id = unlike["artifact_id"].as_str().unwrap();
    assert_eq!(validated(&h, id)["status"], "pass");
    h.preview(id);
    let err = review(&h, id, json!({})).unwrap_err();
    assert_eq!(err.code, "review_vanilla_mismatch");
    assert!(err.message.contains("the style pack 'house' references"), "{}", err.message);
    let over = review(&h, id, json!({"override_vanilla_check": true, "override_reason": "a deliberate one-off bar for this test"})).unwrap();
    assert!(over["approval_id"].is_string());
    assert_eq!(over["vanilla_override"]["status_overridden"], "fail");
    assert_eq!(over["vanilla_check"]["reference_set"]["kind"], "style_pack");

    // without a pack the same art is measured against vanilla, which this project cannot read (no game)
    let plain = import(&h, "decision", &disc(640, 640, 250.0, TEAL), json!({"style_pack": "none"})).unwrap();
    let id = plain["artifact_id"].as_str().unwrap();
    h.validate(id);
    h.preview(id);
    let err = review(&h, id, json!({})).unwrap_err();
    assert_eq!(err.code, "review_needs_vanilla_preview", "{err:?}");
    assert_eq!(err.details.unwrap()["vanilla_check"]["reference_set"]["kind"], "vanilla");
}

#[test]
fn the_pack_match_rule_accepts_the_spread_of_its_own_references() {
    // leave one out over synthetic sets: each reference passes ranges fitted on the others
    let rows: Vec<_> = (0..8)
        .map(|i| imagegen_core::pack::measure_reference(&disc(40, 40, 13.0 + f64::from(i) * 0.6, [70 + i as u8 * 3, 198, 190, 255])).unwrap())
        .collect();
    assert_eq!(imagegen_core::pack::leave_one_out(Family::Decision, &rows), Some((8, 8)));
    let cal = imagegen_core::pack::calibrate(Family::Decision, &rows).unwrap();
    assert_eq!(cal.version, "pack_match_v2");
    let mut bar = Rgba::new(40, 40);
    for y in 18..21 {
        for x in 2..38 {
            bar.set_px(x, y, [128, 128, 128, 255]);
        }
    }
    assert!(!cal.check(&imagegen_core::vanillamatch::measure(&bar).unwrap()).pass);
    assert!(imagegen_core::pack::calibrate(Family::Decision, &rows[..2]).is_none(), "fewer than three references give no ranges");
}

// ------------------------------------------------------------------ kept-colours unit art with a pack's references

const LIGHT_TEAL: [u8; 3] = [89, 199, 194];

/// A flat glyph (a hull with a turret) of `w` x `h` pixels in `colour`, centred on a 76x42 frame.
fn glyph_frame(w: u32, h: u32, colour: [u8; 3]) -> Rgba {
    let mut f = Rgba::new(76, 42);
    let (x0, y0) = (38 - w / 2, 22 - h / 2);
    for y in 0..h {
        for x in 0..w {
            let turret = y < h / 3 && x > w / 3 && x < 2 * w / 3;
            if y >= h / 3 || turret {
                f.set_px(x0 + x, y0 + y, [colour[0], colour[1], colour[2], 255]);
            }
        }
    }
    f
}

/// A counter strip like a terminal-style mod's: the glyph, then the NATO box outlined in its colour, both with a soft
/// glow (alpha 73 and 12 on frame 1, 60 and 18 on frame 2).
fn counter_strip(w: u32, h: u32, colour: [u8; 3]) -> Rgba {
    use imagegen_core::recipes::{pack_look, unit_art};
    let marks = unit_art::parse_symbol("medium_armor").unwrap();
    let strip = unit_art::outline_strip(&glyph_frame(w, h, colour), unit_art::UnitArtKind::CounterLarge, &marks).unwrap();
    pack_look::glow(&strip, 2, &[[73, 12], [60, 18]]).unwrap()
}

fn counter_refs(h: &Harness) -> Vec<Value> {
    [(36, 18), (35, 19), (44, 20), (30, 21), (48, 24), (52, 22), (40, 18), (56, 21)]
        .iter()
        .enumerate()
        .map(|(i, (w, hh))| {
            let rel = format!("art/counter_refs/ref_{i}.png");
            write(h, &rel, &counter_strip(*w, *hh, LIGHT_TEAL));
            json!({"path": rel, "sha256": "", "note": "synthetic test reference"})
        })
        .collect()
}

fn counter_pack(h: &Harness, processing: Value) -> Value {
    save_pack(
        h,
        json!({"name": "house", "families": {"counter_large": {"look": "teal glyph", "form": ["A flat teal glyph."], "processing": processing, "references": counter_refs(h)}}}),
    )
}

/// A large source glyph in `colour`, as a model draws it.
fn big_glyph(colour: [u8; 3]) -> Rgba {
    let mut src = Rgba::new(600, 300);
    for y in 60..240 {
        for x in 60..540 {
            if y >= 120 || (x > 220 && x < 380) {
                src.set_px(x, y, [colour[0], colour[1], colour[2], 255]);
            }
        }
    }
    src
}

fn step_names(v: &Value) -> Vec<String> {
    v["steps"].as_array().unwrap().iter().map(|s| s["op"]["op"].as_str().unwrap().to_string()).collect()
}

#[test]
fn the_palette_snap_moves_a_darker_teal_to_the_packs_light_teal_and_records_it() {
    let h = openai_harness();
    let saved = counter_pack(&h, json!({"keep_colours": true, "crisp": true}));
    let hashes: Vec<Value> =
        saved["pack"]["families"]["counter_large"]["references"].as_array().unwrap().iter().map(|r| r["sha256"].clone()).collect();
    let made = import(&h, "counter_large", &big_glyph([59, 181, 177]), json!({"style_pack": "house", "counter_symbol": "medium_armor"})).unwrap();
    assert_eq!(made["recipe"], json!({"id": "kept_unit_art", "version": 2}));
    let names = step_names(&made);
    assert_eq!(names, vec!["trim_to_subject", "area_fit", "palette_snap", "nato_outline", "edge_glow"], "{names:?}");
    let snap = &made["steps"][2]["op"];
    assert_eq!(snap["palette"], json!([{"rgb": LIGHT_TEAL, "share": 1.0}]));
    assert_eq!(snap["pack"], "house");
    assert_eq!(snap["references"], json!(hashes), "the step records the hash of every reference it was measured from");
    assert_eq!(made["steps"][4]["op"]["glow"], json!([[73, 12], [60, 18]]));
    let fit = &made["steps"][1]["op"];
    assert!(fit["box_width"].as_u64().unwrap() < 62, "a dense glyph is fitted to the references' size: {fit}");

    let out = pixels(&h, made["path"].as_str().unwrap());
    let f = out.split_frames(2).unwrap();
    assert_eq!(f[0].px(38, 26), [89, 199, 194, 255], "the glyph is the pack's teal");
    assert_eq!(f[1].px(14, 20), [89, 199, 194, 255], "so is the outline drawn in its colour");
    assert_eq!(f[1].px(13, 20), [89, 199, 194, 60], "the outline has the pack's glow");
    let edge = (0..76).find(|&x| f[0].px(x, 26)[3] >= 200).unwrap();
    assert_eq!(f[0].px(edge - 2, 26)[3], 12, "two pixels outside the glyph, the outer glow");
    assert_eq!(validated(&h, made["artifact_id"].as_str().unwrap())["status"], "pass");

    // a colour the pack does not have is kept: a salmon naval glyph stays salmon
    let salmon = import(&h, "counter_large", &big_glyph([213, 102, 100]), json!({"style_pack": "house", "counter_symbol": "medium_armor"})).unwrap();
    let s = pixels(&h, salmon["path"].as_str().unwrap()).split_frames(2).unwrap().remove(0);
    assert_eq!(s.px(38, 26), [213, 102, 100, 255]);

    // the switches turn it off; the fit to the references' size stays
    counter_pack(&h, json!({"keep_colours": true, "crisp": true, "snap_palette": false, "edge_glow": false}));
    let off = import(&h, "counter_large", &big_glyph([59, 181, 177]), json!({"style_pack": "house", "counter_symbol": "medium_armor"})).unwrap();
    assert_eq!(step_names(&off), vec!["trim_to_subject", "area_fit", "nato_outline"]);
    assert_eq!(pixels(&h, off["path"].as_str().unwrap()).split_frames(2).unwrap()[0].px(38, 26), [59, 181, 177, 255]);
}

#[test]
fn a_flat_glyph_one_shade_off_passes_the_pack_match_and_a_half_size_one_still_fails() {
    use imagegen_core::{pack, vanillamatch};
    let sizes = [(36, 18), (35, 19), (44, 20), (30, 21), (48, 24), (52, 22), (40, 18), (56, 21)];
    let rows: Vec<pack::ReferenceRow> = sizes.iter().map(|(w, h)| pack::measure_reference(&counter_strip(*w, *h, LIGHT_TEAL)).unwrap()).collect();
    let feats: Vec<vanillamatch::Features> = rows.iter().map(|r| r.features).collect();
    // one shade darker: every channel 16 levels down, the step of `pack::SHADE_STEP`
    let shade = [LIGHT_TEAL[0] - 16, LIGHT_TEAL[1] - 16, LIGHT_TEAL[2] - 16];
    let darker = vanillamatch::measure(&counter_strip(44, 20, shade)).unwrap();
    // the 0.3.5 rule refused it on the middle-value share, a step function of one flat colour
    let v1 = vanillamatch::calibrate_min_max(Family::CounterLarge, &feats, 1.0, 2.0, "pack_match_v1", None, &[]).unwrap();
    let old = v1.check(&darker);
    assert!(!old.pass && old.outside.iter().any(|o| o.feature == "mid_tone_share"), "{old:?}");
    for family in [Family::CounterLarge, Family::Decision] {
        let cal = pack::calibrate(family, &rows).unwrap();
        assert_eq!(cal.version, "pack_match_v2");
        let res = cal.check(&darker);
        assert!(res.pass, "{family}: {:?}", res.outside);
    }
    let cal = pack::calibrate(Family::CounterLarge, &rows).unwrap();
    let mid = cal.ranges.iter().find(|r| r.feature == "mid_tone_share").unwrap();
    assert!(!mid.checked, "unit art measures and reports the middle-value share, and never fails on it");
    assert!(cal.ranges.iter().find(|r| r.feature == "saturation_mean").unwrap().checked);
    assert!(pack::calibrate(Family::Decision, &rows).unwrap().ranges.iter().find(|r| r.feature == "mid_tone_share").unwrap().checked);

    // three shades off is another colour, and still fails on saturation
    let far = [LIGHT_TEAL[0] - 48, LIGHT_TEAL[1] - 48, LIGHT_TEAL[2] - 48];
    let res = cal.check(&vanillamatch::measure(&counter_strip(44, 20, far)).unwrap());
    assert!(res.outside.iter().any(|o| o.feature == "saturation_mean"), "{res:?}");

    // a glyph half the size (a quarter of the ink) still fails. Alpha coverage stays checked, but on a two-frame strip
    // the glowing NATO box holds most of the visible pixels, so the strip's centre of mass, pulled toward frame 2, is
    // what sees the light glyph
    let half = cal.check(&vanillamatch::measure(&counter_strip(22, 10, LIGHT_TEAL)).unwrap());
    assert!(!half.pass && half.outside.iter().any(|o| o.feature == "subject_center_x"), "{half:?}");
    assert!(cal.ranges.iter().find(|r| r.feature == "alpha_coverage").unwrap().checked);
    // a strip with no glyph on frame 1 at all fails alpha coverage too
    let marks = imagegen_core::recipes::unit_art::parse_symbol("medium_armor").unwrap();
    let empty = imagegen_core::recipes::unit_art::outline_strip(
        &glyph_frame(2, 2, LIGHT_TEAL),
        imagegen_core::recipes::unit_art::UnitArtKind::CounterLarge,
        &marks,
    )
    .unwrap();
    let res = cal.check(&vanillamatch::measure(&empty).unwrap());
    assert!(res.outside.iter().any(|o| o.feature == "alpha_coverage"), "{res:?}");
}

#[test]
fn without_a_pack_the_counter_and_the_vanilla_check_are_unchanged() {
    let h = openai_harness();
    // no pack: vanilla's material, no palette or glow step
    let plain = import(&h, "counter_large", &big_glyph([59, 181, 177]), json!({"counter_symbol": "medium_armor"})).unwrap();
    assert_eq!(plain["recipe"]["id"], "unit_counter");
    let names = step_names(&plain);
    assert!(names.contains(&"unit_glyph".to_string()) && !names.iter().any(|n| n == "palette_snap" || n == "edge_glow"), "{names:?}");
    // the vanilla rule still checks the middle-value share of unit art
    let rows: Vec<_> = (0..5).map(|i| imagegen_core::vanillamatch::measure(&counter_strip(36 + i * 4, 20, LIGHT_TEAL)).unwrap()).collect();
    let cal = imagegen_core::vanillamatch::calibrate(Family::CounterLarge, &rows).unwrap();
    assert!(cal.ranges.iter().find(|r| r.feature == "mid_tone_share").unwrap().checked);
    assert_eq!(cal.version, "vanilla_match_v2");
    // a pack that keeps colours but has no references: the art keeps its shades, and the note says why
    save_pack(
        &h,
        json!({"name": "house", "families": {"counter_large": {"look": "teal glyph", "form": ["A flat teal glyph."], "processing": {"keep_colours": true}}}}),
    );
    let bare = import(&h, "counter_large", &big_glyph([59, 181, 177]), json!({"style_pack": "house", "counter_symbol": "medium_armor"})).unwrap();
    assert_eq!(step_names(&bare), vec!["trim_to_subject", "fit_in_box", "nato_outline"]);
    assert!(bare["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("snap_palette")), "{}", bare["notes"]);
}

// ------------------------------------------------------------------ the folder import

fn reference_folder() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("f").tempdir_in(repo_work_dir()).unwrap();
    let put = |rel: &str, img: &Rgba| {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, encode_png(img).unwrap()).unwrap();
    };
    for i in 0..4 {
        put(&format!("decision/house_decision_ref_{i}.png"), &disc(40, 40, 14.0 + f64::from(i), TEAL));
    }
    put("decision/contact-sheet.png", &Rgba::filled(400, 100, [40, 40, 40, 255]));
    std::fs::write(dir.path().join("decision/look.txt"), "flat teal pictogram from look.txt\n").unwrap();
    for i in 0..3 {
        put(&format!("gui_texture/panel_{i}.png"), &Rgba::filled(120, 60, [10, 20, 24, 255]));
        put(&format!("gui_texture/button_{i}.png"), &Rgba::filled(90, 24, [12, 30, 30, 255]));
    }
    let list: Vec<Value> = (0..3)
        .flat_map(|i| {
            [
                json!({"ref": format!("panel_{i}.png"), "texture_kind": "panel", "source_path": format!("gfx/interface/panel_{i}.dds")}),
                json!({"ref": format!("button_{i}.png"), "texture_kind": "button"}),
            ]
        })
        .collect();
    std::fs::write(
        dir.path().join("gui_texture/references.json"),
        serde_json::to_vec(&json!({"family": "gui_texture", "references": list})).unwrap(),
    )
    .unwrap();
    for i in 0..3 {
        put(&format!("top_ref_{i}.png"), &disc(100, 88, 30.0, [150, 150, 150, 255]));
    }
    let top: Vec<Value> = (0..3).map(|i| json!({"ref": format!("top_ref_{i}.png")})).collect();
    std::fs::write(dir.path().join("manifest.json"), serde_json::to_vec(&json!({"references": top})).unwrap()).unwrap();
    std::fs::create_dir_all(dir.path().join("notes")).unwrap();
    std::fs::write(
        dir.path().join("gallery-plan.json"),
        serde_json::to_vec(&json!({"families": {"decision": {"look": "the plan's decision look"}, "gui_texture": {"look": "navy terminal panels"}, "focus": {"look": "silver monochrome"}}})).unwrap(),
    )
    .unwrap();
    dir
}

#[test]
fn a_reference_folder_is_imported_into_the_mod_and_pinned() {
    let h = openai_harness();
    let folder = reference_folder();
    let v = h.engine.import_pack_folder("house", folder.path(), Some(Family::Focus), false).unwrap();
    assert_eq!(v["files_copied"], 13, "{v}");
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("notes is not a family id")));
    let p = imagegen_core::pack::load(&imagegen_core::fsio::ProjectFs::new(h.root()).unwrap(), "house").unwrap();
    let d = &p.families[&Family::Decision];
    assert_eq!(d.look.as_deref(), Some("flat teal pictogram from look.txt"), "look.txt wins over the plan's look");
    assert_eq!(d.references.len(), 4, "the contact sheet is not a reference");
    assert!(d.references.iter().all(|r| r.path.starts_with(".hoi4-imagegen/packs/house/decision/") && r.sha256.len() == 64));
    assert!(h.exists(".hoi4-imagegen/packs/house/decision/house_decision_ref_0.png"));
    let g = &p.families[&Family::GuiTexture];
    assert_eq!(g.look.as_deref(), Some("navy terminal panels"));
    assert!(g.references.is_empty());
    assert_eq!(g.kinds[&imagegen_core::GuiTextureKind::Panel].references.len(), 3);
    assert!(g.kinds[&imagegen_core::GuiTextureKind::Panel].references[0].note.contains("gfx/interface/panel_0.dds"));
    assert_eq!(g.kinds[&imagegen_core::GuiTextureKind::Button].references.len(), 3);
    assert_eq!(p.families[&Family::Focus].references.len(), 3, "top-level files belong to the named family");
    assert_eq!(v["validation"]["ok"], true);
    // nothing is written outside the mod, and the source folder is untouched
    assert!(!folder.path().join(".hoi4-imagegen").exists());

    // importing again needs replace, and keeps hand-set form, canvas and switches
    assert!(h.engine.import_pack_folder("house", folder.path(), Some(Family::Focus), false).is_err());
    let mut edited = serde_json::to_value(&p).unwrap();
    edited["families"]["decision"]["form"] = json!(["A flat pictogram."]);
    edited["families"]["decision"]["processing"] = json!({"crisp": true});
    save_pack(&h, edited);
    h.engine.import_pack_folder("house", folder.path(), Some(Family::Focus), true).unwrap();
    let p = imagegen_core::pack::load(&imagegen_core::fsio::ProjectFs::new(h.root()).unwrap(), "house").unwrap();
    assert_eq!(p.families[&Family::Decision].form, vec!["A flat pictogram.".to_string()]);
    assert!(p.families[&Family::Decision].processing.crisp);
    // and a plan can use what was imported
    let plan = h.engine.plan(request("gui_texture", json!({"style_pack": "house", "texture_kind": "panel", "canvas": [120, 60]}))).unwrap();
    assert_eq!(plan.style_pack.as_ref().unwrap().entry.as_deref(), Some("gui_texture.panel"));
    assert_eq!(plan.references.len(), 3);
}

#[test]
fn an_import_outside_a_folder_or_with_a_reserved_name_is_refused() {
    let h = openai_harness();
    assert!(h.engine.import_pack_folder("none", Path::new("."), None, false).is_err());
    assert!(h.engine.import_pack_folder("house", &h.root().join("absent"), None, false).is_err());
}

// ------------------------------------------------------------------ the measured style paragraph

/// The measured paragraph stays in the prompt when nothing is attached at all (a model that takes no references), and
/// goes only when the request sends references and none of them is automatic.
#[test]
fn the_measured_paragraph_stays_for_a_text_only_model() {
    let h = harness("[defaults]\nprovider = \"openai\"\n", &[("OPENAI_API_KEY", "sk-test-synthetic-000"), ("FAL_KEY", "fal-test-synthetic-000")]);
    let refs = refs(&h, "art/pinned", 3);
    references(&h, json!({"action": "style_save", "style": {"name": "mine", "families": ["decision"], "references": refs}})).unwrap();
    let paragraph =
        |plan: &imagegen_core::plan::Plan| plan.style_sources.as_ref().and_then(|s| s.style_paragraph.clone()).expect("a measured paragraph");
    // a text-only model: nothing is attached, the paragraph is the only style the model gets
    let text_only = h.engine.plan(request("decision", json!({"provider": "fal", "model": "fal-ai/flux/dev", "quality": null}))).unwrap();
    assert!(text_only.references.is_empty());
    assert!(text_only.prompt.text.contains(&paragraph(&text_only)), "{}", text_only.prompt.text);
    // automatic references are sent: the paragraph describes them and stays
    let with_refs = h.engine.plan(request("decision", json!({}))).unwrap();
    assert_eq!(with_refs.references.len(), 3);
    assert!(with_refs.prompt.text.contains(&paragraph(&with_refs)));
    // the caller's own anchors replace the automatic ones: the paragraph would describe images that are not sent
    write(&h, "references/mine.png", &disc(64, 64, 20.0, [200, 40, 40, 255]));
    let anchored = h.engine.plan(request("decision", json!({"references": [{"path": "references/mine.png", "role": "style_anchor"}]}))).unwrap();
    assert_eq!(anchored.references.len(), 1);
    assert!(!anchored.prompt.text.contains(&paragraph(&anchored)));
}

/// A pack's references replace every automatic tier (style_mode replace), style_mode add sends the automatic ones
/// beside them, and the sub-family is still recorded while the pack decides the references and the form.
#[test]
fn pack_references_follow_style_mode_and_the_subfamily_is_recorded() {
    let h = openai_harness();
    save_pack(&h, decision_pack(&h));
    let pinned = refs(&h, "art/pinned", 3);
    references(&h, json!({"action": "style_save", "style": {"name": "mine", "families": ["decision"], "references": pinned}})).unwrap();
    let sub = imagegen_core::subfamily::for_family(Family::Decision).unwrap().ids()[0].to_string();

    let replace = h.engine.plan(request("decision", json!({"style_pack": "house", "subfamily": sub}))).unwrap();
    assert_eq!(replace.references.len(), 5, "only the pack's references");
    assert!(replace.references.iter().all(|r| r.path.starts_with("art/decision_refs/")));
    assert!(replace.style_sources.as_ref().unwrap().chosen.iter().all(|c| !c.sent_to_provider));
    let sel = replace.style_selection.as_ref().unwrap();
    assert_eq!(sel.style_mode, Some(imagegen_core::plan::StyleMode::Replace));
    let s = replace.subfamily.as_ref().unwrap();
    assert_eq!((s.id.as_str(), s.how.as_str()), (sub.as_str(), "named"));
    assert!(s.form_note.is_none(), "the pack's form replaces the family's, so the sub-family note stays out");
    assert!(!s.references_from_subfamily);

    let add = h.engine.plan(request("decision", json!({"style_pack": "house", "style_mode": "add"}))).unwrap();
    let paths: Vec<&str> = add.references.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.iter().filter(|p| p.starts_with("art/decision_refs/")).count() == 5, "{paths:?}");
    assert!(paths.iter().any(|p| p.starts_with("art/pinned/")), "the automatic (pinned) references go beside the pack's: {paths:?}");
    assert_eq!(add.style_selection.as_ref().unwrap().style_mode, Some(imagegen_core::plan::StyleMode::Add));
    assert!(add.style_pack.is_some() && add.style_sources.is_some());
}
