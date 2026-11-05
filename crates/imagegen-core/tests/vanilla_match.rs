//! The review's measured vanilla-match check against the installed game and the labelled gallery cases.
//!
//! Labels live in `tests/fixtures/vanilla_match/`: `reference_set/` holds hand-checked labels on our own generated art
//! (the hard requirements) and `secondary_set/` further hand-checked labels on 0.3.1 output (secondary data, checked
//! for agreement only). The vanilla side is read from the installed game and never copied. Without the game every
//! test prints `SKIPPED` and passes (set `HOI4_GAME_DIR` to point at it).
//!
//! Measured on HOI4 1.19.4.1 with `vanilla_match_v1`. Three of the six images labelled bad measure inside
//! vanilla's own spread on every feature, so no vanilla-calibrated check can refuse them without refusing a large
//! share of vanilla itself: `focus.v022.steel_gate` (a loose gate where vanilla has a plaque, but wide, convex and
//! symmetric enough to sit at vanilla's 60th to 98th percentile), `idea_spirit.v022.war_economy` and
//! `idea_concern.v022.ration_crisis`. They are listed in `KNOWN_GAPS`, printed every run and left to the v3 rubric
//! and the side-by-side preview. `special_project_icon.v031.regen` was first labelled good. The check refuses it,
//! because 46 percent of its box is opaque where vanilla's line art never exceeds 17, and it is labelled bad
//! (2026-11-05: too dense), so it is a bad case now.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use imagegen_core::family::Family;
use imagegen_core::pixels::{decode, DecodeLimits};
use imagegen_core::profile::ProfileSet;
use imagegen_core::vanillamatch::{self, VanillaSet};
use serde_json::Value;

/// Labelled cases the measurement cannot separate (see the module notes). Their outcome is printed, not asserted.
const KNOWN_GAPS: [&str; 3] = ["focus.v022.steel_gate.png", "idea_spirit.v022.war_economy.png", "idea_concern.v022.ration_crisis.png"];

/// Lowest vanilla pass rate accepted for a family's own files.
const MIN_VANILLA_PASS: f64 = 0.97;

/// Fewest secondary-set verdicts the check must agree with (56 of 83 measured on 1.19.4.1, after eight labels were
/// corrected on a second look).
const MIN_SECONDARY_AGREEMENT: usize = 54;

fn find_game() -> Option<PathBuf> {
    let candidates = [
        std::env::var("HOI4_GAME_DIR").ok().map(PathBuf::from),
        Some(PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV")),
        Some(PathBuf::from("C:/Program Files/Steam/steamapps/common/Hearts of Iron IV")),
        std::env::var("HOME").ok().map(|h| PathBuf::from(h).join("Library/Application Support/Steam/steamapps/common/Hearts of Iron IV")),
    ];
    candidates.into_iter().flatten().find(|p| p.join("interface/subuniticons.gfx").is_file())
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vanilla_match")
}

fn family(id: &str) -> Family {
    serde_json::from_value(Value::String(id.to_string())).unwrap_or_else(|_| panic!("unknown family {id}"))
}

fn vanilla(game: &Path, f: Family) -> Option<VanillaSet> {
    let set = ProfileSet::builtin();
    let p = set.for_family(f)?;
    vanillamatch::load_vanilla(game, f, p.canvas, &p.accepted_strips())
}

fn check(set: &VanillaSet, file: &Path) -> vanillamatch::MatchResult {
    let bytes = std::fs::read(file).unwrap();
    let (img, _) = decode(&bytes, file.file_name().and_then(|n| n.to_str()), DecodeLimits::default()).unwrap();
    set.calibration.check(&vanillamatch::measure(&img).expect("measurable"))
}

fn labels(dir: &str) -> Vec<Value> {
    let text = std::fs::read_to_string(fixtures().join(dir).join("labels.json")).unwrap();
    serde_json::from_str::<Value>(&text).unwrap()["labels"].as_array().unwrap().clone()
}

#[test]
fn reference_labels_and_vanilla_pass_rates() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED vanilla match: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let labels = labels("reference_set");
    let mut sets: BTreeMap<Family, VanillaSet> = BTreeMap::new();
    let mut report = String::new();
    for l in &labels {
        let f = family(l["family"].as_str().unwrap());
        if let std::collections::btree_map::Entry::Vacant(slot) = sets.entry(f) {
            let set = vanilla(&game, f).unwrap_or_else(|| panic!("{f}: no vanilla art"));
            // every family's own vanilla files pass its check, but for a few outliers
            let passed = set.images.iter().filter(|i| set.calibration.check(&i.features).pass).count();
            let rate = passed as f64 / set.images.len() as f64;
            report.push_str(&format!("{f}: vanilla pass {passed} of {} ({rate:.3})\n", set.images.len()));
            assert!(rate >= MIN_VANILLA_PASS, "{f}: only {rate:.3} of vanilla passes its own check\n{report}");
            slot.insert(set);
        }
    }
    let mut failures = Vec::new();
    for l in &labels {
        let file = l["file"].as_str().unwrap();
        let label = l["label"].as_str().unwrap();
        let f = family(l["family"].as_str().unwrap());
        let res = check(&sets[&f], &fixtures().join("reference_set").join(file));
        let what = if res.pass { "pass".to_string() } else { format!("refused: {}", vanillamatch::describe(&res.outside)) };
        let gap = KNOWN_GAPS.contains(&file);
        report.push_str(&format!("{label:17} {file}: {what}{}\n", if gap { " [KNOWN GAP]" } else { "" }));
        let want_pass = label != "bad";
        if gap {
            continue;
        }
        if label == "good_extrapolated" {
            // reported, not required: these labels are less certain
            continue;
        }
        if res.pass != want_pass {
            failures.push(file.to_string());
        }
    }
    eprintln!("vanilla match against the reference labels\n{report}");
    assert!(failures.is_empty(), "wrong verdicts: {failures:?}\n{report}");
}

#[test]
fn agreement_with_the_secondary_set() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED vanilla match secondary set: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let mut sets: BTreeMap<Family, Option<VanillaSet>> = BTreeMap::new();
    let (mut agree, mut measured, mut unavailable) = (0usize, 0usize, 0usize);
    let mut report = String::new();
    for l in labels("secondary_set") {
        let f = family(l["family"].as_str().unwrap());
        let set = sets.entry(f).or_insert_with(|| vanilla(&game, f));
        let Some(set) = set else {
            unavailable += 1;
            continue;
        };
        let file = l["file"].as_str().unwrap();
        let verdict = l["verdict"].as_str().unwrap();
        let res = check(set, &fixtures().join("secondary_set").join(file));
        measured += 1;
        let good = verdict != "wrong";
        if res.pass == good {
            agree += 1;
        } else {
            let why = if res.pass { "passed".to_string() } else { vanillamatch::describe(&res.outside) };
            report.push_str(&format!("disagree {verdict:10} {file}: {why}\n"));
        }
    }
    eprintln!("secondary set: the check agrees with {agree} of {measured} verdicts ({unavailable} without vanilla art)\n{report}");
    assert!(agree >= MIN_SECONDARY_AGREEMENT, "agreement fell to {agree} of {measured}\n{report}");
}

/// Every family's vanilla pass rate, for the record. Slow in a debug build: run with `--ignored --release`.
#[test]
#[ignore]
fn every_family_vanilla_pass_rate() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED vanilla pass rates: the game is not installed");
        return;
    };
    let mut report = String::new();
    for f in Family::ALL.iter().copied() {
        match vanilla(&game, f) {
            None => report.push_str(&format!("{f}: no vanilla art (the check is unavailable)\n")),
            Some(set) => {
                let passed = set.images.iter().filter(|i| set.calibration.check(&i.features).pass).count();
                report.push_str(&format!("{f}: {passed} of {} ({:.3})\n", set.images.len(), passed as f64 / set.images.len() as f64));
            }
        }
    }
    eprintln!("vanilla pass rate per family\n{report}");
}
