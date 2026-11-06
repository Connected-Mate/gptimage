//! Vanilla measurements behind the form rules and the `small_detail` check: the MIO trait glyph, the readability of
//! small icons and the landmark aspect. The vanilla side is read from the installed game and never copied. Without
//! the game the vanilla tests print `SKIPPED` and pass (set `HOI4_GAME_DIR` to point at it). Measured on HOI4 1.19.4.1.

use std::path::{Path, PathBuf};

use imagegen_core::family::Family;
use imagegen_core::pixels::Rgba;
use imagegen_core::profile::ProfileSet;
use imagegen_core::validate::{edge_share, small_detail_limit, validate_image, SMALL_DETAIL_CODE};
use imagegen_core::vanillamatch;

fn find_game() -> Option<PathBuf> {
    let candidates = [
        std::env::var("HOI4_GAME_DIR").ok().map(PathBuf::from),
        Some(PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV")),
        Some(PathBuf::from("C:/Program Files/Steam/steamapps/common/Hearts of Iron IV")),
        std::env::var("HOME").ok().map(|h| PathBuf::from(h).join("Library/Application Support/Steam/steamapps/common/Hearts of Iron IV")),
    ];
    candidates.into_iter().flatten().find(|p| p.join("interface/subuniticons.gfx").is_file())
}

fn vanilla(game: &Path, f: Family) -> Vec<Rgba> {
    let set = ProfileSet::builtin();
    let p = set.for_family(f).unwrap();
    vanillamatch::vanilla_images(game, f, p.canvas, &p.accepted_strips()).into_iter().map(|(_, _, _, img)| img).collect()
}

/// The interpolated quantile, as the vanilla-match calibration takes it.
fn quantile(mut v: Vec<f64>, q: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    let pos = q * (v.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    v[lo] + (v[hi] - v[lo]) * (pos - lo as f64)
}

/// Each small family's `small_detail` limit is the 95th percentile of its vanilla files' edge share, so about one
/// vanilla file in twenty gets the warning. Measured: raid 0.468 (27 files), state modifier 0.364 (100), decision 0.342
/// (328), commander trait 0.347 (81), personal trait 0.448 (96), decision category icon 0.112 (91).
#[test]
fn small_detail_limits_are_vanillas_95th_percentile() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED small_detail limits: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let mut report = String::new();
    for f in [Family::Raid, Family::StateModifier, Family::Decision, Family::CommanderTrait, Family::PersonalTrait, Family::DecisionCategoryIcon] {
        let shares: Vec<f64> = vanilla(&game, f).iter().filter_map(edge_share).collect();
        let p95 = quantile(shares.clone(), 0.95);
        let limit = small_detail_limit(f).unwrap();
        let warned = shares.iter().filter(|s| **s > limit).count();
        report.push_str(&format!(
            "{f}: {} files, median {:.3}, 95th percentile {p95:.3}, limit {limit:.3}, {warned} warned\n",
            shares.len(),
            quantile(shares.clone(), 0.5)
        ));
        assert!((p95 - limit).abs() <= 0.005, "{f}: limit {limit} is not vanilla's 95th percentile {p95}\n{report}");
        assert!(warned as f64 <= 0.06 * shares.len() as f64 + 1.0, "{f}: {warned} vanilla files warned\n{report}");
    }
    eprintln!("small_detail edge share\n{report}");
}

/// Twisted rails, two thin parallel lines with sleepers, the way the 0.3.2 gallery's unreadable raid icon was drawn,
/// get the warning. One bold solid pictogram does not. The warning never fails validation.
#[test]
fn thin_parallel_lines_on_a_small_canvas_get_the_warning() {
    let set = ProfileSet::builtin();
    let raid = set.for_family(Family::Raid).unwrap();
    let mut rails = Rgba::new(32, 32);
    for x in 4..28u32 {
        rails.set_px(x, 12, [200, 160, 80, 255]);
        rails.set_px(x, 19, [200, 160, 80, 255]);
        if x % 4 == 0 {
            for y in 11..21 {
                rails.set_px(x, y, [120, 90, 40, 255]);
            }
        }
    }
    let mut bold = Rgba::new(32, 32);
    for y in 6..26u32 {
        for x in 7..25u32 {
            bold.set_px(x, y, [200, 160, 80, 255]);
        }
    }
    let thin = edge_share(&rails).unwrap();
    let solid = edge_share(&bold).unwrap();
    eprintln!("edge share: rails {thin:.3}, bold block {solid:.3}, raid limit {:.3}", small_detail_limit(Family::Raid).unwrap());
    let code =
        |img: &Rgba| validate_image(img, raid, &raid.imagegen.checks, &Default::default()).findings.into_iter().find(|f| f.code == SMALL_DETAIL_CODE);
    let warn = code(&rails).expect("thin rails warn");
    assert_eq!(warn.level, imagegen_core::validate::Level::Warning);
    assert!(code(&bold).is_none(), "a bold shape reads");
    // families without the check never warn
    let focus = set.for_family(Family::Focus).unwrap();
    assert!(!focus.imagegen.checks.iter().any(|c| c == "small_detail"));
}

/// Inside the glyph (alpha 224 and up): mean value, dark share (value under 0.2), light share (over 0.75), colour
/// buckets (3 bits a channel) for 90 percent of the pixels. Also the share of the visible pixels that is the soft
/// shadow around it (alpha 16 to 223).
fn glyph_stats(img: &Rgba) -> [f64; 5] {
    let (mut vals, mut buckets, mut visible, mut soft) = (Vec::new(), std::collections::BTreeMap::<u16, u64>::new(), 0.0, 0.0);
    for p in img.data.chunks_exact(4) {
        if p[3] < 16 {
            continue;
        }
        visible += 1.0;
        if p[3] < 224 {
            soft += 1.0;
            continue;
        }
        vals.push(imagegen_core::styledesc::hsv(p[0], p[1], p[2]).2);
        *buckets.entry((u16::from(p[0] >> 5) << 6) | (u16::from(p[1] >> 5) << 3) | u16::from(p[2] >> 5)).or_default() += 1;
    }
    let n = vals.len() as f64;
    let mut sizes: Vec<u64> = buckets.into_values().collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let total: u64 = sizes.iter().sum();
    let (mut acc, mut colours) = (0u64, 0usize);
    for s in sizes {
        if acc * 10 >= total * 9 {
            break;
        }
        acc += s;
        colours += 1;
    }
    [
        vals.iter().sum::<f64>() / n,
        vals.iter().filter(|v| **v < 0.2).count() as f64 / n,
        vals.iter().filter(|v| **v > 0.75).count() as f64 / n,
        colours as f64,
        soft / visible,
    ]
}

/// The numbers the MIO trait rule is written from (44 files): mean value 0.50 inside the glyph, 6 percent dark and
/// 15 percent light pixels, 14 colour buckets, and a soft shadow that is 48 percent of the visible pixels (medians).
/// A dark, murky painted object, like the 0.3.2 gallery's traits, is far below the light share and above the dark one.
#[test]
fn mio_trait_glyphs_are_light_and_flat() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED MIO trait glyphs: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let files = vanilla(&game, Family::MioTrait);
    assert_eq!(files.len(), 44);
    let stats: Vec<[f64; 5]> = files.iter().map(glyph_stats).collect();
    let med = |k: usize| quantile(stats.iter().map(|s| s[k]).collect(), 0.5);
    let got = [med(0), med(1), med(2), med(3), med(4)];
    eprintln!("MIO trait glyph medians: value {:.3}, dark {:.3}, light {:.3}, colours {:.1}, shadow {:.3}", got[0], got[1], got[2], got[3], got[4]);
    for (k, (want, tol)) in [(0.50, 0.02), (0.06, 0.02), (0.15, 0.02), (14.0, 1.0), (0.48, 0.03)].iter().enumerate() {
        assert!((got[k] - want).abs() <= *tol, "statistic {k}: {} vs {want}", got[k]);
    }
    // a dark painted object: a grey-brown block with dark shading and no highlight
    let mut dark = Rgba::new(48, 48);
    for y in 10..38u32 {
        for x in 10..38u32 {
            let v = if (x + y) % 3 == 0 { 30 } else { 70 };
            dark.set_px(x, y, [v, v - 10, v - 20, 255]);
        }
    }
    let d = glyph_stats(&dark);
    assert!(d[0] < quantile(stats.iter().map(|s| s[0]).collect(), 0.1) && d[2] < 0.01);
}

/// Vanilla's landmarks: frame 1's subject is at most 4.0 times as wide as tall (the small Cataract dam; 95th
/// percentile 3.77, median 1.41 over 52 strips) and its box is centred within 0.47 to 0.55 of the frame, which is the
/// limit the building rule states. The landmark recipe fits frame 1 with its box centred, so a too-wide subject stays
/// centred; only the brief can make it less wide.
#[test]
fn landmark_aspect_and_centre_are_vanillas() {
    let set = ProfileSet::builtin();
    let p = set.for_family(Family::Building).unwrap();
    let rules = p.imagegen.prompt.rules.join(" ");
    assert!(rules.contains("at most about four times as wide as it is tall"), "{rules}");
    // a 6:1 panorama is fitted centred
    let mut wide = Rgba::new(600, 140);
    for y in 20..120u32 {
        for x in 0..600u32 {
            wide.set_px(x, y, [140, 130, 120, 255]);
        }
    }
    let plan = imagegen_core::chain::plan_finishing(p, p.canvas, &wide, false, false, "").unwrap();
    assert!(matches!(plan.ops.last(), Some(imagegen_core::process::Op::FitSubject { .. })), "{:?}", plan.ops);
    let (out, _) = imagegen_core::process::run_chain(&wide, &plan.ops).unwrap();
    let b = out.alpha_bbox(128).unwrap();
    assert!((f64::from(b[0] + b[2]) / 2.0 / f64::from(out.width) - 0.5).abs() < 0.02, "{b:?}");

    let Some(game) = find_game() else {
        eprintln!("SKIPPED landmark aspect: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let mut aspects = Vec::new();
    for strip in vanilla(&game, Family::Building) {
        let frame = strip.split_frames(2).unwrap().remove(0);
        let b = frame.alpha_bbox(128).unwrap();
        aspects.push(f64::from(b[2] - b[0]) / f64::from(b[3] - b[1]));
        let centre = f64::from(b[0] + b[2]) / 2.0 / f64::from(frame.width);
        assert!((0.46..=0.55).contains(&centre), "centre {centre}");
    }
    let max = aspects.iter().copied().fold(0.0, f64::max);
    eprintln!(
        "landmark frame aspect: {} strips, median {:.2}, 95th percentile {:.2}, max {max:.2}",
        aspects.len(),
        quantile(aspects.clone(), 0.5),
        quantile(aspects.clone(), 0.95)
    );
    assert!((max - imagegen_core::builtin::BUILDING_MAX_ASPECT).abs() <= 0.05, "vanilla's widest landmark is {max}");
}

/// Each portrait family's backdrop constant is the median of its own vanilla files' top corners, and the files are
/// separate: leaders and commanders share the pale cream-grey template (L*a*b* 89.8, -1.9, 8.5), scientists have the
/// pale blue-grey one (66.0, -7.2, -14.4), and La Resistance's 163 operative portraits are dark warm charcoal at the top (11.9, -0.1,
/// 5.3) with a spotlight behind the figure: the side band at two thirds of the height is 17.1 to 29.2 lighter.
/// At least 85 percent of each family's own files sit within the warning limit (the rest are special portraits with
/// dark or patterned backdrops, or a hat or hair in a corner), and the other families' backdrops do not.
#[test]
fn portrait_backdrops_are_each_familys_own() {
    use imagegen_core::validate::portrait_backdrop;
    let Some(game) = find_game() else {
        eprintln!("SKIPPED portrait backdrops: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let families = [Family::LeaderPortrait, Family::CommanderPortrait, Family::ScientistPortrait, Family::OperativePortrait];
    let mut report = String::new();
    for f in families {
        let labs: Vec<[f64; 3]> = vanilla(&game, f).iter().filter_map(vanillamatch::backdrop_lab).collect();
        let (want, limit) = portrait_backdrop(f).unwrap();
        let med: Vec<f64> = (0..3).map(|c| quantile(labs.iter().map(|l| l[c]).collect(), 0.5)).collect();
        let dist = |a: &[f64], b: &[f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        let within = labs.iter().filter(|l| dist(&l[..], &want) <= limit).count();
        report.push_str(&format!("{f}: {} files, median ({:.1}, {:.1}, {:.1}), {within} within {limit}\n", labs.len(), med[0], med[1], med[2]));
        for c in 0..3 {
            assert!((med[c] - want[c]).abs() <= 0.3, "{f} channel {c}: {} vs {}\n{report}", med[c], want[c]);
        }
        let share = within as f64 / labs.len() as f64;
        assert!(share >= 0.85, "{f}: only {share:.3} of vanilla within the limit\n{report}");
        if f == Family::OperativePortrait {
            let glows: Vec<f64> = vanilla(&game, f).iter().filter_map(vanillamatch::backdrop_glow).collect();
            let min = imagegen_core::validate::portrait_min_glow(f).unwrap();
            report.push_str(&format!(
                "  glow {:.1} to {:.1}, median {:.1}\n",
                quantile(glows.clone(), 0.0),
                quantile(glows.clone(), 1.0),
                quantile(glows.clone(), 0.5)
            ));
            assert!(labs.len() >= 160, "La Resistance's operatives are read from the DLC folder\n{report}");
            assert!(glows.iter().all(|g| *g >= min), "every vanilla operative has the spotlight\n{report}");
        }
        for g in families {
            let (other, _) = portrait_backdrop(g).unwrap();
            if other != want {
                assert!(dist(&other[..], &want) > limit, "{f} and {g} backdrops are not told apart");
            }
        }
    }
    eprintln!("portrait backdrops\n{report}");
}

/// A scientist portrait on the leaders' cream backdrop gets the backdrop warning, one on its own blue-grey backdrop does
/// not, and a leader portrait on the cream does not. The warning never fails validation.
#[test]
fn a_portrait_on_another_familys_backdrop_gets_the_warning() {
    use imagegen_core::validate::BACKDROP_CODE;
    let set = ProfileSet::builtin();
    let cream = Rgba::filled(156, 210, [229, 226, 209, 255]);
    let blue = Rgba::filled(156, 210, [135, 171, 193, 255]);
    let warns = |f: Family, img: &Rgba| {
        let p = set.for_family(f).unwrap();
        let r = validate_image(img, p, &p.imagegen.checks, &Default::default());
        assert!(r.findings.iter().all(|x| x.level != imagegen_core::validate::Level::Error || x.code != BACKDROP_CODE));
        r.findings.iter().any(|x| x.code == BACKDROP_CODE)
    };
    assert!(warns(Family::ScientistPortrait, &cream));
    assert!(!warns(Family::ScientistPortrait, &blue));
    assert!(!warns(Family::LeaderPortrait, &cream) && !warns(Family::CommanderPortrait, &cream));
    assert!(warns(Family::LeaderPortrait, &blue) && warns(Family::OperativePortrait, &cream));
    // an operative on vanilla's charcoal: evenly dark warns (no spotlight), with the glow beside the shoulders it does not
    let flat = Rgba::filled(156, 210, [34, 31, 24, 255]);
    let mut lit = flat.clone();
    for y in 0..210u32 {
        let t = (f64::from(y) / 145.0).min(1.0);
        let c = |a: f64, b: f64| (a + (b - a) * t).round() as u8;
        for x in 0..156u32 {
            lit.set_px(x, y, [c(34.0, 95.0), c(31.0, 92.0), c(24.0, 85.0), 255]);
        }
    }
    assert!(warns(Family::OperativePortrait, &flat), "an evenly dark backdrop lacks the spotlight");
    assert!(!warns(Family::OperativePortrait, &lit), "{:?}", vanillamatch::backdrop_glow(&lit));
}

/// Vanilla's doctrine icons run their glow close to the canvas edge: 55 of 105 touch it and 73 keep at most 1 px clear
/// (alpha 16 and up), so the doctrine row checks a 1 px margin, and the fit leaves room for the glow on top of it.
#[test]
fn doctrine_margin_is_within_vanillas() {
    let set = ProfileSet::builtin();
    let p = set.for_family(Family::Doctrine).unwrap();
    assert_eq!(p.margin, 1);
    let Some(game) = find_game() else {
        eprintln!("SKIPPED doctrine margins: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let mins: Vec<u32> = vanilla(&game, Family::Doctrine)
        .iter()
        .filter(|i| [i.width, i.height] == p.canvas)
        .filter_map(|i| i.alpha_bbox(16).map(|b| b[0].min(b[1]).min(i.width - b[2]).min(i.height - b[3])))
        .collect();
    let at_most_one = mins.iter().filter(|m| **m <= 1).count();
    eprintln!(
        "doctrine vanilla margins: {} files, {} touch the edge, {at_most_one} at most 1 px",
        mins.len(),
        mins.iter().filter(|m| **m == 0).count()
    );
    assert!(at_most_one * 2 > mins.len(), "most vanilla doctrine icons keep 1 px or less");
}
