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
}
