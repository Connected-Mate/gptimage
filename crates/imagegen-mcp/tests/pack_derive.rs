//! `pack derive`: a style pack made from a mod's own art in one step.
//!
//! SYNTHETIC: the "game" and the "mod" are folders the test draws: vanilla focus icons are warm discs on a 100x88
//! canvas, and the mod's are three groups of plates (teal, red and blue) on its own 170x150 canvas, with near-copies
//! of vanilla, a placeholder, a flat file and a duplicate mixed in. No game file and no file of any real mod is read.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use imagegen_core::config::MapEnv;
use imagegen_core::hashing::FixedClock;
use imagegen_core::pixels::{encode_png, Rgba};
use imagegen_core::refselect::Rng;
use imagegen_mcp::engine::derive::DeriveOptions;
use imagegen_mcp::{Engine, RunMode};
use imagegen_providers::transport::FixtureTransport;
use serde_json::{json, Value};

struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
    engine: Engine,
}

fn write(path: &Path, img: &Rgba) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, encode_png(img).unwrap()).unwrap();
}

/// A warm painted disc, vanilla's focus look, with its centre and size moved by `i`.
fn vanilla_icon(i: u32, w: u32, h: u32) -> Rgba {
    let mut img = Rgba::new(w, h);
    let (cx, cy) = (f64::from(w) * (0.4 + 0.02 * f64::from(i % 10)), f64::from(h) * (0.45 + 0.01 * f64::from(i % 7)));
    let r = f64::from(h) * (0.25 + 0.01 * f64::from(i % 9));
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (f64::from(x) + 0.5 - cx, f64::from(y) + 0.5 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            if d < r {
                let shade = (160.0 + 60.0 * (dx / r)) as u8;
                img.set_px(x, y, [shade, (f64::from(shade) * 0.7) as u8, 40, 255]);
            }
        }
    }
    img
}

/// The mod's plate: a flat colour of its group with a random block pattern, so files of a group differ in their
/// dHash but share their palette.
fn plate(group: usize, seed: u64) -> Rgba {
    let colours = [[60, 200, 190], [220, 40, 50], [50, 80, 230]];
    let c = colours[group];
    let mut rng = Rng::new(seed);
    let mut img = Rgba::new(170, 150);
    let blocks: Vec<bool> = (0..64).map(|_| rng.next_u64().is_multiple_of(2)).collect();
    for y in 10..140u32 {
        for x in 10..160u32 {
            let on = blocks[((y - 10) * 8 / 130 * 8 + (x - 10) * 8 / 150) as usize];
            let k = if on { 1.0 } else { 0.35 };
            img.set_px(x, y, [(f64::from(c[0]) * k) as u8, (f64::from(c[1]) * k) as u8, (f64::from(c[2]) * k) as u8, 255]);
        }
    }
    img
}

fn world() -> World {
    let dir = tempfile::Builder::new().prefix("d").tempdir_in(common::repo_work_dir()).unwrap();
    let root = dir.path().to_path_buf();
    // the installed game: 30 vanilla focus icons and 4 decision icons
    for i in 0..30 {
        write(&root.join(format!("game/gfx/interface/goals/goal_v{i}.png")), &vanilla_icon(i, 100, 88));
    }
    for i in 0..4 {
        write(&root.join(format!("game/gfx/interface/decisions/decision_v{i}.png")), &vanilla_icon(i, 32, 32));
    }
    // the other mod: 36 plates in three groups spread over six country folders, on its own canvas
    let goals = root.join("other/gfx/interface/goals");
    for i in 0..36u32 {
        write(&goals.join(format!("T{}/g{}_{i}.png", i % 6, i % 3)), &plate((i % 3) as usize, u64::from(i) * 7919 + 1));
    }
    // near-copies of vanilla at the mod's canvas, a placeholder, a flat file, a duplicate and vanilla-sized files
    for i in 0..6 {
        write(&goals.join(format!("COPY/copy_{i}.png")), &vanilla_icon(i, 170, 150));
    }
    write(&goals.join("T0/focus_placeholder.png"), &plate(0, 999));
    write(&goals.join("T1/flat.png"), &Rgba::filled(170, 150, [128, 128, 128, 255]));
    std::fs::copy(goals.join("T0/g0_0.png"), goals.join("T5/g0_dup.png")).unwrap();
    for i in 0..3 {
        let small = imagegen_core::resize::resize_area(&plate(1, 500 + i), 100, 88).unwrap();
        write(&goals.join(format!("OLD/old_{i}.png")), &small);
    }
    // three decision icons only: too few for a house style
    for i in 0..3 {
        write(&root.join(format!("other/gfx/interface/decisions/d{i}.png")), &imagegen_core::resize::resize_area(&plate(2, 40 + i), 32, 32).unwrap());
    }
    std::fs::write(root.join("other/descriptor.mod"), "version=\"1\"\nname=\"Synthetic Other Mod\"\n").unwrap();
    std::fs::create_dir_all(root.join("mod")).unwrap();
    let pairs = [
        ("HOI4_GAME_DIR", root.join("game").display().to_string()),
        ("HOI4_IMAGEGEN_CACHE_DIR", root.join("cache").display().to_string()),
        ("HOI4_IMAGEGEN_CHATGPT_AUTH", root.join("auth.json").display().to_string()),
    ];
    let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let engine = Engine::open(
        &root.join("mod"),
        Arc::new(MapEnv::from_pairs(&pairs)),
        Arc::new(FixtureTransport::new()),
        Arc::new(FixedClock(1_793_404_800)),
        RunMode::Cli,
    )
    .unwrap();
    World { _dir: dir, root, engine }
}

fn options(w: &World) -> DeriveOptions {
    DeriveOptions {
        from_mod: Some(w.root.join("other")),
        families: vec!["focus".parse().unwrap(), "decision".parse().unwrap()],
        ..Default::default()
    }
}

fn picks(record: &Value) -> Vec<String> {
    record["families"]["focus"]["picks"].as_array().unwrap().iter().map(|p| p["source"].as_str().unwrap().to_string()).collect()
}

fn record(w: &World) -> Value {
    serde_json::from_slice(&std::fs::read(w.root.join("mod/.hoi4-imagegen/packs/other/derive.json")).unwrap()).unwrap()
}

#[test]
fn derive_picks_a_diverse_set_of_the_mods_own_art_and_saves_a_pack() {
    let w = world();
    let v = w.engine.derive_pack("other", &options(&w)).unwrap();
    assert_eq!(v["from_mod"], "Synthetic Other Mod", "the descriptor's name, not the private path");
    assert_eq!(v["validation"]["ok"], true, "{}", v["validation"]);
    let focus = &v["families"]["focus"];
    assert_eq!(focus["picks"], 8);
    assert_eq!(focus["canvas"], json!([170, 150]), "the mod's own canvas, not vanilla's 100x88");
    assert!(v["skipped"]["decision"].as_str().unwrap().contains("only 3 files"), "{}", v["skipped"]);

    let r = record(&w);
    let f = &r["families"]["focus"];
    assert_eq!(f["near_copies_of_vanilla"], 6, "{f}");
    assert_eq!(f["exact_duplicates"], 1);
    assert_eq!(f["flat_or_empty"], 1);
    assert_eq!(f["filtered_out"]["placeholder_name"], 1);
    assert_eq!(f["filtered_out"]["other_size_cluster"], 3, "the vanilla-sized files are another size cluster");
    let p = picks(&r);
    assert!(p.iter().all(|s| !s.contains("COPY") && !s.contains("placeholder") && !s.contains("flat") && !s.contains("dup")), "{p:?}");
    for g in ["/g0_", "/g1_", "/g2_"] {
        assert!(p.iter().filter(|s| s.contains(g)).count() >= 2, "every style group is picked from: {p:?}");
    }
    for t in 0..6 {
        assert!(p.iter().filter(|s| s.contains(&format!("/T{t}/"))).count() <= 2, "at most two from one folder: {p:?}");
    }
    assert!(f["look"].as_str().unwrap().contains("170x150"), "{}", f["look"]);
    assert!(f["alternates"].as_array().unwrap().len() >= 8);

    // the pack: references copied into the mod as PNG and pinned, the canvas and the look written
    let pack = imagegen_core::pack::load(&imagegen_core::fsio::ProjectFs::new(&w.root.join("mod")).unwrap(), "other").unwrap();
    let e = &pack.families[&imagegen_core::family::Family::Focus];
    assert_eq!(e.canvas, Some([170, 150]));
    assert_eq!(e.references.len(), 8);
    assert!(e.references.iter().all(|r| r.path.starts_with(".hoi4-imagegen/packs/other/focus/") && r.note.contains("Synthetic Other Mod")));
    assert!(!pack.families.contains_key(&imagegen_core::family::Family::Decision));

    // the contact sheet holds vanilla art: it goes to the cache folder, never into the mod
    let sheet = PathBuf::from(v["families"]["focus"]["sheet"].as_str().unwrap());
    assert!(sheet.starts_with(w.root.join("cache")) && sheet.is_file(), "{}", sheet.display());
    let in_pack: Vec<String> =
        walk(&w.root.join("mod/.hoi4-imagegen/packs/other")).into_iter().filter(|p| p.ends_with(".png") && !p.contains("_focus_ref_")).collect();
    assert!(in_pack.is_empty(), "only the references are images in the mod: {in_pack:?}");

    // the same input gives the same picks
    let again = w.engine.derive_pack("other", &DeriveOptions { replace: true, ..options(&w) }).unwrap();
    assert_eq!(again["families"]["focus"]["picks"], 8);
    assert_eq!(picks(&record(&w)), p);
}

fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path().display().to_string().replace('\\', "/"));
        }
    }
    out
}

#[test]
fn review_swaps_and_drops_picks_and_an_existing_pack_needs_replace() {
    let w = world();
    w.engine.derive_pack("other", &options(&w)).unwrap();
    let first = record(&w);
    let a1 = first["families"]["focus"]["alternates"][0]["source"].as_str().unwrap().to_string();
    let err = w.engine.derive_pack("other", &options(&w)).unwrap_err();
    assert_eq!(err.code, "exists");
    let o = DeriveOptions { replace: true, swap: vec!["focus:1=a1".into()], drop: vec!["focus:8".into()], ..options(&w) };
    let v = w.engine.derive_pack("other", &o).unwrap();
    assert_eq!(v["families"]["focus"]["picks"], 7);
    let p = picks(&record(&w));
    assert_eq!(p[0], a1, "alternate 1 took the place of pick 1");
    assert_eq!(&p[1..], &picks(&first)[1..7]);
    // a swap by path, and edits the run cannot apply
    let by_path = DeriveOptions { replace: true, swap: vec![format!("focus:2={}", picks(&first)[7])], ..options(&w) };
    w.engine.derive_pack("other", &by_path).unwrap();
    assert_eq!(picks(&record(&w))[1], picks(&first)[7]);
    for bad in ["focus:9=a1", "focus:1=a99", "decision:1=a1", "focus:1=gfx/nowhere.png"] {
        let o = DeriveOptions { replace: true, swap: vec![bad.into()], ..options(&w) };
        assert!(w.engine.derive_pack("other", &o).is_err(), "{bad}");
    }
}

#[test]
fn the_mcp_action_derives_and_refuses_without_the_game_or_a_mod_folder() {
    let w = world();
    let params =
        json!({"action": "pack_derive", "name": "other", "from_mod": w.root.join("other").display().to_string(), "families": ["focus"], "picks": 4});
    let v = w.engine.references(serde_json::from_value(params).unwrap()).unwrap();
    assert_eq!(v["families"]["focus"]["picks"], 4);
    let params = json!({"action": "pack_derive", "name": "x", "from_mod": w.root.join("nowhere").display().to_string()});
    assert_eq!(w.engine.references(serde_json::from_value(params).unwrap()).unwrap_err().code, "not_found");
    let h = common::harness("", &[]);
    let err = h.engine.derive_pack("x", &DeriveOptions { from_mod: Some(w.root.join("other")), ..Default::default() }).unwrap_err();
    assert_eq!(err.code, "game_not_found", "the near-copy filter needs vanilla's art");
}
