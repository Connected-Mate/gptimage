//! Calibration: compare profiles with the installed game, header only.
//!
//! This is the same method as the research survey: read at most 32 bytes of each DDS and 18 bytes of each
//! TGA, count sizes per folder, and report where a profile's canvas is not the dominant vanilla size. It never
//! reads pixels, copies nothing out of the game folder, and never writes a profile: it prints a proposed diff
//! and a human decides.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::game_version;
use crate::error::{Error, Result};
use crate::profile::{Profile, ProfileSet};

/// Where to look for a profile's vanilla files.
struct Survey {
    profile: &'static str,
    folder: &'static str,
    prefix: Option<&'static str>,
    ext: &'static str,
    recursive: bool,
    mixed: bool,
}

const SURVEYS: &[Survey] = &[
    Survey { profile: "focus_icon", folder: "gfx/interface/goals", prefix: Some("focus_"), ext: "dds", recursive: false, mixed: false },
    Survey { profile: "idea_icon", folder: "gfx/interface/ideas", prefix: Some("idea_"), ext: "dds", recursive: false, mixed: false },
    Survey { profile: "advisor_icon", folder: "gfx/interface/ideas", prefix: Some("portrait_"), ext: "dds", recursive: false, mixed: false },
    Survey { profile: "decision_icon", folder: "gfx/interface/decisions", prefix: Some("decision_"), ext: "dds", recursive: false, mixed: true },
    Survey { profile: "decision_category_icon", folder: "gfx/interface/decisions", prefix: None, ext: "dds", recursive: false, mixed: true },
    Survey { profile: "decision_category_picture", folder: "gfx/interface/decisions", prefix: None, ext: "dds", recursive: false, mixed: true },
    Survey { profile: "tech_icon_small", folder: "gfx/interface/technologies", prefix: None, ext: "dds", recursive: false, mixed: true },
    Survey { profile: "tech_icon_medium", folder: "gfx/interface/technologies", prefix: None, ext: "dds", recursive: false, mixed: true },
    Survey { profile: "event_report", folder: "gfx/event_pictures", prefix: None, ext: "dds", recursive: false, mixed: true },
    Survey { profile: "event_news", folder: "gfx/event_pictures", prefix: None, ext: "dds", recursive: false, mixed: true },
    Survey { profile: "flag_set", folder: "gfx/flags", prefix: None, ext: "tga", recursive: false, mixed: false },
    Survey { profile: "mio_icon", folder: "gfx/interface/military_industrial_organization", prefix: None, ext: "dds", recursive: false, mixed: true },
    Survey { profile: "agency_logo", folder: "gfx/interface/operatives/agencies", prefix: None, ext: "dds", recursive: false, mixed: false },
    Survey { profile: "officer_spirit", folder: "gfx/interface/officer_corp/spirits", prefix: None, ext: "dds", recursive: false, mixed: false },
    Survey { profile: "leader_portrait", folder: "gfx/leaders", prefix: None, ext: "dds", recursive: true, mixed: false },
    Survey { profile: "achievement_icon", folder: "gfx/achievements", prefix: None, ext: "dds", recursive: false, mixed: false },
    Survey { profile: "counter_large", folder: "gfx/interface/counters/divisions_large", prefix: None, ext: "dds", recursive: false, mixed: false },
    Survey { profile: "counter_small", folder: "gfx/interface/counters/divisions_small", prefix: None, ext: "dds", recursive: false, mixed: false },
];

/// A size and how many files have it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SizeCount {
    /// `[width, height]`.
    pub size: [u32; 2],
    /// Files with that size.
    pub files: u64,
}

/// One profile compared with one vanilla folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CalibrationRow {
    /// Profile id.
    pub profile: String,
    /// Game folder that was read.
    pub folder: String,
    /// Files whose header was read.
    pub files: u64,
    /// The five most common sizes.
    pub top: Vec<SizeCount>,
    /// The profile's full-strip size (canvas x frames).
    pub profile_size: [u32; 2],
    /// Files at the profile's size.
    pub files_at_profile_size: u64,
    /// Share of the surveyed files at the profile's size.
    pub share_at_profile_size: f64,
    /// True when the profile's size is the most common size.
    pub profile_is_dominant: bool,
    /// Proposed canvas per frame when the profile is not dominant and another size clearly is.
    pub proposed_canvas: Option<[u32; 2]>,
    /// Where the profile's geometry came from.
    pub origin: String,
    /// Caveats.
    pub notes: Vec<String>,
}

/// The whole report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CalibrationReport {
    /// Game folder.
    pub game_dir: String,
    /// `rawVersion` from the launcher settings.
    pub game_version: Option<String>,
    /// Rows, in survey order.
    pub rows: Vec<CalibrationRow>,
    /// Profiles with no survey.
    pub not_surveyed: Vec<String>,
}

fn read_size(path: &Path, ext: &str) -> Option<[u32; 2]> {
    let mut f = File::open(path).ok()?;
    let mut buf = [0u8; 32];
    let n = f.read(&mut buf).ok()?;
    match ext {
        "dds" if n >= 20 && &buf[0..4] == b"DDS " => {
            let h = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
            let w = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
            Some([w, h])
        }
        "tga" if n >= 18 => Some([u32::from(u16::from_le_bytes([buf[12], buf[13]])), u32::from(u16::from_le_bytes([buf[14], buf[15]]))]),
        _ => None,
    }
}

fn collect(dir: &Path, s: &Survey, depth: u32, counts: &mut BTreeMap<[u32; 2], u64>, total: &mut u64) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if *total >= 60_000 {
            return;
        }
        if p.is_dir() {
            if s.recursive && depth < 4 {
                collect(&p, s, depth + 1, counts, total);
            }
            continue;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !name.ends_with(&format!(".{}", s.ext)) {
            continue;
        }
        if let Some(prefix) = s.prefix {
            if !name.starts_with(prefix) {
                continue;
            }
        }
        if let Some(size) = read_size(&p, s.ext) {
            *counts.entry(size).or_insert(0) += 1;
            *total += 1;
        }
    }
}

fn profile_size(p: &Profile) -> [u32; 2] {
    [p.canvas[0] * p.frames.max(1), p.canvas[1]]
}

/// Survey the game folder and compare every surveyed profile.
pub fn calibrate(game_dir: &Path, set: &ProfileSet) -> Result<CalibrationReport> {
    if !game_dir.is_dir() {
        return Err(Error::NotFound(format!("game folder {}", game_dir.display())));
    }
    let mut rows = Vec::new();
    let mut surveyed = Vec::new();
    for s in SURVEYS {
        let Some(profile) = set.get(s.profile) else { continue };
        surveyed.push(s.profile.to_string());
        let mut counts = BTreeMap::new();
        let mut total = 0u64;
        collect(&game_dir.join(s.folder), s, 0, &mut counts, &mut total);
        let mut sorted: Vec<(&[u32; 2], &u64)> = counts.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let top: Vec<SizeCount> = sorted.iter().take(5).map(|(s, n)| SizeCount { size: **s, files: **n }).collect();
        let want = profile_size(profile);
        let at = counts.get(&want).copied().unwrap_or(0);
        let dominant = top.first().map(|t| t.size);
        let is_dominant = dominant == Some(want);
        let mut notes = Vec::new();
        if total == 0 {
            notes.push(format!("no {} files found under {}", s.ext, s.folder));
        }
        if s.mixed {
            notes.push("the folder mixes several categories, so compare the count at the profile size rather than the dominant size".into());
        }
        let share = if total == 0 { 0.0 } else { (at as f64 / total as f64 * 10_000.0).round() / 10_000.0 };
        let proposed = match dominant {
            Some(d) if !is_dominant && !s.mixed && total >= 20 => {
                let frames = profile.frames.max(1);
                (d[0] % frames == 0).then_some([d[0] / frames, d[1]])
            }
            _ => None,
        };
        rows.push(CalibrationRow {
            profile: s.profile.to_string(),
            folder: s.folder.to_string(),
            files: total,
            top,
            profile_size: want,
            files_at_profile_size: at,
            share_at_profile_size: share,
            profile_is_dominant: is_dominant,
            proposed_canvas: proposed,
            origin: format!("{:?}", profile.origin).to_ascii_lowercase(),
            notes,
        });
    }
    let not_surveyed = set.all().filter(|p| !surveyed.contains(&p.id)).map(|p| p.id.clone()).collect();
    Ok(CalibrationReport {
        game_dir: crate::fsio::display_path(game_dir),
        game_version: game_version(game_dir),
        rows,
        not_surveyed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dds::write_a8r8g8b8;
    use crate::pixels::Rgba;
    use crate::profile::RowOverride;

    fn put(root: &Path, rel: &str, w: u32, h: u32) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, write_a8r8g8b8(&Rgba::new(w, h))).unwrap();
    }

    fn fake_game() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..30 {
            put(dir.path(), &format!("gfx/interface/goals/focus_{i}.dds"), 100, 88);
        }
        put(dir.path(), "gfx/interface/goals/focus_odd.dds", 94, 86);
        put(dir.path(), "gfx/interface/goals/shine_overlay.dds", 4, 4); // wrong prefix: ignored
        for i in 0..25 {
            put(dir.path(), &format!("gfx/interface/ideas/idea_{i}.dds"), 65, 67);
        }
        fs::write(dir.path().join("launcher-settings.json"), r#"{"rawVersion":"1.19.3.0"}"#).unwrap();
        dir
    }

    #[test]
    fn dominant_sizes_are_counted_from_headers_only() {
        let game = fake_game();
        let report = calibrate(game.path(), &ProfileSet::builtin()).unwrap();
        assert_eq!(report.game_version.as_deref(), Some("1.19.3.0"));
        let focus = report.rows.iter().find(|r| r.profile == "focus_icon").unwrap();
        assert_eq!(focus.files, 31);
        assert_eq!(focus.top[0], SizeCount { size: [100, 88], files: 30 });
        assert!(focus.profile_is_dominant);
        assert_eq!(focus.files_at_profile_size, 30);
        assert!(focus.proposed_canvas.is_none());
        let idea = report.rows.iter().find(|r| r.profile == "idea_icon").unwrap();
        assert!(idea.profile_is_dominant && idea.files == 25);
        assert!(report.not_surveyed.contains(&"event_super".to_string()));
    }

    #[test]
    fn a_kit_canvas_that_is_not_vanilla_gets_a_proposal() {
        let game = fake_game();
        let mut project = BTreeMap::new();
        project.insert("focus_icon".to_string(), RowOverride { canvas: Some([94, 86]), ..Default::default() });
        let set = ProfileSet::load(None, &project).unwrap();
        let report = calibrate(game.path(), &set).unwrap();
        let focus = report.rows.iter().find(|r| r.profile == "focus_icon").unwrap();
        assert!(!focus.profile_is_dominant);
        assert_eq!(focus.proposed_canvas, Some([100, 88]));
        assert_eq!(focus.files_at_profile_size, 1);
        assert_eq!(focus.origin, "project");
    }

    #[test]
    fn missing_game_and_empty_folders_are_reported_not_invented() {
        assert!(calibrate(Path::new("/definitely/not/a/game"), &ProfileSet::builtin()).is_err());
        let empty = tempfile::tempdir().unwrap();
        let report = calibrate(empty.path(), &ProfileSet::builtin()).unwrap();
        let focus = report.rows.iter().find(|r| r.profile == "focus_icon").unwrap();
        assert_eq!(focus.files, 0);
        assert!(focus.notes.iter().any(|n| n.contains("no dds files")));
        assert!(focus.proposed_canvas.is_none());
    }
}
