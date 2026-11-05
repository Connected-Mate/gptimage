//! L2: the animated families, end to end through the engine.
//!
//! SYNTHETIC: provider replies come from the fixture transport and the pictures are drawn by these tests. What they
//! prove is the tool's own part: one base frame is generated, the other frames are derived from it (or fitted with
//! it), the animation check measures the strip, the preview writes a frame sheet and a looping GIF, and install
//! registers a `frameAnimatedSpriteType`. Whether the game plays the strip is an in-game check, not made here.

mod common;

use common::*;
use imagegen_core::dds;
use imagegen_core::family::Family;
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba};
use imagegen_core::plan::PlanRequest;
use imagegen_mcp::api::GroupFollowUp;
use imagegen_mcp::EngineError;
use serde_json::{json, Value};

fn generate(h: &Harness, family: &str, asset: &str, png: &[u8], key: &str, canvas: Option<[u32; 2]>) -> (Value, String) {
    h.queue_generation(png);
    let fam: Family = family.parse().unwrap();
    let mut req =
        json!({"family": family, "feature_slug": "iron_pact", "asset": asset, "brief": "a test subject", "provider": "openai", "quality": "low"});
    if let Some(c) = canvas {
        req["canvas"] = json!(c);
    }
    let req: PlanRequest = serde_json::from_value(req).unwrap();
    let follow = GroupFollowUp { approve: true, idempotency_key: Some(key.into()), approve_cost: false };
    let out = h.engine.group_request(fam.group(), req, follow).unwrap();
    let cand = out["generation"]["candidates"][0]["candidate_id"].as_str().unwrap().to_string();
    (out, cand)
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

fn codes(v: &Value) -> Vec<String> {
    v["results"][0]["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap().to_string()).collect()
}

fn png_of(h: &Harness, rel: &str) -> Rgba {
    decode(&h.read(rel), None, DecodeLimits::default()).unwrap().0
}

/// A gold ring of light on black, the way the model is asked to draw a glow.
fn glow_on_black() -> Vec<u8> {
    let mut img = Rgba::filled(928, 816, [0, 0, 0, 255]);
    for y in 0..816u32 {
        for x in 0..928u32 {
            let d = ((f64::from(x) - 464.0).powi(2) + (f64::from(y) - 408.0).powi(2)).sqrt();
            let v = (1.0 - ((d - 250.0).abs() / 60.0)).clamp(0.0, 1.0);
            if v > 0.0 {
                img.set_px(x, y, [(250.0 * v) as u8, (190.0 * v) as u8, (60.0 * v) as u8, 255]);
            }
        }
    }
    encode_png(&img).unwrap()
}

/// An opaque striped picture for the scroll.
fn stripes() -> Vec<u8> {
    let mut img = Rgba::new(928, 816);
    for y in 0..816u32 {
        for x in 0..928u32 {
            let v = if ((x + y) / 50) % 2 == 0 { 170 } else { 90 };
            img.set_px(x, y, [v, v - 20, 60, 255]);
        }
    }
    encode_png(&img).unwrap()
}

/// A notched disc: a turn shows.
fn cog() -> Vec<u8> {
    let mut img = Rgba::new(928, 816);
    for y in 0..816u32 {
        for x in 0..928u32 {
            let (dx, dy) = (f64::from(x) - 464.0, f64::from(y) - 408.0);
            let d = (dx * dx + dy * dy).sqrt();
            let notch = dx.abs() < 30.0 && dy < 0.0;
            if d < 300.0 && !notch {
                img.set_px(x, y, [90 + (x % 40) as u8, 140, 70, 255]);
            }
        }
    }
    encode_png(&img).unwrap()
}

/// The synthetic icon with its colour shifted and a bright core, as an edit of it would be.
fn edited(shift: u8) -> Vec<u8> {
    let mut img = decode(&synthetic_icon_png(), None, DecodeLimits::default()).unwrap().0;
    for p in img.data.chunks_exact_mut(4) {
        if p[3] > 0 {
            p[1] = p[1].saturating_add(shift);
        }
    }
    encode_png(&img).unwrap()
}

#[test]
fn a_blend_glow_is_drawn_on_black_and_gets_its_dim_frame_from_the_bright_one() {
    let h = openai_harness();
    let (out, cand) = generate(&h, "anim_blend_glow", "alert", &glow_on_black(), "k-glow-1", Some([60, 60]));
    assert_eq!(out["plan"]["request"]["transparent"], json!(false), "a glow is asked for as light on black: {:#}", out["plan"]);
    let p = process(&h, &cand, json!({})).unwrap();
    assert_eq!(names(&p, "steps"), vec!["flatten_on", "black_to_alpha", "fit_subject", "glow_frames"]);
    assert_eq!(p["size"], json!([120, 60]), "the plan's canvas is one frame");
    assert_eq!(p["chain"]["complete"], true, "{:#}", p["chain"]);
    let strip = png_of(&h, p["path"].as_str().unwrap());
    let f = strip.split_frames(2).unwrap();
    assert!(f[1].has_transparency() && f[1].alpha(0, 0) == 0, "black became transparent");
    let sum = |i: &Rgba| i.data.chunks_exact(4).map(|p| u64::from(p[3])).sum::<u64>();
    let ratio = sum(&f[0]) as f64 / sum(&f[1]) as f64;
    assert!((ratio - 0.17).abs() < 0.02, "frame 1 is the dim pulse: {ratio}");
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let v = h.validate(&id);
    assert_eq!(v["status"], "pass", "{v:#}");
    assert!(v["results"][0]["checks"].as_array().unwrap().iter().any(|c| c == "animation"), "{v:#}");

    // the preview writes a frame sheet and a looping GIF of the cross-fade
    let pv = h.preview(&id);
    let anim = &pv["animation"];
    assert_eq!(anim["frames"], 2);
    assert_eq!(anim["fps"], "1");
    let gif = h.read(anim["loop_gif"].as_str().unwrap());
    assert_eq!(&gif[..6], b"GIF89a");
    assert_eq!(anim["loop_gif_pictures"], 40, "two frames at 1 fps, blended at 20 pictures a second");
    assert!(h.exists(anim["frame_sheet"].as_str().unwrap()));

    // install registers a frameAnimatedSpriteType with the vanilla glow fields
    let approval = h.approve(&id, "animation_v3")["approval_id"].as_str().unwrap().to_string();
    h.install(&id, &approval, json!({"mode": "register"})).unwrap();
    let gfx = String::from_utf8(h.read("interface/iron_pact_animated.gfx")).unwrap();
    for line in [
        "frameAnimatedSpriteType = {",
        "name = \"GFX_glow_alert\"",
        "texturefile = \"gfx/interface/animated/iron_pact/glows/glow_alert.dds\"",
        "noOfFrames = 2",
        "animation_rate_fps = 1",
        "looping = yes",
        "play_on_show = yes",
        "pause_on_loop = 0.0",
        "alwaystransparent = yes",
        "effectFile = \"gfx/FX/buttonstate_blendframes.lua\"",
    ] {
        assert!(gfx.contains(line), "{line} missing:\n{gfx}");
    }
    let info = dds::parse_header(&h.read("gfx/interface/animated/iron_pact/glows/glow_alert.dds")).unwrap();
    assert_eq!((info.width, info.height), (120, 60));
}

#[test]
fn a_spin_turns_the_base_a_full_circle_in_twenty_frames() {
    let h = openai_harness();
    let (_, cand) = generate(&h, "anim_spin", "cog", &cog(), "k-spin-1", None);
    let p = process(&h, &cand, json!({})).unwrap();
    assert_eq!(names(&p, "steps").last().map(String::as_str), Some("spin_frames"));
    assert_eq!(p["size"], json!([520, 26]));
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let v = h.validate(&id);
    assert_eq!(v["status"], "pass", "{v:#}");
    let pv = h.preview(&id);
    assert_eq!(pv["animation"]["loop_gif_pictures"], 20);
    let approval = h.approve(&id, "animation_v3")["approval_id"].as_str().unwrap().to_string();
    h.install(&id, &approval, json!({"mode": "register"})).unwrap();
    let gfx = String::from_utf8(h.read("interface/iron_pact_animated.gfx")).unwrap();
    assert!(gfx.contains("noOfFrames = 20") && gfx.contains("animation_rate_fps = 7") && gfx.contains("loadType = \"INGAME\""), "{gfx}");
    assert!(!gfx.contains("effectFile"), "a frame strip plays without a blend effect");
}

#[test]
fn a_scroll_is_made_seamless_and_moves_one_width_per_loop() {
    let h = openai_harness();
    let (_, cand) = generate(&h, "anim_scroll", "research", &stripes(), "k-scroll-1", None);
    let p = process(&h, &cand, json!({})).unwrap();
    assert_eq!(names(&p, "steps"), vec!["flatten_on", "cover_resize", "scroll_frames"]);
    assert_eq!(p["size"], json!([648, 72]));
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let v = h.validate(&id);
    assert_eq!(v["status"], "pass", "{v:#}");
    let m = &h.preview(&id)["animation"]["metrics"];
    let seam = m["seam_ratio"].as_f64().unwrap();
    assert!((0.5..1.6).contains(&seam), "{m:#}");
}

#[test]
fn key_frames_are_edits_of_the_base_fitted_with_one_crop() {
    let h = openai_harness();
    let (_, base) = generate(&h, "anim_blend_keyframes", "beacon", &synthetic_icon_png(), "k-key-1", None);
    // without the edits the strip cannot be made, and the refusal says how to make them
    let e = process(&h, &base, json!({})).unwrap_err();
    assert_eq!(e.code, "key_frames_needed");
    assert!(e.message.contains("hoi4.image_edit") && e.message.contains("frame_candidates"), "{}", e.message);
    let mut frames = Vec::new();
    for (i, shift) in [8u8, 16].into_iter().enumerate() {
        h.queue_edit(&edited(shift));
        let out = h
            .engine
            .edit(
                serde_json::from_value(json!({
                    "candidate_id": base, "instruction": "the same beacon, its light brighter", "idempotency_key": format!("k-key-edit-{i}"), "quality": "low"
                }))
                .unwrap(),
            )
            .unwrap();
        frames.push(out["candidates"][0]["candidate_id"].as_str().unwrap().to_string());
    }
    let p = process(&h, &base, json!({"frame_candidates": frames})).unwrap();
    assert_eq!(names(&p, "finishers"), vec!["key_frames"]);
    assert_eq!(p["size"], json!([423, 115]));
    let strip = png_of(&h, p["path"].as_str().unwrap());
    let f = strip.split_frames(3).unwrap();
    assert_eq!(f[0].alpha_bbox(16), f[2].alpha_bbox(16), "one crop and scale for every frame");
    assert_ne!(f[0], f[2]);
    let id = p["artifact_id"].as_str().unwrap().to_string();
    let kv = h.validate(&id);
    assert_eq!(kv["status"], "pass", "{kv:#}");
    // only this asset's raw candidates can be frames, and only key-frame strips take them
    let (_, other) = generate(&h, "anim_blend_keyframes", "lamp", &synthetic_icon_png(), "k-key-2", None);
    let e = process(&h, &base, json!({"frame_candidates": [frames[0], other]})).unwrap_err();
    assert!(e.message.contains("another raw candidate"), "{}", e.message);
    let (_, cog_cand) = generate(&h, "anim_spin", "cog", &cog(), "k-key-3", None);
    assert!(process(&h, &cog_cand, json!({"frame_candidates": frames})).is_err());
}

#[test]
fn the_animation_check_fails_a_still_strip_and_a_frame_that_jumps() {
    let h = openai_harness();
    let disc = |cx: f64| {
        let mut img = Rgba::new(26, 26);
        for y in 0..26u32 {
            for x in 0..26u32 {
                if ((f64::from(x) + 0.5 - cx).powi(2) + (f64::from(y) + 0.5 - 13.0).powi(2)).sqrt() < 8.0 {
                    img.set_px(x, y, [200, 120, 40, 255]);
                }
            }
        }
        img
    };
    let import = |name: &str, frames: Vec<Rgba>| -> Value {
        let strip = Rgba::join_frames(&frames).unwrap();
        let rel = format!("incoming/{name}.png");
        let path = h.root().join(&rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, encode_png(&strip).unwrap()).unwrap();
        let p = h
            .engine
            .process(
                serde_json::from_value(json!({"import": {"path": rel, "family": "anim_spin", "feature_slug": "iron_pact", "asset": name}})).unwrap(),
            )
            .unwrap();
        assert_eq!(names(&p, "steps"), vec!["conform"], "a finished strip is taken whole");
        h.validate(p["artifact_id"].as_str().unwrap())
    };
    let still = import("still", vec![disc(13.0); 20]);
    assert_eq!(still["status"], "fail");
    assert!(codes(&still).contains(&"GFX-ANIM-STATIC".to_string()), "{still:#}");
    let mut jumpy: Vec<Rgba> = (0..20).map(|i| disc(13.0 + f64::from(i % 2) * 0.4)).collect();
    jumpy[10] = disc(19.0);
    let v = import("jumpy", jumpy);
    let c = codes(&v);
    assert!(c.contains(&"GFX-ANIM-ALIGN".to_string()), "{v:#}");
    assert!(c.contains(&"GFX-ANIM-JUMP".to_string()), "{v:#}");
}
