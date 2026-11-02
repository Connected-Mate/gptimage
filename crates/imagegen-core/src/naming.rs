//! Names, patterns and ids. The kit's rule is lowercase snake_case, stable once wired into `.gfx`.

use crate::error::{Error, Result};

/// Longest accepted asset or feature slug.
pub const MAX_NAME_LEN: usize = 48;

const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5",
    "lpt6", "lpt7", "lpt8", "lpt9",
];

/// True for Windows reserved device names (with or without extension).
pub fn is_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("").to_ascii_lowercase();
    RESERVED.contains(&stem.as_str())
}

/// A lowercase snake_case identifier: starts with a letter, then letters, digits and underscores.
pub fn validate_snake(kind: &str, value: &str) -> Result<()> {
    let ok = !value.is_empty()
        && value.len() <= MAX_NAME_LEN
        && value.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false)
        && value.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !value.ends_with('_')
        && !value.contains("__");
    if !ok {
        return Err(Error::Invalid(format!(
            "{kind} '{value}' must be lowercase snake_case: a letter first, then letters, digits and single underscores, at most {MAX_NAME_LEN} characters"
        )));
    }
    if is_reserved_name(value) {
        return Err(Error::Invalid(format!("{kind} '{value}' is a reserved device name")));
    }
    Ok(())
}

/// Flag names are a country tag (three uppercase letters or digits) with an optional lowercase ideology
/// suffix, for example `GER` or `GER_fascism`.
pub fn validate_flag_name(value: &str) -> Result<()> {
    let (tag, rest) = match value.split_once('_') {
        Some((t, r)) => (t, Some(r)),
        None => (value, None),
    };
    let tag_ok = tag.len() == 3 && tag.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let rest_ok =
        rest.map(|r| !r.is_empty() && r.len() <= 30 && r.chars().all(|c| c.is_ascii_lowercase() || c == '_') && !r.ends_with('_')).unwrap_or(true);
    if !tag_ok || !rest_ok {
        return Err(Error::Invalid(format!(
            "flag name '{value}' must be a three character country tag with an optional lowercase ideology suffix, such as GER or GER_fascism"
        )));
    }
    if is_reserved_name(tag) {
        return Err(Error::Invalid(format!("flag tag '{tag}' is a reserved device name")));
    }
    Ok(())
}

/// Expand `{feature_slug}`, `{asset}`, `{ASSET}` (the asset name in upper case, for the vanilla
/// `GFX_MODIFIER_<KEY>` convention) and `{prefix}` in a pattern. Unknown placeholders are an error.
pub fn expand(pattern: &str, feature_slug: &str, asset: &str, prefix: &str) -> Result<String> {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('}').ok_or_else(|| Error::Invalid(format!("unclosed placeholder in pattern '{pattern}'")))?;
        match &after[..end] {
            "feature_slug" => out.push_str(feature_slug),
            "asset" => out.push_str(asset),
            "ASSET" => out.push_str(&asset.to_ascii_uppercase()),
            "prefix" => out.push_str(prefix),
            other => return Err(Error::Invalid(format!("unknown placeholder '{{{other}}}' in pattern '{pattern}'"))),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `feature.asset`, the stable id of one asset in the ledger.
pub fn asset_id(feature_slug: &str, asset: &str) -> String {
    format!("{feature_slug}.{asset}")
}

/// Split an asset id back into slug and asset name.
pub fn split_asset_id(id: &str) -> Result<(&str, &str)> {
    id.split_once('.').ok_or_else(|| Error::Invalid(format!("'{id}' is not an asset id (feature.asset)")))
}

/// Id of the raw candidate at a version: `feature.asset-v3`.
pub fn candidate_id(asset_id: &str, version: u32) -> String {
    format!("{asset_id}-v{version}")
}

/// Id of a processed artifact derived from a candidate: `feature.asset-v3-p1`.
pub fn processed_id(candidate: &str, run: u32) -> String {
    format!("{candidate}-p{run}")
}

/// Parsed artifact id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactId {
    /// `feature.asset`
    pub asset_id: String,
    /// Raw candidate version.
    pub version: u32,
    /// Processing run, when this is a processed artifact.
    pub processed_run: Option<u32>,
}

impl ArtifactId {
    /// Parse `feature.asset-vN` or `feature.asset-vN-pM`.
    pub fn parse(id: &str) -> Result<Self> {
        let bad = || Error::Invalid(format!("'{id}' is not a candidate id (feature.asset-vN or feature.asset-vN-pM)"));
        let (head, run) = match id.rsplit_once("-p") {
            Some((h, m)) if m.chars().all(|c| c.is_ascii_digit()) && !m.is_empty() => (h, Some(m.parse::<u32>().map_err(|_| bad())?)),
            _ => (id, None),
        };
        let (asset, v) = head.rsplit_once("-v").ok_or_else(bad)?;
        if v.is_empty() || !v.chars().all(|c| c.is_ascii_digit()) || !asset.contains('.') {
            return Err(bad());
        }
        Ok(ArtifactId { asset_id: asset.to_string(), version: v.parse().map_err(|_| bad())?, processed_run: run })
    }

    /// The raw candidate id this artifact derives from.
    pub fn raw_id(&self) -> String {
        candidate_id(&self.asset_id, self.version)
    }
}

/// Find the highest `stem-vN.ext` version among file names.
pub fn highest_version<'a>(names: impl IntoIterator<Item = &'a str>, stem: &str, ext: &str) -> u32 {
    let prefix = format!("{stem}-v");
    let suffix = format!(".{ext}");
    names.into_iter().filter_map(|n| n.strip_prefix(&prefix)?.strip_suffix(&suffix)?.parse::<u32>().ok()).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_names() {
        for ok in ["a", "iron_wall", "x1", "march_of_the_wolves"] {
            validate_snake("asset", ok).unwrap();
        }
        for bad in ["", "Iron", "1x", "a__b", "a_", "has space", "a-b", "../x", "nul", "con", &"a".repeat(49)] {
            assert!(validate_snake("asset", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn flag_names() {
        for ok in ["GER", "GER_fascism", "A1B", "SOV_communism_old"] {
            validate_flag_name(ok).unwrap();
        }
        for bad in ["ger", "GERM", "GE", "GER_", "GER_Fascism", "GER-fascism", "NUL"] {
            assert!(validate_flag_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pattern_expansion() {
        assert_eq!(
            expand("gfx/interface/goals/{feature_slug}/{prefix}{asset}", "iron", "wall", "goal_").unwrap(),
            "gfx/interface/goals/iron/goal_wall"
        );
        assert_eq!(expand("GFX_MODIFIER_{ASSET}", "iron", "army_speed", "").unwrap(), "GFX_MODIFIER_ARMY_SPEED");
        assert!(expand("{nope}", "a", "b", "c").is_err());
        assert!(expand("{asset", "a", "b", "c").is_err());
    }

    #[test]
    fn artifact_ids() {
        let id = ArtifactId::parse("iron.wall-v12-p3").unwrap();
        assert_eq!((id.asset_id.as_str(), id.version, id.processed_run), ("iron.wall", 12, Some(3)));
        assert_eq!(id.raw_id(), "iron.wall-v12");
        let raw = ArtifactId::parse("iron.wall-v2").unwrap();
        assert_eq!(raw.processed_run, None);
        for bad in ["wall-v2", "iron.wall", "iron.wall-vx", "iron.wall-v", ""] {
            assert!(ArtifactId::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(candidate_id("a.b", 4), "a.b-v4");
        assert_eq!(processed_id("a.b-v4", 2), "a.b-v4-p2");
        assert_eq!(split_asset_id("a.b").unwrap(), ("a", "b"));
    }

    #[test]
    fn highest_version_scan() {
        let names = ["wall-v1.png", "wall-v3.png", "wall-v10.png", "other-v99.png", "wall-v2.dds", "wall.png"];
        assert_eq!(highest_version(names, "wall", "png"), 10);
        assert_eq!(highest_version(names, "none", "png"), 0);
    }
}
