//! The vanilla measurement the built-in family table is derived from.
//!
//! `data/vanilla-measurement.json` was produced by `scripts/measure_families.py` from the installed game
//! (HOI4 1.19.4.1, every texture of the surveyed folders, pixels sampled). It is embedded here so that a
//! family's `source` text cites real counts, `hoi4.image_families` can show the distribution, and a unit test
//! can fail the build when a re-measure changes what vanilla looks like (see `builtin.rs`).

use std::sync::OnceLock;

use serde_json::Value;

use crate::family::Family;

const DATA: &str = include_str!("../data/vanilla-measurement.json");

/// The parsed measurement.
#[derive(Debug)]
pub struct Measurement {
    root: Value,
}

/// The embedded measurement.
pub fn data() -> &'static Measurement {
    static CELL: OnceLock<Measurement> = OnceLock::new();
    CELL.get_or_init(|| Measurement { root: serde_json::from_str(DATA).expect("data/vanilla-measurement.json is valid JSON") })
}

/// Parse `WxH`.
pub fn parse_size(s: &str) -> Option<[u32; 2]> {
    let (w, h) = s.split_once('x')?;
    Some([w.parse().ok()?, h.parse().ok()?])
}

impl Measurement {
    /// Game build the data was measured from.
    pub fn game_version(&self) -> &str {
        self.root.get("game_version").and_then(Value::as_str).unwrap_or("")
    }

    /// Ids of every measured entry.
    pub fn entry_ids(&self) -> Vec<&str> {
        self.entries().filter_map(|e| e.get("id").and_then(Value::as_str)).collect()
    }

    fn entries(&self) -> impl Iterator<Item = &Value> {
        self.root.get("families").and_then(Value::as_array).into_iter().flatten()
    }

    /// One measured entry by id.
    pub fn entry(&self, id: &str) -> Option<&Value> {
        self.entries().find(|e| e.get("id").and_then(Value::as_str) == Some(id))
    }

    /// Number of textures measured in the entry.
    pub fn count(&self, id: &str) -> Option<u64> {
        self.entry(id)?.get("measured_texture_count")?.as_u64()
    }

    /// Dominant whole-image size of the entry and its file count.
    pub fn dominant(&self, id: &str) -> Option<([u32; 2], u64)> {
        let e = self.entry(id)?;
        let size = parse_size(e.get("canvas_dominant")?.as_str()?)?;
        Some((size, e.get("canvas_dominant_count")?.as_u64()?))
    }

    /// The most common whole-image sizes with their file counts, most common first.
    pub fn distribution(&self, id: &str) -> Vec<([u32; 2], u64)> {
        let Some(map) = self.entry(id).and_then(|e| e.get("canvas_distribution_top")).and_then(Value::as_object) else { return Vec::new() };
        let mut v: Vec<([u32; 2], u64)> = map.iter().filter_map(|(k, n)| Some((parse_size(k)?, n.as_u64()?))).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// Files of the entry at a whole-image size, when that size is among the most common ones.
    pub fn count_at(&self, id: &str, size: [u32; 2]) -> Option<u64> {
        self.distribution(id).into_iter().find(|(s, _)| *s == size).map(|(_, n)| n)
    }

    /// The entry's pixel class (`transparent background`, `opaque canvas (no transparent px)`, ...).
    pub fn alpha_class(&self, id: &str) -> Option<&str> {
        self.entry(id)?.get("pixels")?.get("alpha_class")?.as_str()
    }

    /// `noOfFrames` values the entry's sprites register (`1(none)` when absent).
    pub fn frame_values(&self, id: &str) -> Vec<String> {
        self.entry(id)
            .and_then(|e| e.get("registration"))
            .and_then(|r| r.get("noOfFrames_values"))
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The most common value of a registration count map of the entry (`animation_rate_fps_values`,
    /// `effect_file_values`, ...), with its count.
    pub fn registration_mode(&self, id: &str, key: &str) -> Option<(String, u64)> {
        let map = self.entry(id)?.get("registration")?.get(key)?.as_object()?;
        map.iter().filter_map(|(k, n)| Some((k.clone(), n.as_u64()?))).max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
    }

    /// A number of an animated entry's `animation` object (`median_dim_to_bright_alpha_ratio`, ...).
    pub fn animation_number(&self, id: &str, key: &str) -> Option<f64> {
        self.entry(id)?.get("animation")?.get(key)?.as_f64()
    }

    /// Texture folders the entry measured.
    pub fn folders(&self, id: &str) -> Vec<String> {
        self.entry(id)
            .and_then(|e| e.get("texture_folders"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Family table of the idea measurement (`industrial_concern`, `political_advisor`, ...): the dominant size.
    pub fn idea_category_dominant(&self, category: &str) -> Option<[u32; 2]> {
        let dims = self.root.get("idea_categories")?.get(category)?.get("dims")?.as_object()?;
        dims.iter().filter_map(|(k, n)| Some((parse_size(k)?, n.as_u64()?))).max_by_key(|(_, n)| *n).map(|(s, _)| s)
    }

    /// One sentence that cites the measurement of an entry: game build, file count and the leading sizes.
    pub fn summary(&self, id: &str) -> Option<String> {
        let count = self.count(id)?;
        let (dom, dn) = self.dominant(id)?;
        let others: Vec<String> =
            self.distribution(id).into_iter().filter(|(s, _)| *s != dom).take(3).map(|(s, n)| format!("{}x{}: {n}", s[0], s[1])).collect();
        let pct = if count == 0 { 0.0 } else { dn as f64 * 100.0 / count as f64 };
        let mut s = format!(
            "Measured on HOI4 {} ({}): {count} textures in {}; {}x{} is {dn} files ({pct:.1} percent)",
            self.game_version().split(' ').next().unwrap_or(""),
            id,
            self.folders(id).join(", "),
            dom[0],
            dom[1]
        );
        if !others.is_empty() {
            s.push_str(&format!("; next: {}", others.join(", ")));
        }
        s.push('.');
        Some(s)
    }
}

/// Measured entries a family is derived from (empty for the families the mod defines itself).
pub fn entries_of(family: Family) -> &'static [&'static str] {
    use Family::*;
    match family {
        Focus => &["focus_icon"],
        IdeaSpirit | IdeaConcern => &["idea_spirit_and_advisor"],
        OfficerCorpsSpirit => &["officer_corps_spirit"],
        Decision => &["decision_icon"],
        DecisionCategoryIcon => &["decision_category_icon"],
        StateModifier => &["dynamic_and_state_modifier_icon"],
        SpecialProjectIcon => &["special_project_icon"],
        Raid => &["raid_icons"],
        MioDepartment => &["mio_department_icon"],
        MioTrait | MioPolicy => &["mio_trait_policy_icon"],
        BopSide => &["bop_side_icon"],
        Medal => &["medal_icon"],
        AgencyUpgrade => &["intel_agency_upgrade_icon"],
        OperationIcon | OperationPicture => &["operation_icon"],
        CommanderTrait => &["commander_trait_icon"],
        PersonalTrait => &["personal_trait_icon"],
        Doctrine => &["doctrine_icon"],
        ReportEvent => &["event_picture_report"],
        NewsEvent => &["event_picture_news"],
        DecisionCategoryPicture => &["decision_category_picture"],
        Technology => &["technology_picture"],
        Achievement => &["achievement_icon"],
        LeaderPortrait | CommanderPortrait => &["leader_portrait"],
        OperativePortrait => &["operative_portrait"],
        ScientistPortrait => &["scientist_portrait"],
        AdvisorCard => &["advisor_character_small"],
        Flag => &["flag_large", "flag_medium", "flag_small"],
        CounterLarge | CounterSmall => &["unit_counter"],
        DivisionEmblem => &["division_template_emblem"],
        Building => &["building_icon"],
        FactionLogo => &["faction_icon"],
        AgencyLogo => &["intel_agency_logo"],
        ModifierStatIcon => &["stat_modifier_tooltip_icon"],
        GuiTexture | SuperEvent => &[],
        AnimBlendGlow => &["anim_blend_glow"],
        AnimBlendKeyframes => &["anim_blend_keyframes"],
        AnimScroll => &["anim_scroll"],
        AnimSpin => &["anim_spin"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_measurement_is_the_1_19_4_1_run() {
        let m = data();
        assert!(m.game_version().starts_with("1.19.4.1"), "{}", m.game_version());
        assert_eq!(m.entry_ids().len(), 45);
        assert_eq!(m.dominant("focus_icon").unwrap().0, [100, 88]);
        assert_eq!(parse_size("146x54"), Some([146, 54]));
        assert!(m.summary("decision_icon").unwrap().contains("33x32 is 223 files"));
        assert_eq!(m.idea_category_dominant("industrial_concern"), Some([64, 64]));
    }

    #[test]
    fn every_mapped_entry_exists() {
        let m = data();
        for f in Family::ALL.iter().copied() {
            for id in entries_of(f) {
                assert!(m.entry(id).is_some(), "{f}: no measured entry '{id}'");
            }
        }
    }
}
