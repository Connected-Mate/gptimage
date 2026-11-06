//! The vanilla side of the review: the measured vanilla-match check and the side-by-side preview sheet.
//!
//! Both read the family's vanilla art from the installed game (`imagegen_core::vanillamatch`). The art is measured
//! once per family and game folder and kept in memory. The side-by-side sheet holds vanilla pixels, so it is never
//! written into the mod: it goes to the user's cache folder (`HOI4_IMAGEGEN_CACHE_DIR`, else the platform cache
//! folder) and back to the caller as an image.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use imagegen_core::family::Family;
use imagegen_core::fsio::display_path;
use imagegen_core::hashing::sha256_hex;
use imagegen_core::pixels::{encode_png, Rgba};
use imagegen_core::profile::ProfileSet;
use imagegen_core::vanillamatch::{self, VanillaSet};
use serde_json::{json, Value};

use super::Engine;

/// Most vanilla images on the side-by-side sheet.
pub const SHEET_REFERENCES: usize = 6;

/// A family's measured vanilla art, or why there is none.
type Loaded = Result<Arc<VanillaSet>, String>;

/// Measured vanilla art per (family, game folder). A failed load is cached too, as its reason.
#[derive(Default)]
pub struct Cache {
    inner: Mutex<HashMap<(Family, PathBuf), Loaded>>,
}

impl Engine {
    /// The family's measured vanilla art, or why there is none.
    pub(crate) fn vanilla_set(&self, family: Family) -> Result<Arc<VanillaSet>, String> {
        let Some(game) = self.game_dir() else {
            return Err("the installed game was not found (set HOI4_GAME_DIR or game_dir in .hoi4-imagegen/config.toml)".into());
        };
        let key = (family, game.clone());
        if let Some(hit) = self.vanilla_cache.inner.lock().expect("vanilla cache lock").get(&key) {
            return hit.clone();
        }
        let builtin = ProfileSet::builtin();
        let own = self.profiles.for_family(family);
        let base = builtin.for_family(family).or(own);
        let frame = base.map(|p| p.canvas).unwrap_or([0, 0]);
        let sizes = base.map(|p| p.accepted_strips()).unwrap_or_default();
        let loaded = vanillamatch::load_vanilla(&game, family, frame, &sizes).map(Arc::new).ok_or_else(|| {
            format!("the installed game has fewer than {} measurable vanilla {} images to compare with", vanillamatch::MIN_FILES, family.label())
        });
        self.vanilla_cache.inner.lock().expect("vanilla cache lock").insert(key, loaded.clone());
        loaded
    }

    /// The review's measured check of an artifact against its reference set: the style pack's references when it
    /// was made with a pack, else the family's vanilla art. `status` is `pass`, `fail` or `unavailable` (with
    /// `reason`), and `reference_set` names the set it used. Against vanilla, a sub-family with at least
    /// `subfamily::MIN_CHECK_FILES` measured vanilla files gives the ranges from those files alone; otherwise the whole
    /// family does. `compared_with` records which.
    pub(crate) fn reference_check(&self, a: &super::Artifact, img: &Rgba) -> Value {
        let (set, described) = self.comparison_set(a);
        self.check_with(a.family, a.subfamily.as_deref(), set, described, img)
    }

    fn check_with(&self, family: Family, subfamily: Option<&str>, set: Result<Arc<VanillaSet>, String>, described: Value, img: &Rgba) -> Value {
        let rule = described.get("rule").and_then(Value::as_str).unwrap_or(vanillamatch::VERSION).to_string();
        let against_pack = described.get("kind").and_then(Value::as_str) == Some("style_pack");
        let set = match set {
            Ok(s) => s,
            Err(reason) => return json!({"status": "unavailable", "version": rule, "family": family, "reason": reason, "reference_set": described}),
        };
        let game_version = self.game_dir().and_then(|g| imagegen_core::config::game_version(&g));
        let Some(features) = vanillamatch::measure(img) else {
            return json!({
                "status": "fail",
                "version": rule,
                "family": family,
                "vanilla_images": set.images.len(),
                "game_version": game_version,
                "reason": "the image has no opaque pixel to measure",
                "outside": [],
                "reference_set": described,
            });
        };
        // a pack's references are the mod's own set for the family: the sub-family spread is vanilla's
        let (scope, compared_with) = if against_pack {
            ((0..set.images.len()).collect::<Vec<usize>>(), json!("style_pack"))
        } else {
            let (scope, compared_with) = imagegen_core::subfamily::check_scope(family, subfamily, &set.images);
            (scope, json!(compared_with))
        };
        let own;
        let calibration = if scope.len() == set.images.len() {
            &set.calibration
        } else {
            let rows: Vec<vanillamatch::Features> = scope.iter().map(|&i| set.images[i].features).collect();
            match vanillamatch::calibrate(family, &rows) {
                Some(c) => {
                    own = c;
                    &own
                }
                None => &set.calibration,
            }
        };
        let res = calibration.check(&features);
        let ranges: Vec<&vanillamatch::FeatureRange> = calibration.ranges.iter().filter(|r| r.checked).collect();
        json!({
            "status": if res.pass { "pass" } else { "fail" },
            "version": rule,
            "family": family,
            "subfamily": subfamily,
            "compared_with": compared_with,
            "vanilla_images": scope.len(),
            "game_version": game_version,
            "outside": res.outside,
            "features": res.features,
            "checked_ranges": ranges,
            "reference_set": described,
        })
    }

    /// Where vanilla comparison sheets go: outside the mod, in the user's cache folder.
    fn vanilla_sheet_dir(&self) -> Option<PathBuf> {
        self.project_cache_dir("vanilla-compare")
    }

    /// A folder of this project's in the user's cache folder (`HOI4_IMAGEGEN_CACHE_DIR`, else the platform cache
    /// folder), for files that may hold vanilla pixels and so never go into the mod.
    pub(crate) fn project_cache_dir(&self, what: &str) -> Option<PathBuf> {
        let project = sha256_hex(display_path(self.fs.root()).as_bytes());
        Some(self.user_cache_dir()?.join(what).join(&project[..12]))
    }

    /// The user's cache folder (`HOI4_IMAGEGEN_CACHE_DIR`, else the platform cache folder).
    pub(crate) fn user_cache_dir(&self) -> Option<PathBuf> {
        Some(if let Some(d) = self.env.get("HOI4_IMAGEGEN_CACHE_DIR") {
            PathBuf::from(d)
        } else if let Some(d) = self.env.get("LOCALAPPDATA") {
            PathBuf::from(d).join("hoi4-imagegen").join("cache")
        } else if let Some(d) = self.env.get("XDG_CACHE_HOME") {
            PathBuf::from(d).join("hoi4-imagegen")
        } else {
            PathBuf::from(self.env.get("HOME")?).join(".cache").join("hoi4-imagegen")
        })
    }

    /// The side-by-side sheet: the candidate at native size and enlarged, beside the family's most typical vanilla
    /// images at the same sizes, with the vanilla-match result. `Ok(None)` (with the reason) when there is no vanilla
    /// art to show. The sheet is written outside the mod; its record lists every vanilla file by path and hash.
    pub(crate) fn vanilla_side_by_side(&self, a: &super::Artifact, img: &Rgba, stem: &str) -> Result<(Value, Vec<u8>), String> {
        let (set, described) = self.comparison_set(a);
        let set = set?;
        let refs = set.typical(SHEET_REFERENCES);
        let longest = img.width.max(img.height).max(1);
        let scale = (256 / longest).clamp(1, 6);
        let mut row_native: Vec<Rgba> = vec![super::preview::panel(img, "dark")];
        let mut row_big: Vec<Rgba> = vec![super::preview::panel(&super::preview::upscale_nearest(img, scale), "dark")];
        for r in &refs {
            row_native.push(super::preview::panel(&r.image, "dark"));
            row_big.push(super::preview::panel(&super::preview::upscale_nearest(&r.image, scale), "dark"));
        }
        // the candidate's column is marked with a light bar above it
        let bar = 4u32;
        let width = |row: &[Rgba]| row.iter().map(|p| p.width).sum::<u32>();
        let sheet_w = width(&row_native).max(width(&row_big));
        let h1 = row_native.iter().map(|p| p.height).max().unwrap_or(0);
        let h2 = row_big.iter().map(|p| p.height).max().unwrap_or(0);
        let mut sheet = Rgba::filled(sheet_w, bar + h1 + h2, [60, 60, 66, 255]);
        for x in 0..row_big[0].width.max(row_native[0].width).min(sheet_w) {
            for y in 0..bar {
                sheet.set_px(x, y, [232, 200, 90, 255]);
            }
        }
        let mut x = 0i64;
        for p in &row_native {
            sheet.paste(p, x, i64::from(bar));
            x += i64::from(p.width);
        }
        let mut x = 0i64;
        for p in &row_big {
            sheet.paste(p, x, i64::from(bar + h1));
            x += i64::from(p.width);
        }
        let png = encode_png(&sheet).map_err(|e| e.to_string())?;
        let sha = sha256_hex(&png);
        let path = match self.vanilla_sheet_dir() {
            Some(dir) => {
                let file = dir.join(format!("{stem}-vanilla-{}.png", &sha[..8]));
                std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, &png)).ok().map(|_| display_path(&file))
            }
            None => None,
        };
        let references: Vec<Value> =
            refs.iter().map(|r| json!({"path": display_path(&r.path).replace('\\', "/"), "frame": r.frame, "sha256": r.sha256})).collect();
        let report = json!({
            "path": path,
            "sha256": sha,
            "size": [sheet.width, sheet.height],
            "scale": scale,
            "layout": "top row: the candidate (under the light bar) then the vanilla references at native size on dark; bottom row: the same, enlarged",
            "references": references,
            "reference_set": described,
            "note": "Written outside the mod: the sheet can hold vanilla pixels, and the tool never copies vanilla art into a mod.",
        });
        Ok((report, png))
    }
}
