//! Family profiles.
//!
//! Geometry (canvas, accepted sizes, margin, frames) and its evidence belong to the kit's
//! `.tools/gfx/profiles.json`. This crate reads that file when it exists and layers it over a built-in
//! default set measured from the installed game (HOI4 1.19.3.0 header survey, research report 12).
//! Project overrides come last. Each effective profile records where every value came from, so
//! `image_profiles` can show a calibration diff instead of silently preferring one source.

use std::collections::BTreeMap;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::animation::{AnimationSpec, Motion};
use crate::error::{Error, Result};
use crate::family::{Automation, Family};
use crate::hashing::{canonical_hash, sha256_hex};

/// Where a profile's geometry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileOrigin {
    /// Built-in default measured from the installed game.
    Builtin,
    /// A row of the kit's `profiles.json`.
    Kit,
    /// A project override in `.hoi4-imagegen/config.toml`.
    Project,
}

/// Background the generated art needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    /// Real alpha: the unused canvas is transparent.
    Transparent,
    /// A full picture with no transparency.
    Opaque,
}

/// File container of the installed asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    /// DirectDraw Surface, uncompressed A8R8G8B8, one mip level.
    Dds,
    /// Targa, uncompressed 32-bit.
    Tga,
}

impl Container {
    /// File extension.
    pub fn ext(self) -> &'static str {
        match self {
            Container::Dds => "dds",
            Container::Tga => "tga",
        }
    }
}

/// One size variant of a multi-file asset (flags have three).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Variant {
    /// Variant name, for example `medium`.
    pub name: String,
    /// Canvas of this variant.
    pub canvas: [u32; 2],
    /// Project-relative folder of this variant.
    pub folder: String,
}

/// Sprite registration details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpriteSpec {
    /// Target `.gfx` file under `interface/`, with placeholders.
    pub gfx_file: String,
    /// Sprite name pattern.
    pub name: String,
    /// Companion sprites generated with the main one (`shine`).
    #[serde(default)]
    pub companions: Vec<String>,
}

/// Prompt rules the plan step turns into text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PromptSpec {
    /// Style and content rules, one sentence each.
    pub rules: Vec<String>,
    /// Things to avoid.
    pub negative: Vec<String>,
}

/// Everything the generator needs beyond geometry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImagegenSpec {
    /// Who produces the art.
    pub automation: Automation,
    /// Background requirement.
    pub background: Background,
    /// Installed container.
    pub container: Container,
    /// Default project-relative folder, with placeholders. Flags use `variants` instead.
    pub folder: String,
    /// File name prefix, with placeholders.
    pub prefix: String,
    /// Other file name prefixes the game accepts for the family (the focus family accepts `goal_`).
    #[serde(default)]
    pub alt_prefixes: Vec<String>,
    /// Sprite registration, when the family uses `.gfx` sprites.
    pub sprite: Option<SpriteSpec>,
    /// Companion files the engine expects next to the main file, as name suffixes (`_grey`, `_small`).
    #[serde(default)]
    pub companion_files: Vec<String>,
    /// Size variants for multi-file assets.
    #[serde(default)]
    pub variants: Vec<Variant>,
    /// Deterministic checks that apply.
    pub checks: Vec<String>,
    /// Folders of approved references, relative to the project.
    #[serde(default)]
    pub reference_dirs: Vec<String>,
    /// Rubric id for the visual review.
    pub rubric: String,
    /// Prompt rules.
    pub prompt: PromptSpec,
    /// True when the art must be converted to black and white (news pictures).
    #[serde(default)]
    pub grayscale: bool,
    /// Size the model is asked for when it differs from the asset's canvas (an advisor card is made from a tall
    /// portrait, not from a 65x67 picture).
    #[serde(default)]
    pub source_canvas: Option<[u32; 2]>,
    /// The `frameAnimatedSpriteType` fields and frame motion of an animated family (`None` for every other one).
    #[serde(default)]
    pub animation: Option<AnimationSpec>,
}

/// The vanilla measurement a default was built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VanillaDefault {
    /// Canvas per frame.
    pub canvas: [u32; 2],
    /// Evidence sentence.
    pub source: String,
}

/// An effective profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Profile {
    /// Profile id: always the family id.
    pub id: String,
    /// The family. Fixed for the life of an asset.
    pub family: Family,
    /// Canvas per frame for new art.
    pub canvas: [u32; 2],
    /// Every size treated as valid per frame.
    pub accepted: Vec<[u32; 2]>,
    /// Minimum transparent margin in pixels.
    pub margin: u32,
    /// Frames in a horizontal strip.
    pub frames: u32,
    /// Evidence for the numbers above.
    pub source: String,
    /// Where the geometry came from.
    pub origin: ProfileOrigin,
    /// Fields changed relative to the built-in default.
    #[serde(default)]
    pub overridden: Vec<String>,
    /// Built-in vanilla-measured default, kept so a kit or project value can be compared with it.
    pub vanilla: Option<VanillaDefault>,
    /// Generation, export and naming details.
    pub imagegen: ImagegenSpec,
    /// The style pack applied to this profile and its processing switches (`None` without a pack). Not written when
    /// unset, so the hash of a profile without a pack is what it was before packs existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<crate::pack::PackProfile>,
}

impl Profile {
    /// The processing switches of the applied style pack (all off without a pack).
    pub fn processing(&self) -> crate::pack::ProcessingSwitches {
        self.pack.as_ref().map(|p| p.processing).unwrap_or_default()
    }

    /// Full canvas width including all frames.
    pub fn strip_width(&self) -> u32 {
        self.canvas[0] * self.frames.max(1)
    }

    /// True when the family has no fixed canvas and the mod must define one (GUI textures, super events).
    pub fn needs_mod_canvas(&self) -> bool {
        self.canvas == [0, 0]
    }

    /// Every size of a whole image (all frames) the family accepts, canonical size first.
    pub fn accepted_strips(&self) -> Vec<[u32; 2]> {
        let f = self.frames.max(1);
        let mut out: Vec<[u32; 2]> = Vec::new();
        for a in std::iter::once(&self.canvas).chain(self.accepted.iter()) {
            if *a == [0, 0] {
                continue;
            }
            let strip = [a[0] * f, a[1]];
            if !out.contains(&strip) {
                out.push(strip);
            }
        }
        out
    }

    /// True when a plan may name its own canvas: families with none, and animated sprites, whose size vanilla takes
    /// from the GUI element they sit on (66 two-frame glow textures come in 47 sizes).
    pub fn free_canvas(&self) -> bool {
        self.needs_mod_canvas() || self.imagegen.animation.is_some()
    }

    /// True when the model is asked for a transparent background. A blend glow is drawn as light on black instead,
    /// which the tool turns into alpha: a soft glow is what a model's cut-out alpha renders worst.
    pub fn source_transparent(&self) -> bool {
        self.imagegen.background == Background::Transparent && self.imagegen.animation.as_ref().map(|a| a.motion) != Some(Motion::BlendGlow)
    }

    /// True when the effective canvas differs from the vanilla-measured default.
    pub fn differs_from_vanilla(&self) -> bool {
        self.vanilla.as_ref().map(|v| v.canvas != self.canvas).unwrap_or(false)
    }
}

/// Partial `imagegen` object as it may appear in a kit row or a project override.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ImagegenOverride {
    /// Automation.
    pub automation: Option<Automation>,
    /// Background.
    pub background: Option<Background>,
    /// Container.
    pub container: Option<Container>,
    /// Folder pattern.
    pub folder: Option<String>,
    /// Prefix pattern.
    pub prefix: Option<String>,
    /// Alternative prefixes.
    pub alt_prefixes: Option<Vec<String>>,
    /// Sprite details.
    pub sprite: Option<SpriteSpec>,
    /// Companion file suffixes.
    pub companion_files: Option<Vec<String>>,
    /// Variants.
    pub variants: Option<Vec<Variant>>,
    /// Checks.
    pub checks: Option<Vec<String>>,
    /// Reference folders.
    pub reference_dirs: Option<Vec<String>>,
    /// Rubric id.
    pub rubric: Option<String>,
    /// Prompt rules.
    pub prompt: Option<PromptSpec>,
    /// Grayscale flag.
    pub grayscale: Option<bool>,
    /// Size the model is asked for.
    pub source_canvas: Option<[u32; 2]>,
    /// Animation fields.
    pub animation: Option<AnimationSpec>,
}

/// A kit row or a project override: geometry plus an optional `imagegen` object. Unknown keys are ignored,
/// which is what lets the kit's `gfx.py` and this crate share one file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RowOverride {
    /// Canvas per frame.
    pub canvas: Option<[u32; 2]>,
    /// Accepted sizes.
    pub accepted: Option<Vec<[u32; 2]>>,
    /// Margin.
    pub margin: Option<u32>,
    /// Frames.
    pub frames: Option<u32>,
    /// Evidence.
    pub source: Option<String>,
    /// Generator-only fields.
    pub imagegen: Option<ImagegenOverride>,
}

/// Map a kit `profiles.json` row name to a family.
pub fn kit_row_family(name: &str) -> Option<Family> {
    Some(match name {
        "category" => Family::DecisionCategoryIcon,
        "report" => Family::ReportEvent,
        "news" => Family::NewsEvent,
        "counter-large" => Family::CounterLarge,
        "counter-small" => Family::CounterSmall,
        "focus" => Family::Focus,
        "decision" => Family::Decision,
        "idea" => Family::IdeaSpirit,
        other => return other.parse().ok(),
    })
}

/// The effective profile set.
#[derive(Debug, Clone)]
pub struct ProfileSet {
    profiles: BTreeMap<String, Profile>,
    /// Path of the kit file that was read, if any.
    pub kit_path: Option<String>,
    /// SHA-256 of that file's bytes.
    pub kit_sha256: Option<String>,
    /// Notes produced while merging (unknown rows, ignored keys).
    pub notes: Vec<String>,
}

impl ProfileSet {
    /// The built-in set only.
    pub fn builtin() -> Self {
        let profiles = crate::builtin::profiles().into_iter().map(|p| (p.id.clone(), p)).collect();
        ProfileSet { profiles, kit_path: None, kit_sha256: None, notes: Vec::new() }
    }

    /// Built-in defaults, then the kit file (when given and present), then project overrides.
    pub fn load(kit_json: Option<&Path>, project: &BTreeMap<String, RowOverride>) -> Result<Self> {
        let mut set = ProfileSet::builtin();
        if let Some(path) = kit_json {
            if path.is_file() {
                let bytes = crate::fsio::read_limited(path, 4 * 1024 * 1024)?;
                set.kit_sha256 = Some(sha256_hex(&bytes));
                set.kit_path = Some(crate::fsio::display_path(path));
                let doc: Value = serde_json::from_slice(&bytes).map_err(|e| Error::Config(format!("{} is not valid JSON: {e}", path.display())))?;
                let rows = doc
                    .get("profiles")
                    .and_then(Value::as_object)
                    .ok_or_else(|| Error::Config(format!("{} has no 'profiles' object", path.display())))?;
                for (name, row) in rows {
                    let parsed: RowOverride =
                        serde_json::from_value(row.clone()).map_err(|e| Error::Config(format!("profiles.json row '{name}': {e}")))?;
                    set.apply(name, &parsed, ProfileOrigin::Kit)?;
                }
            } else {
                set.notes.push(format!("kit profiles file {} not found; using built-in defaults", path.display()));
            }
        }
        for (name, row) in project {
            set.apply(name, row, ProfileOrigin::Project)?;
        }
        Ok(set)
    }

    fn apply(&mut self, name: &str, row: &RowOverride, origin: ProfileOrigin) -> Result<()> {
        let Some(family) = kit_row_family(name) else {
            // The 0.1 "custom" profiles are gone: a row must name a family, or it cannot be generated or checked.
            self.notes.push(format!("row '{name}' is not a known family; ignored"));
            return Ok(());
        };
        let id = family.as_str().to_string();
        let p = self.profiles.get_mut(&id).expect("every family has a built-in profile");
        let mut changed = Vec::new();
        if let Some(c) = row.canvas {
            if p.canvas != c {
                changed.push("canvas".to_string());
            }
            p.canvas = c;
            if row.accepted.is_none() && !p.accepted.contains(&c) {
                p.accepted.insert(0, c);
            }
        }
        if let Some(a) = &row.accepted {
            if &p.accepted != a {
                changed.push("accepted".to_string());
            }
            p.accepted = a.clone();
        }
        if let Some(m) = row.margin {
            if p.margin != m {
                changed.push("margin".to_string());
            }
            p.margin = m;
        }
        if let Some(f) = row.frames {
            if p.frames != f {
                changed.push("frames".to_string());
            }
            p.frames = f.max(1);
        }
        if let Some(s) = &row.source {
            p.source = s.clone();
        }
        if let Some(i) = &row.imagegen {
            merge_imagegen(&mut p.imagegen, i, &mut changed);
        }
        p.origin = origin;
        for c in changed {
            if !p.overridden.contains(&c) {
                p.overridden.push(c);
            }
        }
        Ok(())
    }

    /// Profile by id.
    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.get(id)
    }

    /// Profile of a family (the profile whose id is the family id).
    pub fn for_family(&self, family: Family) -> Option<&Profile> {
        self.profiles.get(family.as_str())
    }

    /// All profiles in id order.
    pub fn all(&self) -> impl Iterator<Item = &Profile> {
        self.profiles.values()
    }

    /// Hash of the effective set; plans record it so a profile change invalidates them.
    pub fn effective_hash(&self) -> String {
        let value = serde_json::to_value(self.profiles.values().collect::<Vec<_>>()).unwrap_or(Value::Null);
        canonical_hash(&value)
    }

    /// Hash of one effective profile.
    pub fn profile_hash(profile: &Profile) -> String {
        canonical_hash(&serde_json::to_value(profile).unwrap_or(Value::Null))
    }
}

fn merge_imagegen(dst: &mut ImagegenSpec, src: &ImagegenOverride, changed: &mut Vec<String>) {
    macro_rules! set {
        ($field:ident, $label:literal) => {
            if let Some(v) = &src.$field {
                if &dst.$field != v {
                    changed.push(concat!("imagegen.", $label).to_string());
                }
                dst.$field = v.clone();
            }
        };
    }
    set!(automation, "automation");
    set!(background, "background");
    set!(container, "container");
    set!(folder, "folder");
    set!(prefix, "prefix");
    set!(alt_prefixes, "alt_prefixes");
    if let Some(v) = &src.sprite {
        if dst.sprite.as_ref() != Some(v) {
            changed.push("imagegen.sprite".to_string());
        }
        dst.sprite = Some(v.clone());
    }
    set!(companion_files, "companion_files");
    set!(variants, "variants");
    set!(checks, "checks");
    set!(reference_dirs, "reference_dirs");
    set!(rubric, "rubric");
    set!(prompt, "prompt");
    set!(grayscale, "grayscale");
    if let Some(v) = &src.animation {
        if dst.animation.as_ref() != Some(v) {
            changed.push("imagegen.animation".to_string());
        }
        dst.animation = Some(v.clone());
    }
    if let Some(v) = src.source_canvas {
        if dst.source_canvas != Some(v) {
            changed.push("imagegen.source_canvas".to_string());
        }
        dst.source_canvas = Some(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit_file(dir: &Path, body: &str) -> std::path::PathBuf {
        let p = dir.join("profiles.json");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn builtin_set_covers_every_family_with_a_profile() {
        let set = ProfileSet::builtin();
        for c in Family::ALL.iter().copied() {
            let p = set.for_family(c).unwrap_or_else(|| panic!("no profile for {c}"));
            assert_eq!(p.family, c);
            assert_eq!(p.id, c.as_str());
            assert!(p.accepted.contains(&p.canvas) || p.needs_mod_canvas(), "{c}");
            assert!(!p.source.is_empty());
        }
        assert_eq!(set.for_family(Family::Focus).unwrap().canvas, [100, 88]);
        assert_eq!(set.for_family(Family::IdeaSpirit).unwrap().canvas, [60, 68]);
        assert_eq!(set.for_family(Family::Decision).unwrap().canvas, [33, 32]);
        assert_eq!(set.all().count(), Family::ALL.len());
    }

    #[test]
    fn kit_rows_override_geometry_and_keep_unknown_keys_harmless() {
        let dir = tempfile::tempdir().unwrap();
        let path = kit_file(
            dir.path(),
            r#"{"_about":"x","profiles":{
                "focus":{"canvas":[94,86],"accepted":[[94,86],[100,88]],"margin":2,"frames":1,"source":"project decision","off_size_level":"error",
                         "imagegen":{"prefix":"focus_"}},
                "report":{"canvas":[210,176],"accepted":[[210,176],[209,176]],"margin":0,"frames":1,"source":"kit"}
            }}"#,
        );
        let set = ProfileSet::load(Some(&path), &BTreeMap::new()).unwrap();
        let focus = set.for_family(Family::Focus).unwrap();
        assert_eq!(focus.canvas, [94, 86]);
        assert_eq!(focus.origin, ProfileOrigin::Kit);
        assert!(focus.overridden.contains(&"canvas".to_string()));
        assert_eq!(focus.imagegen.prefix, "focus_");
        assert!(focus.differs_from_vanilla());
        assert_eq!(focus.vanilla.as_ref().unwrap().canvas, [100, 88]);
        let report = set.for_family(Family::ReportEvent).unwrap();
        assert!(report.overridden.is_empty(), "same numbers as the default are not an override: {:?}", report.overridden);
        assert!(set.kit_sha256.is_some());
    }

    #[test]
    fn project_overrides_win_over_the_kit() {
        let dir = tempfile::tempdir().unwrap();
        let path = kit_file(dir.path(), r#"{"profiles":{"idea":{"canvas":[64,64],"accepted":[[64,64]],"margin":2,"frames":1,"source":"kit"}}}"#);
        let mut project = BTreeMap::new();
        project.insert("idea_spirit".to_string(), RowOverride { canvas: Some([62, 70]), ..Default::default() });
        let set = ProfileSet::load(Some(&path), &project).unwrap();
        let p = set.for_family(Family::IdeaSpirit).unwrap();
        assert_eq!(p.canvas, [62, 70]);
        assert_eq!(p.origin, ProfileOrigin::Project);
        assert!(p.accepted.contains(&[62, 70]));
    }

    #[test]
    fn unknown_rows_are_ignored_with_a_note_and_bad_files_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = kit_file(dir.path(), r#"{"profiles":{"my-banner":{"canvas":[300,100],"margin":0,"frames":1,"source":"mine"}}}"#);
        let set = ProfileSet::load(Some(&path), &BTreeMap::new()).unwrap();
        assert!(set.get("my-banner").is_none(), "no custom family exists");
        assert!(set.notes.iter().any(|n| n.contains("my-banner")));
        assert_eq!(set.all().count(), Family::ALL.len());

        let notjson = kit_file(dir.path(), "not json");
        assert!(ProfileSet::load(Some(&notjson), &BTreeMap::new()).is_err());
        let missing = dir.path().join("absent.json");
        let set = ProfileSet::load(Some(&missing), &BTreeMap::new()).unwrap();
        assert!(set.kit_path.is_none() && !set.notes.is_empty());
    }

    #[test]
    fn a_mod_defines_the_canvas_of_gui_textures_through_its_profile() {
        let mut project = BTreeMap::new();
        assert!(ProfileSet::builtin().for_family(Family::GuiTexture).unwrap().needs_mod_canvas());
        project.insert("gui_texture".to_string(), RowOverride { canvas: Some([300, 100]), ..Default::default() });
        let set = ProfileSet::load(None, &project).unwrap();
        let p = set.for_family(Family::GuiTexture).unwrap();
        assert_eq!(p.canvas, [300, 100]);
        assert!(!p.needs_mod_canvas());
    }

    #[test]
    fn accepted_strips_multiply_by_frames() {
        let set = ProfileSet::builtin();
        assert_eq!(set.for_family(Family::CounterLarge).unwrap().accepted_strips(), vec![[152, 42]]);
        assert_eq!(set.for_family(Family::Building).unwrap().accepted_strips(), vec![[128, 64], [64, 32]]);
        assert_eq!(set.for_family(Family::Focus).unwrap().accepted_strips()[0], [100, 88]);
    }

    #[test]
    fn effective_hash_changes_with_geometry() {
        let a = ProfileSet::builtin();
        let mut project = BTreeMap::new();
        project.insert("focus".to_string(), RowOverride { canvas: Some([94, 86]), ..Default::default() });
        let b = ProfileSet::load(None, &project).unwrap();
        assert_ne!(a.effective_hash(), b.effective_hash());
        assert_eq!(a.effective_hash(), ProfileSet::builtin().effective_hash());
    }
}
