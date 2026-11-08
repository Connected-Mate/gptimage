//! The vanilla form of the families whose output drifted from it (agency_upgrade, building, commander_trait,
//! division_emblem, mio_trait, operation_icon, operation_picture, gui_texture): the plan prompt states it, and the
//! automatic vanilla references (tier 4) are vanilla files of that form.
//!
//! SYNTHETIC: the prompt tests plan against an empty project with no game, and nothing is generated. The reference
//! tests read the installed game (HOI4 1.19.4.1) and skip when it is absent. No game file is copied anywhere.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{openai_harness, temp_project};
use imagegen_core::config::MapEnv;
use imagegen_core::hashing::FixedClock;
use imagegen_core::stylesrc::header_size;
use imagegen_core::GuiTextureKind;
use imagegen_mcp::{Engine, RunMode};
use imagegen_providers::transport::FixtureTransport;
use serde_json::{json, Value};

fn request(family: &str, extra: Value) -> imagegen_core::plan::PlanRequest {
    let mut v = json!({
        "family": family, "feature_slug": "forms", "asset": "test_asset", "brief": "a test subject", "provider": "openai", "quality": "low"
    });
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    serde_json::from_value(v).unwrap()
}

/// Each reviewed family's prompt states its vanilla form, and the old wording that drew the wrong images is gone.
#[test]
fn each_reviewed_family_prompt_states_its_vanilla_form() {
    let h = openai_harness();
    let cases: [(&str, &[&str], &[&str]); 11] = [
        ("mio_department", &["department icon form: ONE piece of equipment", "never grey, metallic"], &["Simple flat shading only"]),
        ("mio_policy", &["policy icon form: one object", "view into the distance"], &["One centred subject"]),
        ("bop_side", &["balance of power side form: one person", "head-and-shoulders portrait"], &["One centred subject"]),
        ("state_modifier", &["state modifier icon form: one bold, simple object", "no small details"], &["One centred subject"]),
        (
            "agency_upgrade",
            &["intelligence agency upgrade form: a round painted vignette", "no ring, rim, frame or border around the disc"],
            &["One centred subject", "dark circular frame"],
        ),
        (
            "building",
            &["landmark building form: one real-world landmark", "no ground, rock or hill", "Frame 2", "orange flame", "diorama base"],
            &["the tool adds the second frame (its highlight"],
        ),
        ("commander_trait", &["unit leader trait form: a small military medal", "monochrome metal", "natural colours"], &["One centred subject"]),
        ("division_emblem", &["division template symbol form", "solid black silhouette"], &[]),
        ("mio_trait", &["trait icon form: one small, simple glyph", "round badge, disc or plate behind the object"], &["small round coloured badge"]),
        (
            "operation_icon",
            &["operation phase icon form: a round period photograph", "monochrome sepia", "This family has only this form", "operation_picture"],
            &["One centred subject"],
        ),
        (
            "operation_picture",
            &["operation phase picture form: one period photograph", "This family has only this form", "operation_icon", "round vignette"],
            &["Period illustration look, painterly"],
        ),
    ];
    for (family, has, never) in cases {
        let plan = h.engine.plan(request(family, json!({}))).unwrap_or_else(|e| panic!("{family}: {e:?}"));
        let text = &plan.prompt.text;
        for s in has {
            assert!(text.contains(s), "{family}: '{s}' missing from\n{text}");
        }
        for s in never {
            assert!(!text.contains(s), "{family}: '{s}' still in\n{text}");
        }
        eprintln!("=== {family}\n{text}");
    }
}

/// A GUI texture's kind adds the kind's vanilla form to the prompt without changing the profile, and only a GUI
/// texture takes a kind.
#[test]
fn gui_texture_kind_adds_its_form_and_is_refused_elsewhere() {
    let h = openai_harness();
    let plain = h.engine.plan(request("gui_texture", json!({"canvas": [300, 200]}))).unwrap();
    assert!(plain.prompt.text.contains("GUI texture form: flat interface material"), "{}", plain.prompt.text);
    for kind in GuiTextureKind::ALL {
        let plan = h.engine.plan(request("gui_texture", json!({"canvas": [300, 200], "texture_kind": kind}))).unwrap();
        let rule = kind.form_rule().replace("{w}", "300").replace("{h}", "200");
        assert!(plan.prompt.text.contains(&rule), "{kind}:\n{}", plan.prompt.text);
        assert_eq!(plan.profile.hash, plain.profile.hash, "{kind}: the kind changes the prompt, not the profile");
        assert_eq!(plan.request.texture_kind, Some(kind));
    }
    let err = h.engine.plan(request("focus", json!({"texture_kind": "button"}))).unwrap_err();
    assert!(err.message.contains("texture_kind is for gui_texture"), "{}", err.message);
    // a plan without a kind serialises without the field, so plans made before it existed keep their hash
    assert!(serde_json::to_value(&plain.request).unwrap().get("texture_kind").is_none());
}

fn real_game() -> Option<PathBuf> {
    let p = std::env::var("HOI4_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV"));
    p.join("gfx/interface").is_dir().then_some(p)
}

fn game_engine(game: &std::path::Path) -> (tempfile::TempDir, Engine) {
    let empty = temp_project("");
    let auth = empty.path().join("chatgpt-auth.json").display().to_string();
    let env = MapEnv::from_pairs(&[
        ("OPENAI_API_KEY", "sk-test-synthetic-000"),
        ("HOI4_GAME_DIR", &game.display().to_string()),
        ("HOI4_IMAGEGEN_CHATGPT_AUTH", &auth),
    ]);
    let engine =
        Engine::open(empty.path(), Arc::new(env), Arc::new(FixtureTransport::new()), Arc::new(FixedClock(1_793_404_800)), RunMode::Cli).unwrap();
    (empty, engine)
}

/// Every reference of the plan's style sources (sent or statistics only), as (path, sent).
fn tier4_refs(plan: &imagegen_core::plan::Plan) -> Vec<(String, bool)> {
    let s = plan.style_sources.as_ref().expect("a plan reports its style sources");
    assert_eq!(s.chosen_tier, Some(4), "an empty mod takes vanilla references: {s:#?}");
    s.chosen.iter().map(|c| (c.path.replace('\\', "/"), c.sent_to_provider)).collect()
}

/// The tier-4 references of each reviewed family come from the vanilla folder, names and size of its one form.
/// Skipped without the installed game.
#[test]
fn tier4_references_are_vanilla_files_of_the_family_form() {
    let Some(game) = real_game() else {
        eprintln!("SKIPPED: tier-4 references vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
        return;
    };
    let (_dir, engine) = game_engine(&game);
    // (family, folder every reference is in, a name rule, the size)
    type Rule = fn(&str) -> bool;
    let cases: [(&str, &str, Rule, [u32; 2]); 11] = [
        ("mio_department", "/gfx/interface/military_industrial_organization/department_icons/", |_| true, [48, 48]),
        ("mio_policy", "/gfx/interface/military_industrial_organization/policy_icons/", |_| true, [60, 60]),
        ("bop_side", "/gfx/interface/bop/", |n| !n.contains("_improved") && !n.contains("_max."), [72, 88]),
        ("state_modifier", "/gfx/interface/state_modifiers/", |_| true, [32, 32]),
        ("agency_upgrade", "/gfx/interface/operatives/icons/", |n| n.starts_with("agency_"), [68, 68]),
        ("building", "/gfx/interface/buildings/", |_| true, [128, 64]),
        ("commander_trait", "/gfx/interface/traits/", |n| n.starts_with("trait_") && !n.starts_with("trait_operative_"), [23, 33]),
        ("division_emblem", "/gfx/interface/counters/division_templates_large/", |_| true, [76, 42]),
        ("mio_trait", "/gfx/interface/military_industrial_organization/trait_icons/", |_| true, [48, 48]),
        ("operation_icon", "/gfx/interface/operations/phases_small/", |n| n.starts_with("phase_") && n.ends_with("_small.dds"), [59, 58]),
        ("operation_picture", "/gfx/interface/operations/images/", |n| n.starts_with("phase_") && !n.contains("_small"), [210, 176]),
    ];
    for (family, folder, name_ok, size) in cases {
        let plan = engine.plan(request(family, json!({}))).unwrap_or_else(|e| panic!("{family}: {e:?}"));
        let refs = tier4_refs(&plan);
        assert!(refs.iter().filter(|(_, sent)| *sent).count() >= 3, "{family}: {refs:?}");
        for (path, _) in &refs {
            let name = path.rsplit('/').next().unwrap().to_ascii_lowercase();
            assert!(path.contains(folder), "{family}: {path} is not in {folder}");
            assert!(name_ok(&name), "{family}: {name}");
            assert_eq!(header_size(std::path::Path::new(path)), Some(size), "{family}: {path}");
        }
        eprintln!("{family}: {}", refs.iter().filter(|(_, s)| *s).map(|(p, _)| p.rsplit('/').next().unwrap()).collect::<Vec<_>>().join(", "));
    }
}

/// A GUI texture takes vanilla references of the kind it asks for, and none without a kind. Skipped without the game.
#[test]
fn gui_texture_takes_vanilla_references_by_kind() {
    let Some(game) = real_game() else {
        eprintln!("SKIPPED: GUI texture references vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
        return;
    };
    let (_dir, engine) = game_engine(&game);
    let none = engine.plan(request("gui_texture", json!({"canvas": [300, 200]}))).unwrap();
    let s = none.style_sources.as_ref().unwrap();
    assert_eq!(s.chosen_tier, None, "no kind, no automatic reference");
    assert!(s.tiers.iter().any(|t| t.note.contains("texture_kind")), "{s:#?}");
    for kind in GuiTextureKind::ALL {
        let plan = engine.plan(request("gui_texture", json!({"canvas": [300, 200], "texture_kind": kind}))).unwrap();
        let refs = tier4_refs(&plan);
        assert!(refs.iter().filter(|(_, sent)| *sent).count() >= 3, "{kind}: {refs:?}");
        for (path, _) in &refs {
            let (dir, name) = path.rsplit_once('/').unwrap();
            let name = name.to_ascii_lowercase();
            let stem = name.trim_end_matches(".dds");
            let ok = match kind {
                GuiTextureKind::Button => (dir.ends_with("/gfx/interface") && stem.starts_with("button_")) || dir.ends_with("/gfx/interface/buttons"),
                GuiTextureKind::Frame => dir.ends_with("/gfx/interface") && stem.contains("_frame") && !stem.contains("frames"),
                GuiTextureKind::Panel => dir.ends_with("/gfx/interface/tiles") && stem.starts_with("tiled_"),
                GuiTextureKind::Background => {
                    let s = header_size(std::path::Path::new(path)).unwrap();
                    dir.ends_with("/gfx/interface") && stem.ends_with("_bg") && s[0] >= 256 && s[1] >= 256
                }
            };
            assert!(ok, "{kind}: {path}");
        }
        eprintln!("{kind}: {}", refs.iter().filter(|(_, s)| *s).map(|(p, _)| p.rsplit('/').next().unwrap()).collect::<Vec<_>>().join(", "));
    }
}
