//! The review's measured vanilla-match check against the installed game.
//!
//! Two kinds of evidence, both independent of any person's rating of generated art:
//!
//! * vanilla-calibrated assertions: every family's own vanilla files pass its check (at least [`MIN_VANILLA_PASS`]),
//!   read from the installed game and never copied,
//! * synthetic cases drawn by the test outside a rule the check states: a filled canvas where vanilla icons are cut
//!   out, a subject far smaller than vanilla's, and a solid drawing where vanilla special project icons are pale line
//!   art (their box is at most about 17 percent opaque).
//!
//! The check is a gate, not a full judgement: art of the wrong form can still measure inside vanilla's spread on every
//! feature (a loose object where vanilla has a plaque, for example), and the rubric and the side-by-side preview decide
//! those. Without the game every test prints `SKIPPED` and passes (set `HOI4_GAME_DIR` to point at it).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use imagegen_core::family::Family;
use imagegen_core::pixels::Rgba;
use imagegen_core::profile::ProfileSet;
use imagegen_core::vanillamatch::{self, VanillaSet};

/// Lowest vanilla pass rate accepted for a family's own files.
const MIN_VANILLA_PASS: f64 = 0.97;

/// Families with a synthetic case each.
const FAMILIES: [Family; 6] =
    [Family::CommanderTrait, Family::Focus, Family::IdeaConcern, Family::IdeaSpirit, Family::Medal, Family::SpecialProjectIcon];

fn find_game() -> Option<PathBuf> {
    let candidates = [
        std::env::var("HOI4_GAME_DIR").ok().map(PathBuf::from),
        Some(PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV")),
        Some(PathBuf::from("C:/Program Files/Steam/steamapps/common/Hearts of Iron IV")),
        std::env::var("HOME").ok().map(|h| PathBuf::from(h).join("Library/Application Support/Steam/steamapps/common/Hearts of Iron IV")),
    ];
    candidates.into_iter().flatten().find(|p| p.join("interface/subuniticons.gfx").is_file())
}

fn vanilla(game: &Path, f: Family) -> Option<VanillaSet> {
    let set = ProfileSet::builtin();
    let p = set.for_family(f)?;
    vanillamatch::load_vanilla(game, f, p.canvas, &p.accepted_strips())
}

fn check(set: &VanillaSet, img: &Rgba) -> vanillamatch::MatchResult {
    set.calibration.check(&vanillamatch::measure(img).expect("measurable"))
}

/// The family's most typical vanilla image shrunk to `scale` of its size and centred on the same canvas.
fn shrunk(set: &VanillaSet, scale: f64) -> Rgba {
    let src = &set.typical(1)[0].image;
    let (w, h) = (((f64::from(src.width) * scale) as u32).max(1), ((f64::from(src.height) * scale) as u32).max(1));
    let small = imagegen_core::resize::resize_area(src, w, h).unwrap();
    let mut out = Rgba::new(src.width, src.height);
    out.paste(&small, i64::from((src.width - w) / 2), i64::from((src.height - h) / 2));
    out
}

/// A light, solid disc filling most of the canvas.
fn solid_disc(w: u32, h: u32) -> Rgba {
    let mut img = Rgba::new(w, h);
    let (cx, cy, r) = (f64::from(w) / 2.0, f64::from(h) / 2.0, f64::from(w.min(h)) * 0.42);
    for y in 0..h {
        for x in 0..w {
            if (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy) < r {
                img.set_px(x, y, [235, 235, 235, 255]);
            }
        }
    }
    img
}

#[test]
fn vanilla_passes_and_synthetic_art_outside_the_rules_is_refused() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED vanilla match: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let mut report = String::new();
    let mut sets: BTreeMap<Family, VanillaSet> = BTreeMap::new();
    for f in FAMILIES {
        let set = vanilla(&game, f).unwrap_or_else(|| panic!("{f}: no vanilla art"));
        let passed = set.images.iter().filter(|i| set.calibration.check(&i.features).pass).count();
        let rate = passed as f64 / set.images.len() as f64;
        report.push_str(&format!(
            "{f}: vanilla pass {passed} of {} ({rate:.3})
",
            set.images.len()
        ));
        assert!(
            rate >= MIN_VANILLA_PASS,
            "{f}: only {rate:.3} of vanilla passes its own check
{report}"
        );
        sets.insert(f, set);
    }
    for (f, set) in &sets {
        let [w, h] = ProfileSet::builtin().for_family(*f).unwrap().canvas;
        // a transparent icon family: a canvas filled edge to edge is the wrong form
        let filled = check(set, &Rgba::filled(w, h, [120, 100, 70, 255]));
        report.push_str(&format!(
            "{f} filled canvas: {}
",
            vanillamatch::describe(&filled.outside)
        ));
        assert!(
            !filled.pass,
            "{f}: a filled canvas passed
{report}"
        );
        // a subject at a third of vanilla's size is the wrong form
        let small = check(set, &shrunk(set, 0.33));
        report.push_str(&format!(
            "{f} subject at a third of the size: {}
",
            vanillamatch::describe(&small.outside)
        ));
        assert!(
            !small.pass,
            "{f}: a subject at a third of vanilla's size passed
{report}"
        );
    }
    // special project icons are pale line drawings: a solid disc is far denser than any
    let set = &sets[&Family::SpecialProjectIcon];
    let [w, h] = ProfileSet::builtin().for_family(Family::SpecialProjectIcon).unwrap().canvas;
    let solid = check(set, &solid_disc(w, h));
    report.push_str(&format!(
        "special_project_icon solid disc: {}
",
        vanillamatch::describe(&solid.outside)
    ));
    assert!(
        !solid.pass,
        "a solid drawing passed the line-art check
{report}"
    );
    eprintln!(
        "vanilla match
{report}"
    );
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

/// A saturated, warm painted medallion, like the 0.3.2 gallery's agency upgrades but stronger, at a model's size.
fn warm_medallion(side: u32) -> imagegen_core::pixels::Rgba {
    let mut img = imagegen_core::pixels::Rgba::new(side, side);
    let c = f64::from(side) / 2.0;
    let r = c - 8.0;
    for y in 0..side {
        for x in 0..side {
            let d = (f64::from(x) + 0.5 - c).hypot(f64::from(y) + 0.5 - c);
            if d > r {
                continue;
            }
            let band = ((f64::from(x) / f64::from(side) * 9.0).sin() * 0.5 + 0.5) * 0.35;
            let v = (0.25 + band + 0.2 * (1.0 - d / r)).min(1.0) * if d > r - 6.0 { 0.35 } else { 1.0 };
            // orange-brown, saturation 0.72: max channel v, min channel v * 0.28, green between
            let (hi, lo) = (v * 255.0, v * 0.28 * 255.0);
            let g = lo + (hi - lo) * (0.5 + 0.2 * f64::from(y) / f64::from(side));
            img.set_px(x, y, [hi.round() as u8, g.round() as u8, lo.round() as u8, 255]);
        }
    }
    img
}

/// The agency upgrade check refuses a saturated, warm medallion that the 0.3.3 chain (trim and fit) passed on its
/// form, and the 0.3.4 chain (the round vignette, then the fit) brings it into vanilla's range. On HOI4 1.19.4.1 the
/// family's vanilla pass rate stays 15 of 15 with the family's palette checks.
#[test]
fn a_saturated_warm_agency_medallion_is_refused_until_processed() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED agency medallion vs vanilla: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    use imagegen_core::process::{run_chain, Op};
    let set = vanilla(&game, Family::AgencyUpgrade).expect("vanilla agency upgrades");
    let passed = set.images.iter().filter(|i| set.calibration.check(&i.features).pass).count();
    assert_eq!(passed, set.images.len(), "every vanilla agency upgrade passes");
    for f in ["saturation_mean", "value_mean", "warm_metal_share"] {
        assert!(set.calibration.ranges.iter().any(|r| r.feature == f && r.checked), "{f} is checked for agency upgrades");
    }
    let src = warm_medallion(400);
    let old = [Op::TrimToSubject { min_alpha: 16 }, Op::FitSubject { width: 68, height: 68, margin: 8, min_alpha: 16 }];
    let (before, _) = run_chain(&src, &old).unwrap();
    let res = set.calibration.check(&vanillamatch::measure(&before).unwrap());
    eprintln!("warm medallion before: {}", vanillamatch::describe(&res.outside));
    assert!(!res.pass && res.outside.iter().any(|o| o.feature == "saturation_mean"), "{res:?}");
    let profiles = ProfileSet::builtin();
    let p = profiles.for_family(Family::AgencyUpgrade).unwrap();
    let plan = imagegen_core::chain::plan_finishing(p, p.canvas, &src, false, false, "").unwrap();
    assert!(plan.ops.contains(&Op::RoundVignette), "{:?}", plan.ops);
    let (after, _) = run_chain(&src, &plan.ops).unwrap();
    let res = set.calibration.check(&vanillamatch::measure(&after).unwrap());
    eprintln!(
        "warm medallion after: saturation {}, value {}, gold tones {}",
        res.features["saturation_mean"], res.features["value_mean"], res.features["warm_metal_share"]
    );
    assert!(res.pass, "{}", vanillamatch::describe(&res.outside));
}

/// Each portrait family is checked against its own vanilla files, base game and DLC, with the backdrop in the top
/// corners and, for operatives, the spotlight behind the figure. On HOI4 1.19.4.1: leaders and commanders 268 of 271
/// measured files pass (a regency council, a placeholder and a cut-out portrait fail), scientists 217 of 217,
/// operatives 163 of 163. With 0.3.3's one shared set of every base-game gfx/leaders file it was 231 of 232 for each,
/// and the typical file of another family's backdrop is refused on the backdrop, except that the leader and commander
/// check passes the operatives' dark backdrop: vanilla leaders include special dark backdrops.
#[test]
fn portrait_families_keep_their_own_backdrop() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED portrait backdrops vs vanilla: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let fams = [Family::LeaderPortrait, Family::CommanderPortrait, Family::ScientistPortrait, Family::OperativePortrait];
    let sets: BTreeMap<Family, VanillaSet> = fams.iter().map(|f| (*f, vanilla(&game, *f).expect("vanilla portraits"))).collect();
    let mut report = String::new();
    for (f, set) in &sets {
        let passed = set.images.iter().filter(|i| set.calibration.check(&i.features).pass).count();
        report.push_str(&format!("{f}: vanilla pass {passed} of {}\n", set.images.len()));
        assert!(passed as f64 / set.images.len() as f64 >= MIN_VANILLA_PASS, "{f}\n{report}");
        for r in set.calibration.ranges.iter().filter(|r| r.feature.starts_with("backdrop_")) {
            // the spotlight behind the figure is the operatives' alone
            let want = r.feature != "backdrop_glow" || *f == Family::OperativePortrait;
            assert_eq!(r.checked, want, "{f}: {} checked", r.feature);
        }
    }
    let typical = |f: Family| sets[&f].typical(1)[0].features;
    for (f, set) in &sets {
        for g in fams {
            let same =
                matches!((f, g), (Family::LeaderPortrait | Family::CommanderPortrait, Family::LeaderPortrait | Family::CommanderPortrait)) || *f == g;
            let res = set.calibration.check(&typical(g));
            report.push_str(&format!(
                "{f} check on {g}'s typical file: {}\n",
                if res.pass { "pass".into() } else { vanillamatch::describe(&res.outside) }
            ));
            if same {
                assert!(res.pass, "{f} on {g}\n{report}");
            } else if g == Family::OperativePortrait && *f != Family::ScientistPortrait {
                // vanilla leaders include special dark backdrops (about one in twenty), so their check cannot refuse a dark
                // backdrop. The GFX-PORTRAIT-BACKDROP warning does (see tests/vanilla_forms.rs).
                continue;
            } else {
                assert!(res.outside.iter().any(|o| o.feature.starts_with("backdrop_")), "{f} on {g}\n{report}");
            }
        }
    }
    eprintln!("portrait backdrops\n{report}");
}
