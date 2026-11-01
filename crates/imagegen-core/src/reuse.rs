//! The reuse guard: one set of pixels, one family.
//!
//! Hashing the decoded RGBA pixels (not the file) finds an image however it was re-encoded: the same art as a
//! PNG, a DDS or a TGA hashes the same. `find_cross_family_reuse` looks the hash up among the assets this
//! project already installed (the ledger) and, while it is cheap, among the textures that sit in the texture
//! folders of the other families. New reuse is an error at generation and import (`GFX-FAMILY-REUSE`); the
//! later audit packet calls the same helper to report existing reuse as warnings.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result, RetryClass};
use crate::family::Family;
use crate::fsio::ProjectFs;
use crate::hashing::SystemClock;
use crate::ledger::Ledger;
use crate::lock::REUSE_CODE;
use crate::pixels::{decode, DecodeLimits, Rgba};
use crate::profile::{Profile, ProfileSet};
use crate::records::{kind, InstalledRecord};

/// Most texture files read from the other families' folders in one scan.
pub const MAX_SCANNED_FILES: usize = 4000;
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 6;

/// Where a reused asset was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReuseSource {
    /// An install record in the provenance ledger.
    Ledger,
    /// A texture found in the folder of another family.
    Folder,
}

/// One earlier asset with the same pixels, installed or placed for another family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReuseHit {
    /// The family the earlier asset belongs to.
    pub family: Family,
    /// Project-relative path of the earlier asset.
    pub path: String,
    /// How it was found.
    pub source: ReuseSource,
}

/// SHA-256 of the decoded RGBA pixels (the same value the ledger records as `pixels_sha256`).
pub fn pixel_hash(img: &Rgba) -> String {
    img.pixels_sha256()
}

/// Assets of other families with these pixels, using the built-in folder layout of every family.
pub fn find_cross_family_reuse(project: &ProjectFs, family: Family, hash: &str) -> Result<Vec<ReuseHit>> {
    find_cross_family_reuse_with(project, &ProfileSet::builtin(), family, hash)
}

/// Assets of other families with these pixels, using the mod's effective profiles for the folder layout.
///
/// Hits from the ledger come first, then texture files found in other families' folders. A texture in a folder
/// that more than one family could own is skipped (it cannot be attributed), and the scan stops after
/// [`MAX_SCANNED_FILES`] files.
pub fn find_cross_family_reuse_with(project: &ProjectFs, profiles: &ProfileSet, family: Family, hash: &str) -> Result<Vec<ReuseHit>> {
    let mut hits: Vec<ReuseHit> = Vec::new();

    // 1. what this tool installed
    let ledger = Ledger::new(project.clone(), Arc::new(SystemClock));
    for entry in ledger.entries()?.into_iter().filter(|e| e.kind == kind::INSTALLED) {
        let Ok(rec) = serde_json::from_value::<InstalledRecord>(entry.data) else { continue };
        if rec.family == family {
            continue;
        }
        for f in &rec.files {
            if f.pixels_sha256 == hash && !hits.iter().any(|h| h.path == f.path) {
                hits.push(ReuseHit { family: rec.family, path: f.path.clone(), source: ReuseSource::Ledger });
            }
        }
    }

    // 2. textures sitting in another family's folders
    let layout = folder_layout(profiles);
    let mut roots: Vec<String> = layout.iter().flat_map(|(_, pats)| pats.iter().map(|p| base_of(p))).filter(|b| !b.is_empty()).collect();
    roots.sort();
    roots.dedup();
    // keep only the outermost roots so a folder is walked once
    let roots_copy = roots.clone();
    roots.retain(|r| !roots_copy.iter().any(|o| o != r && r.starts_with(&format!("{o}/"))));
    let mut scanned = 0usize;
    for root in roots {
        walk(project, &root, 0, &mut |rel: &str| {
            if scanned >= MAX_SCANNED_FILES {
                return false;
            }
            let lower = rel.to_ascii_lowercase();
            if !(lower.ends_with(".dds") || lower.ends_with(".tga")) {
                return true;
            }
            let Some(owner) = attribute(&layout, rel) else { return true };
            if owner == family {
                return true;
            }
            scanned += 1;
            let Ok(bytes) = project.read(rel, MAX_FILE_BYTES) else { return true };
            let Ok((img, _)) = decode(&bytes, Some(rel), DecodeLimits::default()) else { return true };
            if pixel_hash(&img) == hash && !hits.iter().any(|h| h.path == rel) {
                hits.push(ReuseHit { family: owner, path: rel.to_string(), source: ReuseSource::Folder });
            }
            true
        })?;
    }
    Ok(hits)
}

/// The refusal for new reuse: `Ok` when nothing else uses these pixels.
pub fn ensure_no_cross_family_reuse(project: &ProjectFs, profiles: &ProfileSet, family: Family, hash: &str, what: &str) -> Result<()> {
    let hits = find_cross_family_reuse_with(project, profiles, family, hash)?;
    let Some(first) = hits.first() else { return Ok(()) };
    let others: Vec<String> = hits.iter().map(|h| format!("{} ({})", h.path, h.family)).collect();
    Err(Error::refused(
        REUSE_CODE,
        format!(
            "{what} has exactly the pixels of {} already used as {}. One image is one family: an idea icon is not a focus or decision icon. Make or bring art made for {family}",
            others.join(", "),
            first.family
        ),
        RetryClass::Terminal,
    ))
}

/// Folder patterns of every family that owns a folder of its own. `gui_texture` is left out: its folder is a
/// catch-all under `gfx/interface/` that would claim every other family's textures.
fn folder_layout(profiles: &ProfileSet) -> Vec<(Family, Vec<Vec<String>>)> {
    Family::ALL
        .iter()
        .copied()
        .filter(|f| *f != Family::GuiTexture)
        .filter_map(|f| profiles.for_family(f).map(|p| (f, folder_patterns(p))))
        .collect()
}

/// Folder pattern as path segments (`{feature_slug}` stays a wildcard segment).
fn folder_patterns(p: &Profile) -> Vec<Vec<String>> {
    let split = |s: &str| s.split('/').filter(|x| !x.is_empty()).map(str::to_string).collect::<Vec<_>>();
    if p.imagegen.variants.is_empty() {
        vec![split(&p.imagegen.folder)]
    } else {
        p.imagegen.variants.iter().map(|v| split(&v.folder)).collect()
    }
}

/// Segments before the first placeholder, joined.
fn base_of(pattern: &[String]) -> String {
    pattern.iter().take_while(|s| !s.starts_with('{')).cloned().collect::<Vec<_>>().join("/")
}

/// How well a folder pattern matches the directory of a file: the number of matched segments, or `None`.
///
/// The pattern is `base / {feature_slug} / tail`. A file matches when its directory starts with `base`, then
/// has either nothing more (a file straight in the base) or one slug segment followed by `tail`.
fn score(pattern: &[String], dir: &[&str]) -> Option<usize> {
    let idx = pattern.iter().position(|s| s.starts_with('{')).unwrap_or(pattern.len());
    let base = &pattern[..idx];
    if dir.len() < base.len() || !base.iter().zip(dir).all(|(a, b)| a == b) {
        return None;
    }
    let rest = &dir[idx..];
    if idx == pattern.len() {
        // no slug segment: only the base itself
        return rest.is_empty().then_some(idx);
    }
    if rest.is_empty() {
        return Some(idx);
    }
    let tail = &pattern[idx + 1..];
    let after = &rest[1..];
    if after.len() >= tail.len() && tail.iter().zip(after).all(|(a, b)| a == b) {
        Some(idx + 1 + tail.len())
    } else {
        None
    }
}

/// The one family whose folder pattern matches a file best; `None` when none does or two tie.
fn attribute(layout: &[(Family, Vec<Vec<String>>)], rel: &str) -> Option<Family> {
    let mut parts: Vec<&str> = rel.split('/').collect();
    parts.pop();
    let scored: Vec<(usize, Family)> =
        layout.iter().filter_map(|(f, pats)| pats.iter().filter_map(|p| score(p, &parts)).max().map(|s| (s, *f))).collect();
    let top = scored.iter().map(|(s, _)| *s).max()?;
    let mut winners = scored.iter().filter(|(s, _)| *s == top).map(|(_, f)| *f);
    let first = winners.next()?;
    if winners.next().is_some() { None } else { Some(first) }
}

fn walk(project: &ProjectFs, dir: &str, depth: usize, visit: &mut dyn FnMut(&str) -> bool) -> Result<bool> {
    if depth > MAX_DEPTH {
        return Ok(true);
    }
    let mut names = project.list_names(dir)?;
    names.sort();
    for name in names {
        let rel = format!("{dir}/{name}");
        let is_dir = project.resolve(&rel).map(|p| p.is_dir()).unwrap_or(false);
        if is_dir {
            if !walk(project, &rel, depth + 1, visit)? {
                return Ok(false);
            }
        } else if !visit(&rel) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dds;
    use crate::hashing::FixedClock;
    use crate::records::{InstalledFile, InstalledRecord};

    fn project() -> (tempfile::TempDir, ProjectFs) {
        let dir = tempfile::tempdir().unwrap();
        let fs = ProjectFs::new(dir.path()).unwrap();
        (dir, fs)
    }

    fn art() -> Rgba {
        let mut img = Rgba::new(60, 68);
        for y in 5..60 {
            for x in 5..50 {
                img.set_px(x, y, [200, 40, 10, 255]);
            }
        }
        img
    }

    #[test]
    fn the_pixel_hash_ignores_the_container() {
        let img = art();
        let dds_bytes = dds::write_a8r8g8b8(&img);
        let back = dds::decode(&dds_bytes).unwrap();
        assert_eq!(pixel_hash(&img), pixel_hash(&back));
        let mut other = img.clone();
        other.set_px(0, 0, [1, 2, 3, 255]);
        assert_ne!(pixel_hash(&img), pixel_hash(&other));
    }

    #[test]
    fn an_installed_asset_of_another_family_is_found_through_the_ledger() {
        let (_d, fs) = project();
        let ledger = Ledger::new(fs.clone(), Arc::new(FixedClock(0)));
        let img = art();
        let rec = InstalledRecord {
            target: "f.a-v1-p1".into(),
            family: Family::IdeaSpirit,
            mode: "handoff".into(),
            files: vec![InstalledFile { path: "gfx/interface/ideas/f/idea_a.dds".into(), sha256: "x".into(), pixels_sha256: pixel_hash(&img), container: "dds".into() }],
            sprites: vec![],
            gfx_file: None,
            replaced: vec![],
            approval_id: "apr".into(),
        };
        ledger.append(kind::INSTALLED, Some("f.a"), Some("f.a-v1-p1"), &rec).unwrap();
        let hits = find_cross_family_reuse(&fs, Family::Decision, &pixel_hash(&img)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].family, hits[0].source), (Family::IdeaSpirit, ReuseSource::Ledger));
        // the same family may reinstall its own pixels, and other pixels are free
        assert!(find_cross_family_reuse(&fs, Family::IdeaSpirit, &pixel_hash(&img)).unwrap().is_empty());
        assert!(find_cross_family_reuse(&fs, Family::Decision, "00").unwrap().is_empty());
        let err = ensure_no_cross_family_reuse(&fs, &ProfileSet::builtin(), Family::Decision, &pixel_hash(&img), "this import").unwrap_err();
        assert_eq!(err.code(), REUSE_CODE);
    }

    #[test]
    fn a_texture_in_another_familys_folder_is_found_by_its_pixels() {
        let (_d, fs) = project();
        let img = art();
        fs.write_new("gfx/interface/ideas/mymod/idea_hand_made.dds", &dds::write_a8r8g8b8(&img)).unwrap();
        let hits = find_cross_family_reuse(&fs, Family::Decision, &pixel_hash(&img)).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!((hits[0].family, hits[0].source), (Family::IdeaSpirit, ReuseSource::Folder));
        // a texture in a family's own folder is not reuse for that family
        assert!(find_cross_family_reuse(&fs, Family::IdeaSpirit, &pixel_hash(&img)).unwrap().is_empty());
    }

    #[test]
    fn nested_family_folders_are_attributed_to_the_deepest_family() {
        let (_d, fs) = project();
        let img = art();
        // concerns live under the idea folder but are their own family
        fs.write_new("gfx/interface/ideas/mymod/concerns/idea_c.dds", &dds::write_a8r8g8b8(&img)).unwrap();
        let hits = find_cross_family_reuse(&fs, Family::IdeaSpirit, &pixel_hash(&img)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].family, Family::IdeaConcern);
        assert!(find_cross_family_reuse(&fs, Family::IdeaConcern, &pixel_hash(&img)).unwrap().is_empty());
    }

    #[test]
    fn attribution_needs_a_unique_best_match() {
        let set = ProfileSet::builtin();
        let layout = folder_layout(&set);
        assert_eq!(attribute(&layout, "gfx/interface/goals/x/focus_a.dds"), Some(Family::Focus));
        assert_eq!(attribute(&layout, "gfx/interface/decisions/x/category_icons/a.dds"), Some(Family::DecisionCategoryIcon));
        assert_eq!(attribute(&layout, "gfx/flags/medium/GER.tga"), Some(Family::Flag));
        assert_eq!(attribute(&layout, "gfx/achievements/a.dds"), Some(Family::Achievement));
        assert_eq!(attribute(&layout, "gfx/interface/unrelated/a.dds"), None);
    }
}
