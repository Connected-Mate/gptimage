//! The family lock, the grouped tools and the hardened validation, seen through the engine.
//!
//! SYNTHETIC: provider replies come from the fixture transport and were written for the tests.

mod common;

use common::{openai_harness, synthetic_icon_png, Harness};
use imagegen_core::dds;
use imagegen_core::family::{Family, Group};
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba};
use imagegen_core::records::{kind, ValidatedRecord};
use imagegen_core::validate::check_set_hash;
use imagegen_mcp::api::{
    AnimationRequest, FlagRequest, IconRequest, InterfaceRequest, PictureRequest, PortraitRequest, ReviewParams, UnitRequest, ValidateParams,
};
use imagegen_mcp::EngineError;
use serde_json::{json, Value};

fn code(e: &EngineError) -> &str {
    &e.code
}

/// Plan through a group tool, generate through it with approve, and process the candidate.
fn make(h: &Harness, group_tool: &str, family: &str, asset: &str, key: &str) -> String {
    h.queue_generation(&synthetic_icon_png());
    let params = json!({"family": family, "feature_slug": "f", "asset": asset, "brief": "an iron wall", "provider": "openai", "quality": "low", "approve": true, "idempotency_key": key});
    let out = match group_tool {
        "icon" => {
            let (req, follow) = serde_json::from_value::<IconRequest>(params).unwrap().into_parts();
            h.engine.group_request(Group::Icon, req, follow)
        }
        "picture" => {
            let (req, follow) = serde_json::from_value::<PictureRequest>(params).unwrap().into_parts();
            h.engine.group_request(Group::Picture, req, follow)
        }
        other => panic!("{other}"),
    }
    .unwrap();
    let cand = out["generation"]["candidates"][0]["candidate_id"].as_str().unwrap().to_string();
    let proc = h.engine.process(serde_json::from_value(json!({"candidate_id": cand})).unwrap()).unwrap();
    proc["artifact_id"].as_str().unwrap().to_string()
}

fn approve_with(h: &Harness, id: &str, rubric: &str) -> String {
    assert_eq!(h.validate(id)["status"], "pass", "{:#}", h.validate(id));
    h.preview(id);
    h.approve(id, rubric)["approval_id"].as_str().unwrap().to_string()
}

fn write_import(h: &Harness, name: &str, img: &Rgba) -> String {
    std::fs::create_dir_all(h.root().join("incoming")).unwrap();
    let rel = format!("incoming/{name}");
    std::fs::write(h.root().join(&rel), encode_png(img).unwrap()).unwrap();
    rel
}

fn import(h: &Harness, path: &str, family: &str, asset: &str) -> Result<Value, EngineError> {
    h.engine.process(serde_json::from_value(json!({"import": {"path": path, "family": family, "feature_slug": "f", "asset": asset}})).unwrap())
}

fn blob(w: u32, h: u32) -> Rgba {
    let mut img = Rgba::new(w, h);
    for y in 3..h - 3 {
        for x in 3..w - 3 {
            img.set_px(x, y, [(x * 5 % 200) as u8 + 30, (y * 7 % 200) as u8 + 30, 90, 255]);
        }
    }
    img
}

#[test]
fn an_idea_artifact_cannot_be_validated_reviewed_or_installed_as_anything_else() {
    let h = openai_harness();
    let id = make(&h, "icon", "idea_spirit", "iron_will", "k-idea-1");
    assert!(id.starts_with("f.iron_will-v1-p"));

    // validate has a family argument for plain files only: naming another one for an artifact is refused
    let e = h.engine.validate(serde_json::from_value(json!({"targets": [id], "family": "focus"})).unwrap()).unwrap_err();
    assert_eq!(code(&e), "GFX-FAMILY-MISMATCH");
    // review and install have no family or profile parameter at all
    for extra in ["family", "profile", "category", "checks"] {
        let mut review = json!({"target": id, "verdict": "approved", "reviewer": "t"});
        review[extra] = json!("focus");
        assert!(serde_json::from_value::<ReviewParams>(review).is_err(), "review must not accept {extra}");
        let mut install = json!({"target": id, "approval_id": "apr-0000000000000000"});
        install[extra] = json!("focus");
        assert!(serde_json::from_value::<imagegen_mcp::api::InstallParams>(install).is_err(), "install must not accept {extra}");
        let mut process = json!({"candidate_id": id});
        process[extra] = json!("focus");
        assert!(serde_json::from_value::<imagegen_mcp::api::ProcessParams>(process).is_err(), "process must not accept {extra}");
    }
    // validating with the right family, or none, records the artifact's own family
    let v = h.validate(&id);
    assert_eq!(v["results"][0]["family"], "idea_spirit");
    assert_eq!(v["results"][0]["profile"]["id"], "idea_spirit");
    let ok = h.engine.validate(serde_json::from_value(json!({"targets": [id], "family": "idea_spirit"})).unwrap()).unwrap();
    assert_eq!(ok["status"], "pass");

    // everything downstream is the idea family's: rubric, paths and sprite names
    let approval = approve_with(&h, &id, "icon_v1");
    let out = h.install(&id, &approval, json!({})).unwrap();
    assert_eq!(out["final_files"][0]["path"], "gfx/interface/ideas/f/idea_iron_will.dds");
    assert_eq!(out["sprite"]["names"], json!(["GFX_idea_iron_will"]));
    let entries = h.engine.ledger.entries().unwrap();
    for e in entries.iter().filter(|e| ["validated", "reviewed", "installed", "processed", "generated"].contains(&e.kind.as_str())) {
        assert_eq!(e.data["family"], "idea_spirit", "{} record must carry the family: {}", e.kind, e.data);
    }
    assert!(h.exists(".hoi4-imagegen/records/f.iron_will.json"));
    assert_eq!(serde_json::from_slice::<Value>(&h.read(".hoi4-imagegen/records/f.iron_will.json")).unwrap()["family"], "idea_spirit");
}

#[test]
fn importing_an_advisor_sized_image_as_a_focus_icon_is_refused() {
    let h = openai_harness();
    let mut art = Rgba::new(65, 67);
    for y in 1..66 {
        for x in 1..64 {
            art.set_px(x, y, [200, 40, 40, 255]);
        }
    }
    let path = write_import(&h, "card.png", &art);
    let e = import(&h, &path, "focus", "fake").unwrap_err();
    assert_eq!(code(&e), "GFX-FAMILY-MISMATCH", "{e}");
    assert!(e.message.contains("advisor_card"), "{}", e.message);
    // nothing was recorded for the refused import
    assert!(h.engine.ledger.entries().unwrap().iter().all(|e| e.kind != kind::IMPORTED));
    // the same image is welcome as its own family
    assert!(import(&h, &path, "advisor_card", "card").is_ok());
}

#[test]
fn the_pixels_of_an_installed_idea_icon_cannot_become_a_decision_icon() {
    let h = openai_harness();
    let id = make(&h, "icon", "idea_spirit", "iron_will", "k-idea-2");
    let approval = approve_with(&h, &id, "icon_v1");
    h.install(&id, &approval, json!({})).unwrap();

    // the installed DDS itself, and the same pixels re-encoded as a PNG, are refused as a decision icon
    let installed = "gfx/interface/ideas/f/idea_iron_will.dds";
    let e = import(&h, installed, "decision", "borrowed").unwrap_err();
    assert_eq!(code(&e), "GFX-FAMILY-REUSE", "{e}");
    assert!(e.message.contains("idea_spirit"), "{}", e.message);
    let (img, _) = decode(&h.read(installed), Some(installed), DecodeLimits::default()).unwrap();
    let png = write_import(&h, "again.png", &img);
    assert_eq!(code(&import(&h, &png, "focus", "borrowed").unwrap_err()), "GFX-FAMILY-REUSE");
    // the same family may take its own pixels again
    assert!(import(&h, &png, "idea_spirit", "second_copy").is_ok());

    // same-size families are caught by pixels, not by size: a doctrine icon is not an idea concern
    let doctrine = blob(64, 64);
    let path = write_import(&h, "doctrine.png", &doctrine);
    let d = import(&h, &path, "doctrine", "tactics").unwrap();
    let did = d["artifact_id"].as_str().unwrap().to_string();
    let approval = approve_with(&h, &did, "icon_v1");
    h.install(&did, &approval, json!({})).unwrap();
    let e = import(&h, &path, "idea_concern", "same_pixels").unwrap_err();
    assert_eq!(code(&e), "GFX-FAMILY-REUSE");
    // the helper the audit packet will use sees the same thing
    let hash = imagegen_core::reuse::pixel_hash(&doctrine);
    let hits = imagegen_core::reuse::find_cross_family_reuse(&h.engine.fs, Family::IdeaConcern, &hash).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].family, Family::Doctrine);
}

#[test]
fn a_provider_that_echoes_another_familys_art_is_refused_before_it_is_stored() {
    let h = openai_harness();
    let id = make(&h, "icon", "idea_spirit", "iron_will", "k-idea-3");
    let approval = approve_with(&h, &id, "icon_v1");
    h.install(&id, &approval, json!({})).unwrap();
    // the provider now answers with exactly the installed idea pixels, for a decision icon
    let installed = "gfx/interface/ideas/f/idea_iron_will.dds";
    let (img, _) = decode(&h.read(installed), Some(installed), DecodeLimits::default()).unwrap();
    h.queue_generation(&encode_png(&img).unwrap());
    let (req, follow) = serde_json::from_value::<IconRequest>(
        json!({"family": "decision", "feature_slug": "f", "asset": "echo", "brief": "x", "provider": "openai", "quality": "low", "approve": true, "idempotency_key": "k-echo"}),
    )
    .unwrap()
    .into_parts();
    let e = h.engine.group_request(Group::Icon, req, follow).unwrap_err();
    assert_eq!(code(&e), "GFX-FAMILY-REUSE");
    assert!(!h.exists("docs/assets/f/source_png/echo-v1.png"));
}

#[test]
fn icons_cannot_be_enlarged_beyond_one_and_a_quarter_times() {
    let h = openai_harness();
    // a 40x40 icon fitted into 60x68 would be enlarged 1.45 times
    let mut small = Rgba::new(40, 40);
    for y in 2..38 {
        for x in 2..38 {
            small.set_px(x, y, [90, 120, 60, 255]);
        }
    }
    let path = write_import(&h, "small.png", &small);
    let e = import(&h, &path, "idea_spirit", "tiny").unwrap_err();
    assert_eq!(code(&e), "GFX-UPSCALE-LIMIT", "{e}");
    // a larger source is shrunk without complaint
    let big = blob(200, 200);
    let mut big_alpha = big.clone();
    big_alpha.set_px(0, 0, [0, 0, 0, 0]);
    let path = write_import(&h, "big.png", &big_alpha);
    assert!(import(&h, &path, "idea_spirit", "large").is_ok());
}

#[test]
fn opaque_families_never_keep_provider_alpha() {
    let h = openai_harness();
    // the synthetic provider image is transparent with a soft subject: a picture must come out fully opaque
    for (tool, family, asset) in [("picture", "news_event", "n1"), ("picture", "decision_category_picture", "d1")] {
        let id = make(&h, tool, family, asset, &format!("k-{asset}"));
        let artifact = h
            .engine
            .ledger
            .entries()
            .unwrap()
            .into_iter()
            .rev()
            .find(|e| e.kind == kind::PROCESSED && e.artifact_id.as_deref() == Some(id.as_str()))
            .unwrap();
        let path = artifact.data["path"].as_str().unwrap();
        let (img, _) = decode(&h.read(path), None, DecodeLimits::default()).unwrap();
        assert!(img.data.chunks_exact(4).all(|p| p[3] == 255), "{family} output has translucent pixels");
        assert!(artifact.data["steps"].as_array().unwrap().iter().any(|s| s["op"]["op"] == "flatten_on"), "{family}: no flatten step");
        let v = h.validate(&id);
        assert_eq!(v["status"], "pass", "{v:#}");
        assert!(v["results"][0]["checks"].as_array().unwrap().iter().any(|c| c == "opaque"));
    }
    // finished art imported at its own size gets flattened too, and a stray alpha pixel is gone
    let mut art = blob(82, 52);
    art.set_px(10, 10, [10, 10, 10, 128]);
    let path = write_import(&h, "flag.png", &art);
    let p = import(&h, &path, "flag", "IRP").unwrap();
    let (out, _) = decode(&h.read(p["path"].as_str().unwrap()), None, DecodeLimits::default()).unwrap();
    assert!(out.data.chunks_exact(4).all(|p| p[3] == 255));
    // a file that still has alpha fails the opaque check on its own
    let bytes = dds::write_a8r8g8b8(&{
        let mut x = Rgba::filled(397, 153, [80, 80, 80, 255]);
        x.set_px(5, 5, [80, 80, 80, 200]);
        x
    });
    std::fs::create_dir_all(h.root().join("gfx/event_pictures/f")).unwrap();
    std::fs::write(h.root().join("gfx/event_pictures/f/news_event_x.dds"), bytes).unwrap();
    let v = h
        .engine
        .validate(serde_json::from_value(json!({"targets": ["gfx/event_pictures/f/news_event_x.dds"], "family": "news_event"})).unwrap())
        .unwrap();
    let codes: Vec<&str> = v["results"][0]["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"GFX-ALPHA-IN-OPAQUE"), "{codes:?}");
}

#[test]
fn validation_cannot_skip_checks_and_the_gate_notices_a_short_check_set() {
    let h = openai_harness();
    // there is no checks parameter any more
    assert!(serde_json::from_value::<ValidateParams>(json!({"targets": ["x"], "checks": []})).is_err());
    assert!(serde_json::from_value::<ValidateParams>(json!({"targets": ["x"], "profile": "focus"})).is_err());

    let id = make(&h, "icon", "decision", "rally", "k-dec-1");
    let approval = approve_with(&h, &id, "icon_v1");
    // the recorded validation carries the hash of the family's full check set
    let want = check_set_hash(&imagegen_core::profile::ProfileSet::builtin().for_family(Family::Decision).unwrap().imagegen.checks);
    let recorded: ValidatedRecord =
        serde_json::from_value(h.engine.ledger.entries().unwrap().into_iter().rev().find(|e| e.kind == kind::VALIDATED).unwrap().data).unwrap();
    assert_eq!(recorded.check_set_hash, want);
    assert_eq!(recorded.family, Family::Decision);

    // a later validation record that ran only one check (written the way an old or hostile client might) is refused
    let mut short = recorded.clone();
    short.checks = vec!["size".into()];
    short.check_set_hash = check_set_hash(&short.checks);
    h.engine.ledger.append(kind::VALIDATED, Some(&recorded.target), Some(&id), &short).unwrap();
    let e = h.install(&id, &approval, json!({})).unwrap_err();
    assert_eq!(code(&e), "install_gate_checks_incomplete", "{e}");
    // and review will not approve on top of it either
    let answers: serde_json::Map<String, Value> =
        imagegen_core::rubric::questions("icon_v1").iter().map(|(k, _)| (k.to_string(), json!(true))).collect();
    let e = h
        .engine
        .review(serde_json::from_value(json!({"target": id, "verdict": "approved", "reviewer": "t", "answers": answers})).unwrap())
        .unwrap_err();
    assert_eq!(e.code, "review_needs_validation");
    // validating again restores a full record and the install goes through (the approval is still the same file's)
    assert_eq!(h.validate(&id)["status"], "pass");
    assert!(h.install(&id, &approval, json!({})).is_ok());
}

#[test]
fn every_group_tool_refuses_a_family_of_another_group_at_runtime() {
    let h = openai_harness();
    let mut tried = 0;
    for tool_group in Group::ALL {
        for family in Family::ALL.iter().copied().filter(|f| f.group() != tool_group) {
            let params = json!({"family": family.as_str(), "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai"});
            let (req, follow) = match tool_group {
                Group::Icon => serde_json::from_value::<IconRequest>(params).unwrap().into_parts(),
                Group::Picture => serde_json::from_value::<PictureRequest>(params).unwrap().into_parts(),
                Group::Portrait => serde_json::from_value::<PortraitRequest>(params).unwrap().into_parts(),
                Group::Flag => serde_json::from_value::<FlagRequest>(params).unwrap().into_parts(),
                Group::Unit => serde_json::from_value::<UnitRequest>(params).unwrap().into_parts(),
                Group::Interface => serde_json::from_value::<InterfaceRequest>(params).unwrap().into_parts(),
                Group::Animation => serde_json::from_value::<AnimationRequest>(params).unwrap().into_parts(),
            };
            let e = h.engine.group_request(tool_group, req, follow).unwrap_err();
            assert_eq!(code(&e), "family_not_in_group", "{} through {}", family, tool_group.tool_name());
            assert!(e.message.contains(family.group().tool_name()), "{}", e.message);
            tried += 1;
        }
    }
    assert_eq!(tried, 7 * 44 - 44);
    // the right group works without spending
    let (req, follow) =
        serde_json::from_value::<IconRequest>(json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai"}))
            .unwrap()
            .into_parts();
    let out = h.engine.group_request(Group::Icon, req, follow).unwrap();
    assert_eq!(out["generated"], false);
    assert!(out["plan_id"].as_str().unwrap().starts_with("plan-"));
    assert!(h.fixture.requests().is_empty(), "a plan costs nothing");
    // approve without a key is a refused request, not a spend
    let (req, follow) = serde_json::from_value::<IconRequest>(
        json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "provider": "openai", "approve": true}),
    )
    .unwrap()
    .into_parts();
    assert_eq!(code(&h.engine.group_request(Group::Icon, req, follow).unwrap_err()), "invalid_argument");
}

#[test]
fn the_family_enum_of_each_tool_schema_lists_exactly_its_group() {
    fn family_enum(schema: schemars::Schema) -> Vec<String> {
        let v = serde_json::to_value(schema).unwrap();
        let fam = &v["properties"]["family"];
        let fam = if fam["enum"].is_array() { fam.clone() } else { panic!("family has no inline enum: {v}") };
        fam["enum"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
    }
    let cases = [
        (Group::Icon, family_enum(schemars::schema_for!(IconRequest))),
        (Group::Picture, family_enum(schemars::schema_for!(PictureRequest))),
        (Group::Portrait, family_enum(schemars::schema_for!(PortraitRequest))),
        (Group::Flag, family_enum(schemars::schema_for!(FlagRequest))),
        (Group::Unit, family_enum(schemars::schema_for!(UnitRequest))),
        (Group::Interface, family_enum(schemars::schema_for!(InterfaceRequest))),
        (Group::Animation, family_enum(schemars::schema_for!(AnimationRequest))),
    ];
    let mut all = Vec::new();
    for (g, listed) in cases {
        let want: Vec<String> = g.family_ids().into_iter().map(str::to_string).collect();
        assert_eq!(listed, want, "{}", g.tool_name());
        all.extend(listed);
    }
    all.sort();
    let mut every: Vec<String> = Family::ALL.iter().map(|f| f.as_str().to_string()).collect();
    every.sort();
    assert_eq!(all, every, "every family is in exactly one tool");
}

#[test]
fn the_families_tool_reports_measured_specs_and_style_sources() {
    let h = openai_harness();
    let all = h.engine.families_info(serde_json::from_value(json!({})).unwrap()).unwrap();
    assert_eq!(all["families"].as_array().unwrap().len(), 44);
    assert_eq!(all["groups"].as_array().unwrap().len(), 7);
    let one = h.engine.families_info(serde_json::from_value(json!({"family": "idea_spirit"})).unwrap()).unwrap();
    let f = &one["family"];
    assert_eq!(f["canvas"], json!([60, 68]));
    assert_eq!(f["group"], "icon");
    assert_eq!(f["tool"], "hoi4.image_icon");
    assert_eq!(f["measured"]["game_version"].as_str().unwrap().split(' ').next().unwrap(), "1.19.4.1");
    assert_eq!(f["measured"]["distribution"][0]["size"], json!([60, 68]));
    assert!(f["check_set_hash"].as_str().unwrap().len() == 64);
    assert_eq!(f["style_sources"]["reference_dirs"][0]["dir"], "references/idea_spirit");
    assert!(f["mod_override"]["overridden"].as_array().unwrap().is_empty());
    // a mod's own profile row shows up as the override
    let h2 =
        common::harness("[defaults]\nprovider = \"openai\"\n[profiles.focus]\ncanvas = [94, 86]\n", &[("OPENAI_API_KEY", "sk-test-synthetic-000")]);
    let f = h2.engine.families_info(serde_json::from_value(json!({"family": "focus"})).unwrap()).unwrap();
    assert_eq!(f["family"]["canvas"], json!([94, 86]));
    assert_eq!(f["family"]["mod_override"]["differs_from_vanilla"], true);
    assert_eq!(f["family"]["mod_override"]["vanilla_canvas"], json!([100, 88]));
}
