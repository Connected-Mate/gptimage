//! How Hearts of Iron IV 1.19.4.1 finds each family's art, the companions it looks up next to it, and what the
//! mod's script must say so the lookup succeeds.
//!
//! The families fall into five kinds of lookup (evidence is file and line in the installed game):
//!
//! * **Full name.** Script names the sprite in full, for example a focus `icon = GFX_focus_x`. Every name works as
//!   long as the script writes it exactly, so the handoff gives the exact text.
//! * **Short key.** Script gives a key and the engine adds the prefix: idea `picture = x` is `GFX_idea_x`,
//!   decision `icon = x` is `GFX_decision_x`, a category's `icon = x` is `GFX_decision_category_x` and its
//!   `picture = x` is `GFX_decision_cat_picture_x`. The prefixes are strings in `hoi4.exe`.
//! * **Engine key.** The engine builds the name from a database key, so the asset name must equal that key: the
//!   sub-unit (`GFX_unit_<key>_icon_medium`, `_icon_medium_white` and the text icon `GFX_unit_<key>_icon_small`),
//!   the leader trait (`GFX_trait_<key>`), the technology (`GFX_<key>_medium`), an idea without a `picture`
//!   (`GFX_idea_<key>`, which is how officer corps spirits are drawn) and a raid type (`GFX_raid_type_icon_<key>`).
//! * **Numbered.** Division template symbols are `GFX_div_templ_<n>_large` and `GFX_div_templ_<n>_small`; the
//!   player picks one by number in the division designer.
//! * **File name.** No sprite at all: achievements (`gfx/achievements/<key>.dds`) and flags (`gfx/flags/<TAG>.tga`).
//!
//! Sprite names are compared without case in places (vanilla characters reference 20 portrait sprites in a case
//! that differs from the definition), but nothing in vanilla proves the lookup is case-insensitive, so every
//! name here keeps vanilla's exact case: `GFX_MODIFIER_<KEY>` is upper case like the 159 vanilla ones.

use crate::family::Family;
use crate::recipes::derived::DeriveKind;

/// How the game finds a family's sprite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupKind {
    /// Script names the sprite in full: any name works when the script uses it exactly.
    FullName,
    /// Script gives a short key and the engine adds the prefix.
    ShortKey,
    /// The engine builds the name from a database key, which must be the asset name.
    EngineKey,
    /// The engine builds the name from a number the player picks.
    Numbered,
    /// The engine loads a file by name; there is no sprite.
    FileName,
}

impl LookupKind {
    /// Stable id.
    pub fn as_str(self) -> &'static str {
        match self {
            LookupKind::FullName => "full_name",
            LookupKind::ShortKey => "short_key",
            LookupKind::EngineKey => "engine_key",
            LookupKind::Numbered => "numbered",
            LookupKind::FileName => "file_name",
        }
    }
}

/// The vanilla lookup of one family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lookup {
    /// Kind of lookup.
    pub kind: LookupKind,
    /// What the mod writes so the game finds the art. `{sprite}` is the main sprite name, `{asset}` the asset name.
    pub script: &'static str,
    /// Where vanilla shows the lookup (installed game, 1.19.4.1).
    pub evidence: &'static str,
    /// An asset name for which the tool's sprite names must equal vanilla sprites, for the test against the game.
    /// `None` for families whose vanilla names are free-form.
    pub vanilla_example: Option<&'static str>,
}

/// The vanilla lookup of a family.
pub fn lookup_of(family: Family) -> Lookup {
    use Family::*;
    use LookupKind::*;
    let l = |kind, script, evidence, vanilla_example| Lookup { kind, script, evidence, vanilla_example };
    match family {
        Focus => l(FullName, "national focus: icon = {sprite}", "common/national_focus/*.txt: all 3,007 icon values are full GFX_ names (interface/goals.gfx:755 GFX_focus_generic_industry_1)", None),
        IdeaSpirit | IdeaConcern => l(ShortKey, "idea: picture = {asset}", "common/ideas/CSA.txt:8 picture = generic_air_manufacturer_2 -> interface/ideas.gfx:1069 GFX_idea_generic_air_manufacturer_2; 1,400 of 1,455 short picture keys resolve to GFX_idea_<key>; hoi4.exe has the string GFX_idea_", Some("generic_air_manufacturer_2")),
        OfficerCorpsSpirit => l(EngineKey, "name the officer corps spirit idea {asset} (an idea without picture is drawn with GFX_idea_<idea key>), or write picture = {asset}", "common/ideas/army_spirits.txt:4 bold_attack_spirit has no picture -> interface/officer_corp_spirits.gfx:15 GFX_idea_bold_attack_spirit", Some("bold_attack_spirit")),
        Decision => l(ShortKey, "decision: icon = {asset}", "common/decisions/ARG.txt:764 icon = oil -> interface/decisions.gfx:723 GFX_decision_oil; 91 of 104 short keys resolve; hoi4.exe has GFX_decision_", Some("oil")),
        DecisionCategoryIcon => l(ShortKey, "decision category: icon = {asset}", "common/decisions/categories/00_decision_categories.txt:16 icon = generic_propaganda -> interface/decisions.gfx:22 GFX_decision_category_generic_propaganda; hoi4.exe has GFX_decision_category_", Some("generic_propaganda")),
        DecisionCategoryPicture => l(ShortKey, "decision category: picture = {asset}", "common/decisions/categories/CHI_decision_categories.txt:166 picture = generic_border_conflicts -> interface/decisions.gfx:352 GFX_decision_cat_picture_generic_border_conflicts; hoi4.exe has GFX_decision_cat_picture_", Some("generic_border_conflicts")),
        StateModifier => l(FullName, "dynamic modifier: icon = {sprite}", "common/dynamic_modifiers/*.txt: 375 of 377 icon values are full GFX_ names (interface/countrystateview.gfx:750 GFX_modifiers_tur_kemalist_loyalties)", None),
        SpecialProjectIcon => l(FullName, "special project: icon = {sprite}", "common/special_projects/projects/air_projects.txt:1071 icon = GFX_sp_air_axial_jet_engine", None),
        Raid => l(EngineKey, "name the raid type {asset} (the map icon is looked up as GFX_raid_type_icon_<raid type>), or write custom_map_icon = {sprite}", "common/raids/_documentation.md:22 and :232 (automatic lookup GFX_raid_type_icon_ + raid type) -> interface/military_raids/military_raids.gfx:281 GFX_raid_type_icon_airfield_raid", Some("airfield_raid")),
        MioDepartment => l(FullName, "military industrial organization: icon = {sprite}", "common/military_industrial_organization/organizations/00_generic_organization.txt:5128 icon = GFX_generic_mio_department_icon_air_transport_production", None),
        MioTrait => l(FullName, "organization trait: icon = {sprite}", "interface/military_industrial_organization/industrial_organization_policies_and_traits_icons.gfx:126 GFX_generic_mio_trait_icon_ap_attack, named in full by trait icon values", None),
        MioPolicy => l(FullName, "organization policy: icon = {sprite}", "common/military_industrial_organization/policies/_general_policies.txt:77 icon = GFX_idea_eng_spirit_of_the_industrial_revolution", None),
        BopSide => l(FullName, "balance of power side: icon = {sprite}", "common/bop/PRC.txt:10 icon = GFX_PRC_bop_left_side", None),
        Medal => l(FullName, "unit medal: icon = GFX_medal_icon_{asset} with frame = 1 (the game also loads GFX_medal_icon_{asset}_large)", "common/unit_medals/00_default.txt:41 icon = \"GFX_medal_icon_nor\" with frame = 1; interface/medals.gfx:247 GFX_medal_icon_eng and :253 GFX_medal_icon_eng_large: each of the 21 GFX_medal_icon_* sprites has a _large twin, and no .gui names one", Some("eng")),
        AgencyUpgrade => l(FullName, "intelligence agency upgrade: picture = {sprite}", "common/intelligence_agency_upgrades/intelligence_agency_upgrades.txt:73 picture = GFX_agency_airforce_department", None),
        OperationIcon => l(FullName, "operation phase: icon = {sprite}", "common/operation_phases/00_operation_phases.txt icon values (73, full names) -> interface/lar_operations.gfx:546 GFX_phase_infiltrate_military_bribe_small (59x58)", Some("infiltrate_military_bribe")),
        OperationPicture => l(FullName, "operation phase: picture = {sprite}", "common/operation_phases/lar_historical_operations.txt:47 picture = GFX_phase_bruneval_attack -> interface/lar_operations.gfx:187 GFX_phase_infiltrate_military_bribe (210x176)", Some("infiltrate_military_bribe")),
        CommanderTrait | PersonalTrait => l(EngineKey, "name the unit leader trait {asset} in common/unit_leader (its icon is GFX_trait_<trait key>)", "common/unit_leader/*.txt: 189 of 196 leader traits have GFX_trait_<key>; traits have no icon field; hoi4.exe has GFX_trait_; interface/aat_traits.gfx:10 GFX_trait_jaeger", Some("jaeger")),
        Doctrine => l(FullName, "doctrine: icon = {sprite}", "common/doctrines/subdoctrines/land/combat_support_subdoctrines.txt:282 icon = GFX_doctrine_air_cavalry_medium", None),
        ReportEvent | NewsEvent => l(FullName, "event: picture = {sprite}", "events/*.txt: 830 of 833 picture values are full GFX_ names (interface/eventpictures.gfx:14 GFX_news_event_polish_army)", None),
        Technology => l(EngineKey, "name the technology {asset} (its picture is GFX_<technology key>_medium)", "interface/Technologies.gfx:183 GFX_infantry_weapons_medium for technology infantry_weapons; 429 of 552 technologies have GFX_<key>_medium; hoi4.exe pairs tech_icon with _medium", Some("infantry_weapons")),
        Achievement => l(FileName, "name the achievement {asset} in common/achievements.txt (the game loads gfx/achievements/<key>.dds, _grey.dds and _not_eligible.dds)", "gfx/achievements: 292 achievement keys have <key>.dds, <key>_grey.dds and (291) <key>_not_eligible.dds; no .gfx registers them", None),
        LeaderPortrait | CommanderPortrait | OperativePortrait | ScientistPortrait => l(FullName, "character portraits: large = {sprite}", "common/characters/*.txt: 2,045 of 2,047 large values are full GFX_ names (interface/_scientists_portraits.gfx:7 GFX_portrait_AST_lawrence_wackett)", None),
        AdvisorCard => l(FullName, "character portraits: small = {sprite} (in the civilian or army block of the advisor)", "common/characters/*.txt: all 2,887 small values are full GFX_ names (interface/ideas.gfx:6562 GFX_portrait_ETH_amha_selassie_small)", None),
        Flag => l(FileName, "no script: the game loads gfx/flags/<TAG>[_ideology].tga and the medium and small copies by file name", "gfx/flags, gfx/flags/medium, gfx/flags/small (82x52, 41x26, 10x7)", None),
        CounterLarge => l(EngineKey, "name the sub-unit {asset} in common/units (the battalion icon is GFX_unit_<sub-unit key>_icon_medium)", "interface/subuniticons.gfx:81 GFX_unit_engineer_icon_medium for sub-unit engineer, whose sprite is infantry (common/units/engineer.txt:4): the key, not sprite, names the icon; 100 sub-units whose sprite differs from the key all have GFX_unit_<key>_icon_medium; hoi4.exe has GFX_unit_ and _icon_medium", Some("engineer")),
        CounterSmall => l(EngineKey, "name the sub-unit {asset} in common/units (the on-map counter is GFX_unit_<sub-unit key>_icon_medium_white, the tooltip text icon GFX_unit_<sub-unit key>_icon_small)", "interface/subuniticons.gfx:201 GFX_unit_infantry_icon_medium_white (noOfFrames = 2); interface/texticons.gfx:953 GFX_unit_infantry_icon_small (noOfFrames = 2); hoi4.exe has _icon_medium_white and builds unit_<key>_icon_small| text icons for sub-unit tooltips", Some("infantry")),
        DivisionEmblem => l(Numbered, "no script: the player picks symbol {n} in the division designer (GFX_div_templ_{n}_large and GFX_div_templ_{n}_small)", "interface/subuniticons.gfx:318 GFX_div_templ_0_large to :418 GFX_div_templ_121_large and :425 GFX_div_templ_0_small; numbers 0-43 and 65-121 (a gap at 44-64); hoi4.exe has GFX_div_templ_ and _large", Some("0")),
        Building => l(FullName, "landmark building: special_icon = GFX_{asset}_icon_small (the raid window shows the large GFX_{asset}_icon)", "common/buildings/01_landmark_buildings.txt:134 special_icon = GFX_eiffel_tower_icon_small -> interface/buildings/landmarks.gfx:144; the large GFX_eiffel_tower_icon is at :7 under '# Big Icons in Raid Window #' and no file names it", Some("eiffel_tower")),
        FactionLogo => l(FullName, "faction: icon = {sprite} (the game also loads {sprite}_miniature)", "common/factions/templates/allies.txt:30 icon = GFX_faction_icon_allies; interface/factions/factions.gfx:20 GFX_faction_logo_axis and :175 GFX_faction_logo_axis_miniature: all 31 logos have a 32x32 _miniature and hoi4.exe has the string _miniature", Some("axis")),
        AgencyLogo => l(FullName, "intelligence agency: picture = {sprite}", "common/intelligence_agencies/00_intelligence_agencies.txt:719 picture = GFX_intelligence_agency_logo_AFG_communism (noOfFrames = 2)", None),
        ModifierStatIcon => l(FullName, "a .gui element: spriteType = \"{sprite}\"", "interface/unitleaderwindow.gui:468 spriteType = \"GFX_MODIFIER_SUPPLY_CONSUMPTION_FACTOR\" -> interface/countryarmyview.gfx:173; hoi4.exe builds no GFX_MODIFIER_ name, and all 159 vanilla ones are upper case", Some("supply_consumption_factor")),
        GuiTexture => l(FullName, "a .gui element: spriteType = \"{sprite}\"", "interface/*.gui name sprites in full", None),
        SuperEvent => l(FullName, "the mod's super event window: spriteType = \"{sprite}\"", "no vanilla super events; a mod's .gui names the sprite in full", None),
        AnimBlendGlow | AnimBlendKeyframes | AnimScroll | AnimSpin => l(FullName, "a .gui iconType: spriteType = \"{sprite}\" (a frameAnimatedSpriteType plays when the element is shown)", "interface/alerts.gfx:16 GFX_yellow_alert_glow (noOfFrames = 2, buttonstate_blendframes.lua) and interface/leadergroups.gfx:46 GFX_defending_icon_anim_strip (noOfFrames = 20) are named in full by .gui elements", None),
    }
}

/// A companion image the game looks up next to the main sprite, derived from the main art at processing time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivedCompanion {
    /// Companion key in the processed record and on the final file.
    pub key: &'static str,
    /// How it is made.
    pub kind: DeriveKind,
    /// Canvas of one frame.
    pub frame: [u32; 2],
    /// Frames in the strip.
    pub frames: u32,
    /// Minimum transparent margin the companion is checked for.
    pub margin: u32,
    /// Folder pattern, or `None` for the main file's folder.
    pub folder: Option<&'static str>,
    /// File name pattern without extension (`{prefix}`, `{asset}`).
    pub file: &'static str,
    /// Sprite name pattern (`{asset}`, `{n}`).
    pub sprite: &'static str,
}

impl DerivedCompanion {
    /// Width and height of the whole strip.
    pub fn strip(&self) -> [u32; 2] {
        [self.frame[0] * self.frames.max(1), self.frame[1]]
    }
}

/// The companions of a family at a frame canvas. A landmark made at the small 32x32 frame is itself the small
/// sprite and has none.
pub fn derived_companions(family: Family, canvas: [u32; 2]) -> Vec<DerivedCompanion> {
    let c = |key, kind, frame, frames, margin, folder, file, sprite| DerivedCompanion { key, kind, frame, frames, margin, folder, file, sprite };
    match family {
        Family::CounterSmall => vec![c(
            "text_icon",
            DeriveKind::TextIcon,
            [30, 12],
            2,
            0,
            Some("gfx/texticons/{feature_slug}"),
            "unit_{asset}_icon_small",
            "GFX_unit_{asset}_icon_small",
        )],
        Family::DivisionEmblem => vec![c(
            "small",
            DeriveKind::SmallEmblem,
            [30, 12],
            1,
            0,
            Some("gfx/interface/counters/division_templates_small/{feature_slug}"),
            "{prefix}{asset}",
            "GFX_div_templ_{n}_small",
        )],
        Family::FactionLogo => {
            vec![c("miniature", DeriveKind::Miniature, [32, 32], 1, 0, None, "{prefix}{asset}_miniature", "GFX_faction_logo_{asset}_miniature")]
        }
        Family::Building if canvas == [64, 64] => {
            vec![c("small", DeriveKind::LandmarkSmall, [32, 32], 2, 0, None, "{prefix}{asset}_small", "GFX_{asset}_icon_small")]
        }
        Family::Medal => vec![c("small", DeriveKind::HalfSize, [28, 42], 1, 1, None, "{prefix}{asset}_small", "GFX_medal_icon_{asset}")],
        _ => Vec::new(),
    }
}

/// The derived companion of a family with this key, if any (at any canvas).
pub fn derived_companion(family: Family, key: &str) -> Option<DerivedCompanion> {
    [[64, 64], [0, 0]].iter().flat_map(|c| derived_companions(family, *c)).find(|d| d.key == key)
}

/// Highest division template symbol number vanilla 1.19.4.1 registers (`interface/subuniticons.gfx:418`).
pub const VANILLA_DIV_TEMPLATE_MAX: u32 = 121;

/// Every division template symbol number registered in a `.gfx` text (`GFX_div_templ_<n>_large` or `_small`,
/// without case). Comments are ignored.
pub fn div_template_numbers(gfx_text: &str) -> Vec<u32> {
    const KEY: &str = "gfx_div_templ_";
    let mut out = Vec::new();
    for line in gfx_text.lines() {
        let code = line.split('#').next().unwrap_or("").to_ascii_lowercase();
        let mut rest = code.as_str();
        while let Some(at) = rest.find(KEY) {
            let after = &rest[at + KEY.len()..];
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            let tail = &after[digits.len()..];
            if !digits.is_empty() && (tail.starts_with("_large") || tail.starts_with("_small")) {
                if let Ok(n) = digits.parse() {
                    out.push(n);
                }
            }
            rest = after;
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The number a new division template symbol takes: one past the highest of vanilla's numbers and every number
/// already taken (by the mod's `.gfx` files or by earlier installs). Vanilla's gap at 44-64 is never reused, so a
/// mod symbol never shadows a number the game may still reserve.
pub fn next_div_template_number(vanilla_max: u32, taken: impl IntoIterator<Item = u32>) -> u32 {
    taken.into_iter().fold(vanilla_max, u32::max) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_has_a_lookup_with_evidence_and_script_text() {
        for f in Family::ALL.iter().copied() {
            let l = lookup_of(f);
            assert!(l.evidence.len() > 20, "{f}");
            assert!(!l.script.is_empty(), "{f}");
            if matches!(l.kind, LookupKind::ShortKey | LookupKind::EngineKey | LookupKind::Numbered) {
                assert!(l.vanilla_example.is_some(), "{f}: a built name needs a vanilla example for the game test");
            }
        }
    }

    #[test]
    fn companions_have_whole_strips_and_known_keys() {
        assert_eq!(derived_companions(Family::CounterSmall, [30, 12])[0].strip(), [60, 12]);
        assert_eq!(derived_companions(Family::DivisionEmblem, [76, 42])[0].strip(), [30, 12]);
        assert_eq!(derived_companions(Family::FactionLogo, [100, 100])[0].strip(), [32, 32]);
        assert_eq!(derived_companions(Family::Building, [64, 64])[0].strip(), [64, 32]);
        assert!(derived_companions(Family::Building, [32, 32]).is_empty(), "the small landmark is itself the small sprite");
        assert_eq!(derived_companions(Family::Medal, [56, 84])[0].strip(), [28, 42]);
        assert!(derived_companions(Family::Focus, [100, 88]).is_empty());
        assert_eq!(derived_companion(Family::Building, "small").map(|d| d.frames), Some(2));
        assert!(derived_companion(Family::Focus, "small").is_none());
    }

    #[test]
    fn division_template_numbers_are_read_and_the_next_one_follows_the_highest() {
        let text = "spriteTypes = {\n\tspriteType = { name = \"GFX_div_templ_0_large\" }\n\tspriteType = { name = \"gfx_DIV_TEMPL_130_small\" }\n\t# spriteType = { name = \"GFX_div_templ_999_large\" }\n\tspriteType = { name = \"GFX_div_templ_7_medium\" }\n\tspriteType = { name = \"GFX_div_templ_x_large\" }\n}\n";
        assert_eq!(div_template_numbers(text), vec![0, 130]);
        assert_eq!(next_div_template_number(VANILLA_DIV_TEMPLATE_MAX, []), 122);
        assert_eq!(next_div_template_number(VANILLA_DIV_TEMPLATE_MAX, [5, 44, 130]), 131);
        assert_eq!(next_div_template_number(VANILLA_DIV_TEMPLATE_MAX, [121]), 122);
    }
}
