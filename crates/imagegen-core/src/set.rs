//! Set mode: every icon one focus tree file needs, planned as one set.
//!
//! A focus tree names its icons one focus at a time, and making them one by one drifts in style and repeats
//! subjects. [`focus_tree_set`] reads the tree through the shared HOI4 engine (the same index and content trace as the
//! audit), keeps the `icon` of every focus in the one file, and groups the focuses by the sprite they show: several
//! focuses that share a sprite share one asset, so nothing is made twice. Each icon carries what the game knows about
//! it (whether the sprite is defined and its texture exists) and what the brief can say about it: the focus ids and
//! their English names and descriptions from the mod's localisation.
//!
//! The engine side (`hoi4.image_set`) plans one `focus` asset per icon with the same style pack and style anchors,
//! and can generate them. Nothing here writes to the project or the game.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use hoi4_engine::sprite_audit::{Allowlist, SpriteReport};
use hoi4_engine::sprite_trace::TraceOptions;
use hoi4_engine::{Workspace, WorkspaceOptions};
use serde::Serialize;

use crate::error::{Error, Result};

/// Most icons one set holds.
pub const MAX_ICONS: usize = 300;
/// Longest localised text quoted in a brief.
const MAX_TEXT: usize = 300;
/// Localisation files read at most.
const MAX_LOC_FILES: usize = 2000;

/// One icon of a set: a sprite and the focuses that show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetIcon {
    /// The sprite the focuses name, as written.
    pub sprite: String,
    /// The asset name planned for it (snake_case, unique within the set).
    pub asset: String,
    /// Focus ids that show this sprite, in file order.
    pub focuses: Vec<String>,
    /// Their English names, where the mod's localisation has them.
    pub names: Vec<String>,
    /// Their English descriptions (`<id>_desc`), where present.
    pub descriptions: Vec<String>,
    /// True when a loaded `.gfx` file defines the sprite.
    pub defined: bool,
    /// The texture the sprite points at, if defined.
    pub texture: Option<String>,
    /// True when that texture exists in the mod or the game.
    pub texture_exists: bool,
}

impl SetIcon {
    /// True when the game would show no art: the sprite is undefined or its texture is missing.
    pub fn missing(&self) -> bool {
        !self.defined || !self.texture_exists
    }
}

/// The icons of one focus tree file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FocusTreeSet {
    /// The file, relative to the mod.
    pub file: String,
    /// Focus uses that could not be resolved (an icon built at run time), as `focus:<id>`.
    pub dynamic: Vec<String>,
    /// The icons, in the order the file first names them.
    pub icons: Vec<SetIcon>,
}

/// An asset name from a sprite name: `GFX_focus_GER_rearm` and `GFX_goal_rearm` lose their prefixes, the rest is
/// lower-cased snake_case. Never empty: a name with no letter becomes `icon`.
pub fn asset_from_sprite(sprite: &str) -> String {
    let lower = sprite.to_ascii_lowercase();
    let mut rest = lower.strip_prefix("gfx_").unwrap_or(&lower);
    for p in ["focus_", "goal_"] {
        if let Some(r) = rest.strip_prefix(p) {
            rest = r;
            break;
        }
    }
    let mut out = String::new();
    for c in rest.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let mut out = out.trim_end_matches('_').to_string();
    if !out.starts_with(|c: char| c.is_ascii_lowercase()) {
        out = format!("icon_{out}").trim_end_matches('_').to_string();
    }
    out.truncate(crate::naming::MAX_NAME_LEN);
    out.trim_end_matches('_').to_string()
}

fn walk_yml(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: u32) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() >= MAX_LOC_FILES {
            return;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && depth < 4 {
            walk_yml(&e.path(), out, depth + 1);
        } else if ft.is_file() && e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".yml") {
            out.push(e.path());
        }
    }
}

/// English localisation of `keys` from the mod's `localisation/english` folder (and `localisation/` files named
/// `*_l_english.yml`). Formatting codes are stripped.
pub fn english_texts(root: &Path, keys: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut files = Vec::new();
    walk_yml(&root.join("localisation"), &mut files, 0);
    let mut out = BTreeMap::new();
    for f in files {
        let name = f.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let in_english = f.components().any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("english"));
        if !in_english && !name.contains("l_english") {
            continue;
        }
        let Ok(bytes) = crate::fsio::read_limited(&f, 8 * 1024 * 1024) else { continue };
        let loc = hoi4_syntax::localisation::LocFile::parse(bytes);
        for e in &loc.entries {
            if keys.contains(&e.key) && !out.contains_key(&e.key) {
                out.insert(e.key.clone(), clean_text(&loc.value(e)));
            }
        }
    }
    out
}

/// Localised text without colour codes (`§Y`, `§!`), icons (`£x`) and scripted parts (`[...]`, `$...$`).
pub fn clean_text(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut skip_to: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(end) = skip_to {
            if c == end {
                skip_to = None;
            }
            continue;
        }
        match c {
            '§' => {
                chars.next();
            }
            '£' => {
                while chars.peek().is_some_and(|c| c.is_alphanumeric() || *c == '_') {
                    chars.next();
                }
            }
            '[' => skip_to = Some(']'),
            '$' => skip_to = Some('$'),
            '\\' if chars.peek() == Some(&'n') => {
                chars.next();
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    let joined = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > MAX_TEXT {
        format!("{}...", joined.chars().take(MAX_TEXT).collect::<String>())
    } else {
        joined
    }
}

/// The icons of the focus tree in `file` (relative to `root`). Reads the mod and, when given, the game; writes nothing.
pub fn focus_tree_set(root: &Path, game: Option<&Path>, file: &str) -> Result<FocusTreeSet> {
    let file = file.replace('\\', "/").trim_start_matches("./").to_string();
    crate::fsio::check_relative(&file)?;
    if !root.join(&file).is_file() {
        return Err(Error::NotFound(format!("{file} (the focus tree file, relative to the mod)")));
    }
    let ws = Workspace::load(&WorkspaceOptions {
        game: game.map(Path::to_path_buf),
        target: Some(root.to_path_buf()),
        include_dlc: game.is_some(),
        ..Default::default()
    });
    let target = ws.target_layer().map(|i| ws.layers[i].name.clone());
    let (_, report) = SpriteReport::run(&ws, &Allowlist::default(), &TraceOptions::default());
    let mut icons: Vec<SetIcon> = Vec::new();
    let mut dynamic = Vec::new();
    for u in &report.uses {
        if u.content_type != "focus.icon" || !u.path.eq_ignore_ascii_case(&file) || target.as_deref().is_some_and(|t| t != u.layer) {
            continue;
        }
        let focus = u.subject.as_deref().and_then(|s| s.strip_prefix("focus:")).unwrap_or("").to_string();
        if u.dynamic || u.sprite.is_empty() {
            dynamic.push(format!("focus:{focus}"));
            continue;
        }
        let texture_exists = u.texture.as_ref().is_some_and(|t| t.exists);
        match icons.iter_mut().find(|i| i.sprite.eq_ignore_ascii_case(&u.sprite)) {
            Some(i) => {
                if !focus.is_empty() && !i.focuses.contains(&focus) {
                    i.focuses.push(focus);
                }
            }
            None => icons.push(SetIcon {
                sprite: u.sprite.clone(),
                asset: String::new(),
                focuses: if focus.is_empty() { vec![] } else { vec![focus] },
                names: Vec::new(),
                descriptions: Vec::new(),
                defined: u.defined,
                texture: u.texture.as_ref().map(|t| t.path.clone()),
                texture_exists,
            }),
        }
    }
    if icons.len() > MAX_ICONS {
        return Err(Error::Invalid(format!("{file} names {} icons; a set holds at most {MAX_ICONS}", icons.len())));
    }
    // asset names, unique within the set
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for i in &mut icons {
        let base = asset_from_sprite(&i.sprite);
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        taken.insert(name.clone());
        i.asset = name;
    }
    let keys: BTreeSet<String> = icons.iter().flat_map(|i| i.focuses.iter().flat_map(|f| [f.clone(), format!("{f}_desc")])).collect();
    let texts = english_texts(root, &keys);
    for i in &mut icons {
        i.names = i.focuses.iter().filter_map(|f| texts.get(f).cloned()).filter(|t| !t.is_empty()).collect();
        i.descriptions = i.focuses.iter().filter_map(|f| texts.get(&format!("{f}_desc")).cloned()).filter(|t| !t.is_empty()).collect();
    }
    Ok(FocusTreeSet { file, dynamic, icons })
}

/// The brief of one icon: what the focuses are called and say, and that the icon is one of a set with distinct
/// subjects.
pub fn icon_brief(icon: &SetIcon, set_size: usize) -> String {
    let named = if icon.names.is_empty() { icon.focuses.join(", ") } else { icon.names.join("; ") };
    let mut brief = format!("The icon of the national focus {named}");
    if icon.names.is_empty() {
        brief.push_str(" (a focus id: read it as the focus's subject)");
    }
    brief.push('.');
    if let Some(d) = icon.descriptions.first() {
        brief.push_str(&format!(" The focus says: \"{d}\""));
    }
    if set_size > 1 {
        brief.push_str(&format!(
            " One of {set_size} icons of the same focus tree, made as one set: show this focus's own subject, not a generic emblem the other focuses could share."
        ));
    }
    brief
}

/// What gives an icon its dominant colour: the subject words in its focus names and descriptions, checked in order.
/// The country never picks a colour: a tree about one nation would otherwise paint every icon in that nation's
/// colours and symbols and leave no contrast between icons.
const SUBJECT_COLOURS: &[(&[&str], &str)] = &[
    (&["navy", "naval", "fleet", "ship", "sea", "harbour", "harbor", "port", "submarine", "convoy", "maritime"], "navy and sea blue"),
    (&["air", "aircraft", "fighter", "bomber", "pilot", "airfield", "airborne", "paratroop"], "pale sky blue and silver"),
    (&["army", "tank", "armour", "armor", "infantry", "conscript", "division", "garrison", "artillery", "military", "rearm"], "steel grey and olive"),
    (
        &[
            "industry",
            "industrial",
            "factory",
            "factories",
            "production",
            "mine",
            "mining",
            "coal",
            "oil",
            "fuel",
            "steel",
            "machine",
            "construction",
            "infrastructure",
        ],
        "brass, amber and smoke grey",
    ),
    (&["farm", "agricultur", "harvest", "peasant", "food", "forest", "rural"], "green and golden wheat"),
    (&["science", "research", "technology", "university", "radio", "nuclear", "scientist", "engineer"], "pale blue and white"),
    (
        &["church", "religion", "faith", "culture", "cultural", "art", "heritage", "tradition", "monarch", "crown", "king"],
        "cream and gold with deep red",
    ),
    (&["revolt", "revolution", "rebel", "protest", "strike", "unrest", "uprising", "communis", "workers", "terror"], "dark red and black"),
    (&["spy", "spies", "intelligence", "secret", "police", "surveillance", "sabotage", "propaganda"], "dark charcoal and cold blue"),
    (&["trade", "economy", "economic", "bank", "money", "finance", "tax", "debt", "loan", "invest"], "gold and deep blue"),
    (&["diplomacy", "diplomatic", "treaty", "alliance", "negotiat", "union", "peace", "neutral", "embassy"], "warm tan and teal"),
];
/// Colours handed to icons no subject word places, in turn, so neighbours differ.
const SPARE_COLOURS: &[&str] = &[
    "ochre and sand",
    "deep teal and white",
    "dark violet and plum",
    "brick red and rust",
    "slate blue and silver",
    "olive and khaki",
    "warm cream and charcoal",
    "burnt orange and dark brown",
];

/// True when a word of the text starts with `stem` and is at most three letters longer (so `army` matches `armies`'
/// stem `army`, and `air` does not match `airplane` loosely enough to claim `repair`).
fn has_stem(text: &str, stem: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric()).any(|t| t.starts_with(stem) && t.len() <= stem.len() + 3)
}

/// The dominant colour planned for each icon of a set, in order. A subject word picks it, except that no colour
/// goes to more than a quarter of the set (rounded up, at least two): later icons that would exceed it take the next
/// spare colour. Icons no word places take the spare colours in turn.
pub fn colour_plan(icons: &[&SetIcon]) -> Vec<&'static str> {
    let cap = icons.len().div_ceil(4).max(2);
    let mut used: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut spare = 0usize;
    let mut out = Vec::with_capacity(icons.len());
    for icon in icons {
        let text = format!("{} {}", icon.names.join(" "), icon.descriptions.join(" ")).to_lowercase();
        let by_subject = SUBJECT_COLOURS
            .iter()
            .find(|(words, _)| words.iter().any(|w| has_stem(&text, w)))
            .map(|(_, c)| *c)
            .filter(|c| used.get(c).copied().unwrap_or(0) < cap);
        let colour = by_subject.unwrap_or_else(|| {
            // the next spare colour that is still under the cap
            for _ in 0..SPARE_COLOURS.len() {
                let c = SPARE_COLOURS[spare % SPARE_COLOURS.len()];
                spare += 1;
                if used.get(c).copied().unwrap_or(0) < cap {
                    return c;
                }
            }
            SPARE_COLOURS[spare % SPARE_COLOURS.len()]
        });
        *used.entry(colour).or_insert(0) += 1;
        out.push(colour);
    }
    out
}

/// The brief of one icon in a set, with the colour planned for it: the dominant colour follows the subject, and the
/// country's own colours and symbols stay a small accent.
pub fn icon_brief_coloured(icon: &SetIcon, set_size: usize, colour: &str) -> String {
    let mut brief = icon_brief(icon, set_size);
    brief.push_str(&format!(
        " Colour: this icon is mainly {colour}, as its subject calls for. The country's own colours and symbols are at most a small accent (a ribbon, a flag, one emblem), never the colour of the whole icon."
    ));
    brief
}

/// How alike the dominant colours of a generated set are.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Variety {
    /// `ok`, `too_alike` (over 45 percent of the icons share one dominant hue, or the coloured ones span fewer than
    /// three hues) or `too_few` (fewer than five icons to judge).
    pub verdict: &'static str,
    /// Icons measured.
    pub icons: usize,
    /// Distinct dominant hues (30 degree buckets) among the icons that have one.
    pub distinct_hues: usize,
    /// The share of icons in the most common bucket.
    pub largest_share: f64,
    /// Icons per hue bucket, `neutral` for those with almost no saturated colour.
    pub buckets: BTreeMap<String, Vec<String>>,
    /// What to do, when the verdict is `too_alike`.
    pub advice: Option<String>,
}

/// The dominant hue bucket of an image: opaque pixels weighted by their saturation, in 12 buckets of 30 degrees
/// (`h0` red, `h4` green, `h8` blue), or `neutral` when saturated pixels (above 0.25) are under 12 percent of the
/// opaque ones.
pub fn dominant_hue(img: &crate::pixels::Rgba) -> String {
    let mut hist = [0f64; 12];
    let (mut opaque, mut coloured) = (0u64, 0u64);
    for p in img.data.chunks_exact(4) {
        if p[3] < 200 {
            continue;
        }
        opaque += 1;
        let (r, g, b) = (f64::from(p[0]) / 255.0, f64::from(p[1]) / 255.0, f64::from(p[2]) / 255.0);
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        let d = mx - mn;
        if mx < 0.12 || d / mx.max(1e-6) < 0.25 {
            continue;
        }
        coloured += 1;
        let h = if (mx - r).abs() < f64::EPSILON {
            ((g - b) / d).rem_euclid(6.0)
        } else if (mx - g).abs() < f64::EPSILON {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } * 60.0;
        hist[((h / 30.0) as usize) % 12] += d;
    }
    if opaque == 0 || (coloured as f64) < 0.12 * opaque as f64 {
        return "neutral".into();
    }
    // a hue on a bucket edge counts a little for both neighbours
    let score = |a: usize| hist[a] + 0.5 * (hist[(a + 1) % 12] + hist[(a + 11) % 12]);
    let best = (0..12).max_by(|&a, &b| score(a).partial_cmp(&score(b)).unwrap_or(std::cmp::Ordering::Equal));
    format!("h{}", best.unwrap_or(0))
}

/// Judge a set from `(icon name, dominant hue bucket)` pairs. Five or more icons are judged: the set is `too_alike`
/// when over 45 percent of them share one bucket, or when most are coloured and span fewer than three hues.
pub fn judge_variety(items: &[(String, String)]) -> Variety {
    let mut buckets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, bucket) in items {
        buckets.entry(bucket.clone()).or_default().push(name.clone());
    }
    let n = items.len();
    let coloured: Vec<usize> = buckets.iter().filter(|(k, _)| k.as_str() != "neutral").map(|(_, v)| v.len()).collect();
    let distinct_hues = coloured.len();
    let largest = buckets.values().map(Vec::len).max().unwrap_or(0);
    let largest_share = if n == 0 { 0.0 } else { (largest as f64 / n as f64 * 1000.0).round() / 1000.0 };
    let verdict = if n < 5 {
        "too_few"
    } else if largest_share > 0.45 || (distinct_hues < 3 && coloured.iter().sum::<usize>() * 2 > n) {
        "too_alike"
    } else {
        "ok"
    };
    let advice = (verdict == "too_alike").then(|| {
        "Too many icons share one dominant colour. Make the ones in the largest bucket again with a different planned colour (set icons carry one in their brief), or choose references with more varied colours.".to_string()
    });
    Variety { verdict, icons: n, distinct_hues, largest_share, buckets, advice }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names_drop_the_sprite_prefixes() {
        assert_eq!(asset_from_sprite("GFX_focus_GER_Rearm"), "ger_rearm");
        assert_eq!(asset_from_sprite("GFX_goal_generic_construct_civ_factory"), "generic_construct_civ_factory");
        assert_eq!(asset_from_sprite("GFX_focus_1936-plan"), "icon_1936_plan");
        assert_eq!(asset_from_sprite("GFX_"), "icon");
        crate::naming::validate_snake("asset", &asset_from_sprite("GFX_focus_A__B")).unwrap();
    }

    #[test]
    fn localised_text_loses_codes_and_scripted_parts() {
        assert_eq!(clean_text("§YRearm§! the [ROOT.GetName] army £pol_power now\\nfast"), "Rearm the army now fast");
        assert!(clean_text(&"x".repeat(400)).ends_with("..."));
    }

    #[test]
    fn a_brief_names_the_focus_and_the_set() {
        let icon = SetIcon {
            sprite: "GFX_focus_a".into(),
            asset: "a".into(),
            focuses: vec!["TAG_a".into()],
            names: vec!["Steel for the Navy".into()],
            descriptions: vec!["Build ships.".into()],
            defined: false,
            texture: None,
            texture_exists: false,
        };
        assert!(icon.missing());
        let b = icon_brief(&icon, 12);
        assert!(b.contains("Steel for the Navy") && b.contains("\"Build ships.\"") && b.contains("One of 12 icons"), "{b}");
    }

    fn icon_named(name: &str) -> SetIcon {
        SetIcon {
            sprite: format!("GFX_focus_{name}"),
            asset: name.to_lowercase().replace(' ', "_"),
            focuses: vec![name.into()],
            names: vec![name.into()],
            descriptions: vec![],
            defined: false,
            texture: None,
            texture_exists: false,
        }
    }

    #[test]
    fn colours_follow_the_subject_and_not_the_country() {
        let icons: Vec<SetIcon> =
            ["Irish Navy Expansion", "Irish Tank Corps", "Irish Air Force", "Irish Heavy Industry", "Irish Harvest", "Irish Nuclear Research"]
                .iter()
                .map(|n| icon_named(n))
                .collect();
        let refs: Vec<&SetIcon> = icons.iter().collect();
        let plan = colour_plan(&refs);
        assert_eq!(plan[0], "navy and sea blue");
        assert_eq!(plan[1], "steel grey and olive");
        assert_eq!(plan[2], "pale sky blue and silver");
        assert_eq!(plan[3], "brass, amber and smoke grey");
        assert_eq!(plan[4], "green and golden wheat");
        assert_eq!(plan[5], "pale blue and white");
        let b = icon_brief_coloured(&icons[1], 6, plan[1]);
        assert!(b.contains("mainly steel grey and olive") && b.contains("never the colour of the whole icon"), "{b}");
    }

    #[test]
    fn no_colour_takes_more_than_a_quarter_of_a_set() {
        // twelve naval focuses: the cap is 3, the rest take spare colours
        let icons: Vec<SetIcon> = (0..12).map(|i| icon_named(&format!("Fleet {i}"))).collect();
        let refs: Vec<&SetIcon> = icons.iter().collect();
        let plan = colour_plan(&refs);
        let navy = plan.iter().filter(|c| **c == "navy and sea blue").count();
        assert_eq!(navy, 3, "{plan:?}");
        let distinct: BTreeSet<_> = plan.iter().collect();
        assert!(distinct.len() >= 4, "{plan:?}");
        assert!(!has_stem("repair the roads", "air"));
    }

    fn flat(rgb: [u8; 3]) -> crate::pixels::Rgba {
        let mut img = crate::pixels::Rgba::new(8, 8);
        for p in img.data.chunks_exact_mut(4) {
            p.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        img
    }

    #[test]
    fn dominant_hue_buckets_and_the_variety_verdict() {
        assert_eq!(dominant_hue(&flat([200, 30, 30])), "h0");
        assert_eq!(dominant_hue(&flat([30, 200, 30])), "h4");
        assert_eq!(dominant_hue(&flat([30, 30, 200])), "h8");
        assert_eq!(dominant_hue(&flat([120, 120, 125])), "neutral");
        let greens: Vec<(String, String)> = (0..8).map(|i| (format!("i{i}"), dominant_hue(&flat([30, 190 + i as u8, 40])))).collect();
        let v = judge_variety(&greens);
        assert_eq!(v.verdict, "too_alike");
        assert!(v.advice.is_some() && v.largest_share > 0.9, "{v:?}");
        let mixed: Vec<(String, String)> = [[200, 30, 30], [30, 200, 30], [30, 30, 200], [200, 180, 30], [120, 30, 160], [120, 120, 125]]
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("i{i}"), dominant_hue(&flat(*c))))
            .collect();
        assert_eq!(judge_variety(&mixed).verdict, "ok");
        assert_eq!(judge_variety(&mixed[..3]).verdict, "too_few");
    }
}
