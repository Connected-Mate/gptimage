//! Project style profiles: `.hoi4-imagegen/styles/<name>.json`.
//!
//! A style profile is project-owned data with no authority of its own: reference images pinned by SHA-256,
//! prompt fragments, negative rules, palette notes and the provider that suits the style. It never changes
//! what an agent is allowed to do.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::family::Family;
use crate::fsio::ProjectFs;
use crate::hashing::sha256_hex;
use crate::naming::validate_snake;

/// Folder of style files.
pub const STYLE_DIR: &str = ".hoi4-imagegen/styles";

/// Why a style was applied to a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StyleWhy {
    /// The caller named it (`style: <name>`).
    Explicit,
    /// The style has `default: true`.
    Default,
    /// The style's non-empty `families` list names the family.
    FamilyList,
    /// No style was applied.
    None,
}

impl StyleWhy {
    /// The id used in plans.
    pub fn as_str(self) -> &'static str {
        match self {
            StyleWhy::Explicit => "explicit",
            StyleWhy::Default => "default",
            StyleWhy::FamilyList => "family_list",
            StyleWhy::None => "none",
        }
    }
}

/// A pinned reference image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StyleRef {
    /// Project-relative path.
    pub path: String,
    /// SHA-256 of the file when it was approved.
    pub sha256: String,
    /// Why the image is in the set (rights note, source).
    #[serde(default)]
    pub note: String,
}

/// A style profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StyleProfile {
    /// Schema version.
    #[serde(default = "one")]
    pub schema: u32,
    /// Name (snake_case); also the file stem.
    pub name: String,
    /// What the style is for.
    #[serde(default)]
    pub description: String,
    /// Sentences added to every prompt.
    #[serde(default)]
    pub prompt_fragments: Vec<String>,
    /// Things to avoid.
    #[serde(default)]
    pub negative_rules: Vec<String>,
    /// Palette notes.
    #[serde(default)]
    pub palette_notes: String,
    /// Families the style is available to. Empty means available to any family, but never applied automatically
    /// (see [`StyleProfile::auto_applies_to`]). A non-empty list also makes the style the automatic style of the
    /// families it names.
    #[serde(default, alias = "categories")]
    pub families: Vec<Family>,
    /// `true` applies the style automatically to the families it lists, or to every family when it lists none.
    /// `false` turns automatic use off even for a listed family (the style stays available by name). Absent means
    /// "automatic only for the families the list names".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<bool>,
    /// Approved reference images, pinned by hash.
    #[serde(default)]
    pub references: Vec<StyleRef>,
    /// Preferred provider.
    #[serde(default)]
    pub provider: Option<String>,
    /// Preferred model.
    #[serde(default)]
    pub model: Option<String>,
    /// Seed policy note.
    #[serde(default)]
    pub seed_policy: Option<String>,
    /// Who approved the style.
    #[serde(default)]
    pub approved_by: Option<String>,
    /// When (RFC 3339).
    #[serde(default)]
    pub approved_on: Option<String>,
}

fn one() -> u32 {
    1
}

impl StyleProfile {
    /// Check names and sizes.
    pub fn validate(&self) -> Result<()> {
        validate_snake("style name", &self.name)?;
        if self.prompt_fragments.len() > 40 || self.negative_rules.len() > 40 || self.references.len() > 16 {
            return Err(Error::Invalid("a style may have at most 40 prompt fragments, 40 negative rules and 16 references".into()));
        }
        for f in self.prompt_fragments.iter().chain(&self.negative_rules) {
            if f.len() > 600 {
                return Err(Error::Invalid("a prompt fragment may be at most 600 characters".into()));
            }
        }
        Ok(())
    }

    /// True when the style may be chosen by name for a family (`style: <name>`): always, unless a non-empty
    /// `families` list leaves the family out.
    pub fn applies_to(&self, c: Family) -> bool {
        self.families.is_empty() || self.families.contains(&c)
    }

    /// Why the style is applied automatically to a family, or `None` when it is only available by name.
    ///
    /// * `default: false` never applies automatically.
    /// * `default: true` applies to the listed families, or to every family when the list is empty.
    /// * No `default` key: applies only to the families a non-empty list names. An empty list means "available to
    ///   any family, never applied automatically".
    pub fn auto_applies_to(&self, c: Family) -> Option<StyleWhy> {
        let listed = self.families.contains(&c);
        match self.default {
            Some(false) => None,
            Some(true) if self.families.is_empty() || listed => Some(StyleWhy::Default),
            Some(true) => None,
            None if listed => Some(StyleWhy::FamilyList),
            None => None,
        }
    }

    /// Check that every pinned reference still matches its recorded hash.
    pub fn verify_references(&self, fs: &ProjectFs) -> Result<()> {
        for r in &self.references {
            let bytes = fs.read(&r.path, crate::refs::MAX_REFERENCE_BYTES)?;
            let now = sha256_hex(&bytes);
            if now != r.sha256 {
                return Err(Error::refused(
                    "style_reference_changed",
                    format!(
                        "reference {} no longer matches the hash recorded when the style '{}' was approved; re-approve the style",
                        r.path, self.name
                    ),
                    crate::error::RetryClass::RefreshPreconditions,
                ));
            }
        }
        Ok(())
    }
}

/// Path of a style file.
pub fn style_path(name: &str) -> Result<String> {
    validate_snake("style name", name)?;
    Ok(format!("{STYLE_DIR}/{name}.json"))
}

/// Load a style by name.
pub fn load(fs: &ProjectFs, name: &str) -> Result<StyleProfile> {
    let rel = style_path(name)?;
    let bytes = fs.read(&rel, 256 * 1024).map_err(|e| match e {
        Error::NotFound(_) => Error::NotFound(format!("style '{name}' (expected {rel})")),
        other => other,
    })?;
    let style: StyleProfile = serde_json::from_slice(&bytes).map_err(|e| Error::Config(format!("{rel}: {e}")))?;
    if style.name != name {
        return Err(Error::Config(format!("{rel} declares the name '{}'", style.name)));
    }
    style.validate()?;
    Ok(style)
}

/// Names of the saved styles.
pub fn list(fs: &ProjectFs) -> Result<Vec<String>> {
    let mut names: Vec<String> = fs.list_names(STYLE_DIR)?.into_iter().filter_map(|n| n.strip_suffix(".json").map(str::to_string)).collect();
    names.sort();
    Ok(names)
}

/// Save a style. An existing style is replaced only when `replace` is true; the old file is kept as a
/// versioned backup next to it so nothing is lost.
pub fn save(fs: &ProjectFs, style: &StyleProfile, replace: bool) -> Result<String> {
    style.validate()?;
    let rel = style_path(&style.name)?;
    let mut body = serde_json::to_vec_pretty(style).map_err(|e| Error::Config(e.to_string()))?;
    body.push(b'\n');
    if fs.exists(&rel) {
        if !replace {
            return Err(Error::Exists(format!("style '{}' already exists; pass replace to overwrite it (the old file is backed up)", style.name)));
        }
        let old = fs.read(&rel, 256 * 1024)?;
        fs.write_next_version(&format!("{STYLE_DIR}/backup"), &style.name, "json", 0, &old)?;
        fs.replace(&rel, &body)?;
    } else {
        fs.write_new(&rel, &body)?;
    }
    Ok(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(name: &str) -> StyleProfile {
        StyleProfile {
            schema: 1,
            name: name.into(),
            description: "aged oil".into(),
            prompt_fragments: vec!["Aged oil painting look.".into()],
            negative_rules: vec!["no text".into()],
            palette_notes: "earth tones".into(),
            families: vec![Family::Focus],
            default: None,
            references: vec![],
            provider: Some("openai".into()),
            model: None,
            seed_policy: None,
            approved_by: Some("maintainer".into()),
            approved_on: Some("2026-10-31T00:00:00Z".into()),
        }
    }

    #[test]
    fn save_load_list_round_trip_and_backup_on_replace() {
        let dir = tempfile::tempdir().unwrap();
        let fs = ProjectFs::new(dir.path()).unwrap();
        let rel = save(&fs, &style("soviet_oil"), false).unwrap();
        assert_eq!(rel, ".hoi4-imagegen/styles/soviet_oil.json");
        assert_eq!(load(&fs, "soviet_oil").unwrap(), style("soviet_oil"));
        assert_eq!(list(&fs).unwrap(), vec!["soviet_oil"]);
        assert!(matches!(save(&fs, &style("soviet_oil"), false).unwrap_err(), Error::Exists(_)));
        let mut changed = style("soviet_oil");
        changed.description = "changed".into();
        save(&fs, &changed, true).unwrap();
        assert_eq!(load(&fs, "soviet_oil").unwrap().description, "changed");
        assert_eq!(fs.list_names(".hoi4-imagegen/styles/backup").unwrap(), vec!["soviet_oil-v1.json"]);
    }

    #[test]
    fn bad_names_and_missing_styles_fail() {
        let dir = tempfile::tempdir().unwrap();
        let fs = ProjectFs::new(dir.path()).unwrap();
        assert!(save(&fs, &style("../evil"), false).is_err());
        assert!(save(&fs, &style("Has Space"), false).is_err());
        assert!(matches!(load(&fs, "absent").unwrap_err(), Error::NotFound(_)));
        assert!(style_path("../x").is_err());
    }

    #[test]
    fn pinned_references_detect_edits() {
        let dir = tempfile::tempdir().unwrap();
        let fs = ProjectFs::new(dir.path()).unwrap();
        fs.write_new("references/a.png", b"original").unwrap();
        let mut s = style("pinned");
        s.references.push(StyleRef { path: "references/a.png".into(), sha256: sha256_hex(b"original"), note: "mine".into() });
        s.verify_references(&fs).unwrap();
        fs.replace("references/a.png", b"edited").unwrap();
        let err = s.verify_references(&fs).unwrap_err();
        assert_eq!(err.code(), "style_reference_changed");
    }

    #[test]
    fn applies_to_respects_categories() {
        let s = style("x");
        assert!(s.applies_to(Family::Focus) && !s.applies_to(Family::IdeaSpirit));
        let mut all = style("y");
        all.families.clear();
        assert!(all.applies_to(Family::IdeaSpirit));
    }

    #[test]
    fn automatic_use_needs_a_listed_family_or_default_true() {
        let listed = style("a"); // families = [Focus], default unset
        assert_eq!(listed.auto_applies_to(Family::Focus), Some(StyleWhy::FamilyList));
        assert_eq!(listed.auto_applies_to(Family::IdeaSpirit), None);
        let mut none = style("b");
        none.families.clear();
        assert_eq!(none.auto_applies_to(Family::Focus), None, "no families and no default: available by name, never automatic");
        none.default = Some(true);
        assert_eq!(none.auto_applies_to(Family::IdeaSpirit), Some(StyleWhy::Default));
        let mut dl = style("c");
        dl.default = Some(true);
        assert_eq!(dl.auto_applies_to(Family::Focus), Some(StyleWhy::Default));
        assert_eq!(dl.auto_applies_to(Family::IdeaSpirit), None, "default applies only to the listed families when there are some");
        dl.default = Some(false);
        assert_eq!(dl.auto_applies_to(Family::Focus), None);
        assert!(dl.applies_to(Family::Focus), "default = false still allows naming it");
    }

    #[test]
    fn the_default_key_is_optional_and_not_written_when_unset() {
        let old: StyleProfile = serde_json::from_str(r#"{"name":"old","categories":["focus"]}"#).unwrap();
        assert_eq!(old.default, None);
        assert_eq!(old.auto_applies_to(Family::Focus), Some(StyleWhy::FamilyList));
        assert!(!serde_json::to_string(&old).unwrap().contains("\"default\""));
        let new: StyleProfile = serde_json::from_str(r#"{"name":"new","default":true}"#).unwrap();
        assert_eq!(new.default, Some(true));
    }
}
