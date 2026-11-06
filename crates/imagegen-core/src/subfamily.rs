//! Sub-families: the kinds a family's vanilla art splits into (army, naval, politics, research and so on).
//!
//! A file's sub-family comes from its name and folder tokens through the theme table of [`crate::refselect`]
//! ([`classify`]): the group whose themes its tokens hit most, else `other`. The same rule runs on the game files
//! when the data is generated and on every candidate at run time, so the two agree without a stored file list.
//!
//! `data/subfamilies.json` is generated from the installed game ([`build`]; the `subfamilies_data` test regenerates it
//! and checks it). It holds, per family with a large enough pool, each sub-family's file count, a few example file
//! names, its style groups (a k-medoids of the style descriptors, by medoid file name and size), measured form and
//! palette numbers, and a form note written from those numbers. It holds no image data.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::family::Family;
use crate::profile::ProfileSet;
use crate::refselect::{themes_of, tokens};
use crate::styledesc::{self, Vector};
use crate::vanillamatch::{self, Features};

/// Sub-family groups and the themes of [`crate::refselect::THEMES`] each one takes, in priority order for ties.
/// The narrow themes come first, so `military_industry` is industry and `naval_invasion` naval.
pub const GROUPS: &[(&str, &[&str])] = &[
    ("naval", &["naval"]),
    ("air", &["air"]),
    ("industry", &["industry", "infrastructure"]),
    ("economy", &["economy", "agriculture"]),
    ("research", &["research"]),
    ("intelligence", &["intelligence"]),
    ("politics", &["politics"]),
    ("diplomacy", &["diplomacy"]),
    ("society", &["population", "religion"]),
    ("army", &["army", "war", "defence"]),
];

/// The group of files that match no theme.
pub const OTHER: &str = "other";
/// Fewest files a sub-family needs to be listed (smaller groups count as `other`).
pub const MIN_LISTED: usize = 8;
/// Fewest measured vanilla files of a sub-family before the vanilla-match check uses its own spread.
pub const MIN_CHECK_FILES: usize = 20;
/// Fewest vanilla files a family needs to be split.
pub const MIN_FAMILY_FILES: usize = 60;
/// Most style groups per sub-family.
pub const MAX_STYLE_GROUPS: usize = 8;
/// Example file names kept per sub-family.
const EXAMPLES: usize = 3;

/// The group a file belongs to by its name and folder tokens, or [`OTHER`].
pub fn classify(stem: &str, dirs: &[&str]) -> &'static str {
    let mut toks = tokens(stem);
    if let Some(d) = dirs.last() {
        toks.extend(tokens(d));
    }
    best_group(toks.iter().flat_map(|t| themes_of(t)))
}

fn best_group<'a>(themes: impl Iterator<Item = &'a str>) -> &'static str {
    let mut count = [0usize; 16];
    for t in themes {
        for (i, (_, ts)) in GROUPS.iter().enumerate() {
            if ts.contains(&t) {
                count[i] += 1;
            }
        }
    }
    let best = (0..GROUPS.len()).fold(None, |b: Option<usize>, i| match b {
        Some(j) if count[j] >= count[i] => Some(j),
        _ if count[i] > 0 => Some(i),
        b => b,
    });
    best.map(|i| GROUPS[i].0).unwrap_or(OTHER)
}

/// Measured numbers of a sub-family's vanilla files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Measured {
    /// Files measured.
    pub images: usize,
    /// Share that is a compact, framed shape (convex hull and inscribed ellipse at least 0.9 and 0.85 opaque: a plaque,
    /// disc, shield or badge).
    pub framed_share: f64,
    /// Share that is an open, loose subject (the rest).
    pub loose_share: f64,
    /// Mean HSV saturation.
    pub saturation_mean: f64,
    /// Mean share of gold or bronze tones.
    pub warm_metal_share: f64,
}

/// One style group: a k-medoid of the sub-family's style descriptors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StyleGroup {
    /// File name of the medoid.
    pub medoid: String,
    /// Measured files nearest to it.
    pub size: usize,
}

/// One sub-family of a family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Subfamily {
    /// Id (`army`, `naval`, ..., `other`).
    pub id: String,
    /// Vanilla files in it.
    pub files: usize,
    /// A few file names.
    pub examples: Vec<String>,
    /// Style groups, largest first.
    pub style_groups: Vec<StyleGroup>,
    /// Measured numbers, when files could be measured.
    pub measured: Option<Measured>,
    /// The form note added to the prompt, written from the measured numbers.
    pub form_note: Option<String>,
}

/// A family's sub-families.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FamilySubfamilies {
    /// Family.
    pub family: Family,
    /// Vanilla files of the family.
    pub files: usize,
    /// Its sub-families, in [`GROUPS`] order, `other` last.
    pub subfamilies: Vec<Subfamily>,
}

/// The generated data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Data {
    /// Game version the data was measured on.
    pub game_version: Option<String>,
    /// How it was made.
    pub method: String,
    /// Families with sub-families.
    pub families: Vec<FamilySubfamilies>,
}

/// The data shipped in the binary.
pub fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../data/subfamilies.json")).expect("data/subfamilies.json parses"))
}

/// A family's sub-families, when it has any.
pub fn for_family(family: Family) -> Option<&'static FamilySubfamilies> {
    data().families.iter().find(|f| f.family == family)
}

impl FamilySubfamilies {
    /// A listed sub-family by id.
    pub fn get(&self, id: &str) -> Option<&Subfamily> {
        self.subfamilies.iter().find(|s| s.id == id)
    }

    /// Ids, in order.
    pub fn ids(&self) -> Vec<&str> {
        self.subfamilies.iter().map(|s| s.id.as_str()).collect()
    }

    /// The listed sub-family of a file: its group when listed, else `other` (when listed).
    pub fn member(&self, stem: &str, dirs: &[&str]) -> Option<&str> {
        let g = classify(stem, dirs);
        self.get(g).or_else(|| self.get(OTHER)).map(|s| s.id.as_str())
    }
}

/// The sub-family a brief points to: the listed group whose themes the brief's words hit most. `None` when the
/// family has no sub-families or the brief matches none.
pub fn infer(family: Family, brief: &str, asset: &str) -> Option<&'static str> {
    let fam = for_family(family)?;
    let words: Vec<String> = tokens(brief).into_iter().chain(tokens(asset)).collect();
    let g = best_group(words.iter().flat_map(|w| themes_of(w)));
    (g != OTHER).then_some(())?;
    fam.get(g).map(|s| s.id.as_str())
}

/// Where the vanilla-match check takes its spread from.
pub fn check_scope(family: Family, subfamily: Option<&str>, images: &[vanillamatch::VanillaImage]) -> (Vec<usize>, String) {
    let all: Vec<usize> = (0..images.len()).collect();
    let (Some(id), Some(fam)) = (subfamily, for_family(family)) else {
        return (all, "the whole family".into());
    };
    let idx: Vec<usize> = images
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            let stem = i.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let parent = i.path.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            fam.member(&stem, &[parent.as_str()]) == Some(id)
        })
        .map(|(k, _)| k)
        .collect();
    if idx.len() >= MIN_CHECK_FILES {
        (idx.clone(), format!("the sub-family {id} ({} measured files)", idx.len()))
    } else {
        (all, format!("the whole family (the sub-family {id} has {} measured files, fewer than {MIN_CHECK_FILES})", idx.len()))
    }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn feature(name: &str) -> usize {
    vanillamatch::NAMES.iter().position(|n| *n == name).expect("known feature")
}

/// Families not split: their names are per country, per person or per achievement, not per theme.
pub const NOT_SPLIT: &[Family] = &[
    Family::Flag,
    Family::LeaderPortrait,
    Family::CommanderPortrait,
    Family::OperativePortrait,
    Family::ScientistPortrait,
    Family::AdvisorCard,
    Family::Achievement,
];

/// The form note from the measured numbers, for icon families (a picture fills its frame, so it has none).
fn form_note(family: Family, id: &str, m: &Measured) -> Option<String> {
    if family.group() != crate::family::Group::Icon {
        return None;
    }
    let tenths = |x: f64| (x * 10.0).round() as u32;
    let shape = if m.framed_share >= 0.95 {
        "nearly all are a compact framed shape (a plaque, disc, shield or badge)".to_string()
    } else if m.framed_share <= 0.05 {
        "nearly all are an open, loose subject with no frame".to_string()
    } else {
        format!(
            "about {} in 10 are a compact framed shape (a plaque, disc, shield or badge) and {} in 10 an open, loose subject",
            tenths(m.framed_share),
            10 - tenths(m.framed_share)
        )
    };
    let colour = if m.saturation_mean < 0.2 {
        "muted, nearly grey colour"
    } else if m.saturation_mean < 0.35 {
        "moderate colour"
    } else {
        "strong colour"
    };
    let metal = if m.warm_metal_share >= 0.25 { ", with much gold or bronze" } else { "" };
    let what = if id == OTHER { "files that match no theme".to_string() } else { format!("{id} files") };
    Some(format!("Vanilla {} {what}: {shape}, in {colour}{metal}. Choose the form that suits the brief within that range.", family.label()))
}

/// Generate the data from the installed game.
pub fn build(game: &Path) -> Data {
    let builtin = ProfileSet::builtin();
    let mut families = Vec::new();
    for &family in Family::ALL.iter().filter(|f| !NOT_SPLIT.contains(f)) {
        let Some(profile) = builtin.for_family(family) else { continue };
        let sizes = profile.accepted_strips();
        let names = crate::stylesrc::vanilla_files(game, family, &sizes);
        if names.len() < MIN_FAMILY_FILES {
            continue;
        }
        let group_of = |p: &Path| {
            let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let parent = p.parent().and_then(|d| d.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            classify(&stem, &[parent.as_str()])
        };
        let mut counts: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for p in &names {
            counts.entry(group_of(p)).or_default().push(p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
        }
        // groups too small to list count as other
        let listed: Vec<&str> = GROUPS.iter().map(|(g, _)| *g).filter(|g| counts.get(g).is_some_and(|v| v.len() >= MIN_LISTED)).collect();
        if listed.len() < 2 {
            continue;
        }
        let id_of = |g: &'static str| if listed.contains(&g) { g } else { OTHER };
        let mut files: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (g, v) in counts {
            files.entry(id_of(g)).or_default().extend(v);
        }
        // measure the same sample the vanilla-match check reads
        let measured: Vec<(String, &'static str, Features, Vector)> = vanillamatch::vanilla_images(game, family, profile.canvas, &sizes)
            .into_iter()
            .filter_map(|(path, _, _, img)| {
                let f = vanillamatch::measure(&img)?;
                let a = styledesc::analyse(&img)?;
                Some((path.file_name()?.to_string_lossy().to_string(), id_of(group_of(&path)), f, a.v))
            })
            .collect();
        let (hull, ellipse, sat, metal) = (feature("hull_fill"), feature("ellipse_fill"), feature("saturation_mean"), feature("warm_metal_share"));
        let mut subs = Vec::new();
        for id in listed.iter().copied().chain(std::iter::once(OTHER)) {
            let Some(mut list) = files.get(id).cloned() else { continue };
            list.sort();
            let rows: Vec<&(String, &str, Features, Vector)> = measured.iter().filter(|m| m.1 == id).collect();
            let m = (!rows.is_empty()).then(|| {
                let n = rows.len() as f64;
                let framed = rows.iter().filter(|r| r.2[hull] >= 0.9 && r.2[ellipse] >= 0.85).count() as f64 / n;
                Measured {
                    images: rows.len(),
                    framed_share: round3(framed),
                    loose_share: round3(1.0 - framed),
                    saturation_mean: round3(rows.iter().map(|r| r.2[sat]).sum::<f64>() / n),
                    warm_metal_share: round3(rows.iter().map(|r| r.2[metal]).sum::<f64>() / n),
                }
            });
            let points: Vec<Vector> = rows.iter().map(|r| r.3).collect();
            let k = (points.len() / 10).clamp(1, MAX_STYLE_GROUPS).min(points.len());
            let med = styledesc::kmedoids(&points, k);
            let style_groups = med.medoids.iter().zip(&med.sizes).map(|(&i, &size)| StyleGroup { medoid: rows[i].0.clone(), size }).collect();
            let examples = (0..EXAMPLES.min(list.len())).map(|i| list[i * list.len() / EXAMPLES.min(list.len())].clone()).collect();
            subs.push(Subfamily {
                id: id.to_string(),
                files: list.len(),
                examples,
                style_groups,
                form_note: m.as_ref().and_then(|m| form_note(family, id, m)),
                measured: m,
            });
        }
        families.push(FamilySubfamilies { family, files: names.len(), subfamilies: subs });
    }
    Data {
        game_version: crate::config::game_version(game),
        method: format!(
            "files: the family's vanilla texture folders and name rules; group: the theme of the file name and folder tokens (refselect::THEMES, subfamily::GROUPS), groups under {MIN_LISTED} files count as other; measured: the vanilla-match sample of up to {} files; style groups: deterministic k-medoids of the style descriptors, one group per 10 measured files, at most {MAX_STYLE_GROUPS}",
            vanillamatch::MAX_FILES
        ),
        families,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_and_briefs_fall_into_the_same_groups() {
        assert_eq!(classify("goal_generic_navy_battleship", &["goals"]), "naval");
        assert_eq!(classify("focus_generic_military_industry", &[]), "industry", "a tie goes to the narrower theme");
        assert_eq!(classify("goal_generic_army_tanks", &[]), "army");
        assert_eq!(classify("idea_secret_police", &[]), "intelligence");
        assert_eq!(classify("goal_generic_soviet_politics", &[]), "politics");
        assert_eq!(classify("focus_something_else", &[]), OTHER);
    }

    #[test]
    fn the_shipped_data_is_well_formed() {
        let d = data();
        for f in &d.families {
            assert!(f.subfamilies.len() >= 2, "{}", f.family);
            for s in &f.subfamilies {
                assert!(s.id == OTHER || GROUPS.iter().any(|(g, _)| *g == s.id), "{}", s.id);
                assert!(s.id == OTHER || s.files >= MIN_LISTED);
                assert!(s.examples.len() <= EXAMPLES);
                if let Some(note) = &s.form_note {
                    let words = tokens(note);
                    for w in ["gun", "guns", "weapon", "weapons", "kill", "blood", "attack", "bomb", "explosion", "combat", "war"] {
                        assert!(!words.iter().any(|x| x == w), "{note}");
                    }
                }
            }
        }
    }

    fn image(name: &str) -> vanillamatch::VanillaImage {
        vanillamatch::VanillaImage {
            path: std::path::PathBuf::from(format!("game/gfx/interface/goals/{name}.dds")),
            frame: None,
            sha256: String::new(),
            image: crate::pixels::Rgba::new(1, 1),
            features: [0.0; vanillamatch::N],
        }
    }

    /// The vanilla-match check uses a sub-family's own spread from MIN_CHECK_FILES measured files, else the family's.
    #[test]
    fn the_check_scope_follows_the_subfamily_when_it_is_large_enough() {
        let Some(fam) = for_family(Family::Focus) else { return };
        if fam.get("naval").is_none() || fam.get("air").is_none() {
            return;
        }
        let mut images: Vec<_> = (0..MIN_CHECK_FILES + 2).map(|i| image(&format!("goal_generic_navy_{i}"))).collect();
        images.extend((0..5).map(|i| image(&format!("goal_generic_air_fighter_{i}"))));
        images.extend((0..9).map(|i| image(&format!("goal_generic_quiet_{i}"))));
        let (idx, why) = check_scope(Family::Focus, Some("naval"), &images);
        assert_eq!(idx.len(), MIN_CHECK_FILES + 2);
        assert!(why.starts_with("the sub-family naval"), "{why}");
        let (idx, why) = check_scope(Family::Focus, Some("air"), &images);
        assert_eq!(idx.len(), images.len(), "too few air files: the whole family");
        assert!(why.starts_with("the whole family (the sub-family air has 5"), "{why}");
        let (idx, why) = check_scope(Family::Focus, None, &images);
        assert_eq!((idx.len(), why.as_str()), (images.len(), "the whole family"));
    }

    #[test]
    fn a_brief_points_to_a_listed_subfamily() {
        let Some(fam) = for_family(Family::Focus) else { return };
        if fam.get("naval").is_some() {
            assert_eq!(infer(Family::Focus, "a battleship fleet at anchor", "grand_fleet"), Some("naval"));
        }
        assert_eq!(infer(Family::Focus, "a quiet mountain lake", "lake"), None);
        assert_eq!(infer(Family::GuiTexture, "a fleet", "x"), None);
    }
}
