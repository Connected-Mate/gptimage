//! `hoi4.image_set`: every icon of one focus tree file as one set.
//!
//! SYNTHETIC: the focus tree, its localisation, its `.gfx` file and its one texture are written by the test. Nothing
//! is generated: plans are free, and no game folder is read.

mod common;

use common::{openai_harness, Harness};
use imagegen_core::pixels::{encode_png, Rgba};
use serde_json::{json, Value};

const TREE: &str = r#"focus_tree = {
	id = test_tree
	focus = {
		id = TST_steel
		icon = GFX_focus_TST_steel
		x = 1
		y = 0
		cost = 10
	}
	focus = {
		id = TST_more_steel
		icon = GFX_focus_TST_steel
		x = 2
		y = 0
		cost = 10
	}
	focus = {
		id = TST_navy
		icon = GFX_goal_tst_navy
		x = 3
		y = 0
		cost = 10
	}
	focus = {
		id = TST_old
		icon = GFX_focus_TST_old
		x = 4
		y = 0
		cost = 10
	}
}
"#;

fn write(h: &Harness, rel: &str, bytes: &[u8]) {
    let p = h.root().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn tree(h: &Harness) {
    write(h, "common/national_focus/test_tree.txt", TREE.as_bytes());
    write(
        h,
        "localisation/english/test_l_english.yml",
        "\u{feff}l_english:\n TST_steel:0 \"Steel for the Fleet\"\n TST_steel_desc:0 \"§YMore§! steel [ROOT.GetName] mills.\"\n TST_navy:0 \"A Blue-Water Navy\"\n"
            .as_bytes(),
    );
    // one icon already exists: its sprite is defined and its texture is there
    write(
        h,
        "interface/test_goals.gfx",
        b"spriteTypes = {\n\tspriteType = {\n\t\tname = \"GFX_focus_TST_old\"\n\t\ttexturefile = \"gfx/interface/goals/tst_old.png\"\n\t}\n}\n",
    );
    write(h, "gfx/interface/goals/tst_old.png", &encode_png(&Rgba::filled(100, 88, [90, 80, 60, 255])).unwrap());
}

fn set(h: &Harness, params: Value) -> Result<Value, imagegen_mcp::EngineError> {
    h.engine.set(serde_json::from_value(params).unwrap())
}

#[test]
fn a_focus_tree_is_planned_as_one_set_without_duplicates() {
    let h = openai_harness();
    tree(&h);
    let v = set(&h, json!({"focus_tree": "common/national_focus/test_tree.txt", "quality": "low"})).unwrap();
    assert_eq!(v["feature_slug"], "test_tree");
    assert_eq!(v["icons_in_tree"], 3, "three distinct sprites: {v:#}");
    assert_eq!(v["skipped_present"], 1, "the icon with a sprite and a texture is not planned again");
    assert_eq!(v["planned"], 2);
    let icons = v["icons"].as_array().unwrap();
    let steel = icons.iter().find(|i| i["sprite"] == "GFX_focus_TST_steel").unwrap();
    assert_eq!(steel["asset"], "tst_steel");
    assert_eq!(steel["focuses"], json!(["TST_steel", "TST_more_steel"]), "two focuses share one asset");
    assert_eq!(steel["installs_as"], "GFX_focus_tst_steel");
    let navy = icons.iter().find(|i| i["sprite"] == "GFX_goal_tst_navy").unwrap();
    assert_eq!(navy["asset"], "tst_navy");
    assert_eq!(navy["tree_names_it"], false, "a goal_ sprite is installed under the focus name, and the reply says so");
    assert_eq!(v["shared_sprites"][0]["focuses"], json!(["TST_steel", "TST_more_steel"]));
    // the brief names the focus from the localisation, without its formatting codes
    let brief = std::fs::read_to_string(h.root().join(format!(".hoi4-imagegen/plans/{}.json", steel["plan_id"].as_str().unwrap()))).unwrap();
    assert!(brief.contains("Steel for the Fleet") && brief.contains("More steel mills.") && brief.contains("One of 2 icons"), "{brief}");
    // every icon of the tree on request
    let all = set(&h, json!({"focus_tree": "common/national_focus/test_tree.txt", "only_missing": false})).unwrap();
    assert_eq!(all["planned"], 3);
}

#[test]
fn set_refuses_what_it_cannot_do() {
    let h = openai_harness();
    tree(&h);
    let e = set(&h, json!({"focus_tree": "common/national_focus/nope.txt"})).unwrap_err();
    assert_eq!(e.code, "not_found");
    let e = set(&h, json!({"focus_tree": "common/national_focus/test_tree.txt", "generate": true})).unwrap_err();
    assert!(e.message.contains("idempotency_key"), "{e}");
    let e = set(&h, json!({"focus_tree": "../outside.txt"})).unwrap_err();
    assert_ne!(e.code, "ok");
}
