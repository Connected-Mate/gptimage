//! Final paths, sprite names and file bytes for an installed asset.

use crate::error::{Error, Result};
use crate::family::Family;
use crate::lookup::{derived_companions, lookup_of};
use crate::naming::{expand, validate_flag_name, validate_snake};
use crate::pixels::Rgba;
use crate::profile::{Container, Profile};
use crate::resize::resize_lanczos3;
use crate::sprite::{animated_sprite, framed_sprite, framed_sprite_eager, plain_sprite, shine_sprite, SpriteEntry};
use crate::{dds, tga};

/// Name ending of a balance of power side and of every stage of it.
pub const BOP_SIDE_SUFFIX: &str = "_side";
/// Keys of the optional weakened stages of a balance of power side: vanilla's `<name>_bad_side` and `<name>_bad_improved_side`.
pub const BOP_BAD_KEYS: [&str; 2] = ["_bad", "_bad_improved"];

/// True for a file an install writes only when the processed record has its image: the weakened stages.
pub fn is_optional_companion(family: Family, file: &FinalFile) -> bool {
    family == Family::BopSide && file.variant.as_deref().is_some_and(|k| BOP_BAD_KEYS.contains(&k))
}

/// One file an install writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalFile {
    /// Project-relative path.
    pub rel: String,
    /// Variant name for multi-file assets (`normal`, `medium`, `small` for flags).
    pub variant: Option<String>,
    /// Canvas of this file (all frames).
    pub canvas: [u32; 2],
    /// Container.
    pub container: Container,
}

/// Check the asset name against the family's rule.
pub fn validate_asset_name(category: Family, asset: &str) -> Result<()> {
    if category == Family::Flag {
        validate_flag_name(asset)
    } else {
        validate_snake("asset", asset)
    }
}

/// The file name prefix with placeholders resolved.
pub fn resolved_prefix(profile: &Profile, feature_slug: &str, asset: &str) -> Result<String> {
    expand(&profile.imagegen.prefix, feature_slug, asset, "")
}

/// Final files for an asset.
pub fn final_files(profile: &Profile, canvas: [u32; 2], feature_slug: &str, asset: &str) -> Result<Vec<FinalFile>> {
    validate_snake("feature slug", feature_slug)?;
    validate_asset_name(profile.family, asset)?;
    let prefix = resolved_prefix(profile, feature_slug, asset)?;
    let ext = profile.imagegen.container.ext();
    let strip = [canvas[0] * profile.frames.max(1), canvas[1]];
    if !profile.imagegen.variants.is_empty() {
        return profile
            .imagegen
            .variants
            .iter()
            .map(|v| {
                let folder = expand(&v.folder, feature_slug, asset, &prefix)?;
                Ok(FinalFile {
                    rel: format!("{}/{prefix}{asset}.{ext}", folder.trim_end_matches('/')),
                    variant: Some(v.name.clone()),
                    canvas: [v.canvas[0] * profile.frames.max(1), v.canvas[1]],
                    container: profile.imagegen.container,
                })
            })
            .collect();
    }
    let folder = expand(&profile.imagegen.folder, feature_slug, asset, &prefix)?;
    let folder = folder.trim_end_matches('/');
    // a balance of power side is named `<name>_side`, its improved stage `<name>_improved_side` (vanilla's names)
    let bop = profile.family == Family::BopSide;
    let main_suffix = if bop { BOP_SIDE_SUFFIX } else { "" };
    let mut files = vec![FinalFile {
        rel: format!("{folder}/{prefix}{asset}{main_suffix}.{ext}"),
        variant: None,
        canvas: strip,
        container: profile.imagegen.container,
    }];
    // companions the recipe makes next to the main file (the achievement states): same folder, suffixed name
    // (a side's optional weakened stages are listed too: install writes it only when the processed record has it)
    let mut keys: Vec<&str> = crate::chain::recipe_of(profile).companion_keys().iter().copied().filter(|k| k.starts_with('_')).collect();
    if bop {
        keys.extend(BOP_BAD_KEYS);
    }
    for key in keys {
        files.push(FinalFile {
            rel: format!("{folder}/{prefix}{asset}{key}{main_suffix}.{ext}"),
            variant: Some(key.to_string()),
            canvas: strip,
            container: profile.imagegen.container,
        });
    }
    // companions the game looks up next to the main sprite (small emblem, text icon, miniature, half size)
    for d in derived_companions(profile.family, canvas) {
        let dir = match d.folder {
            Some(pattern) => expand(pattern, feature_slug, asset, &prefix)?,
            None => folder.to_string(),
        };
        let name = expand(d.file, feature_slug, asset, &prefix)?;
        files.push(FinalFile {
            rel: format!("{}/{name}.{ext}", dir.trim_end_matches('/')),
            variant: Some(d.key.to_string()),
            canvas: d.strip(),
            container: profile.imagegen.container,
        });
    }
    Ok(files)
}

/// Replace `{n}` (the division template symbol number) in a pattern.
fn with_number(pattern: &str, number: Option<u32>) -> Result<String> {
    if !pattern.contains("{n}") {
        return Ok(pattern.to_string());
    }
    let n = number.ok_or_else(|| Error::Invalid(format!("the sprite name '{pattern}' needs a division template number, and none was chosen")))?;
    Ok(pattern.replace("{n}", &n.to_string()))
}

/// The main sprite name. A landmark made at the small 32x32 frame is the province window's `_small` sprite.
pub fn main_sprite_name(profile: &Profile, canvas: [u32; 2], feature_slug: &str, asset: &str, number: Option<u32>) -> Result<Option<String>> {
    let Some(spec) = &profile.imagegen.sprite else { return Ok(None) };
    let prefix = resolved_prefix(profile, feature_slug, asset)?;
    let mut name = expand(&with_number(&spec.name, number)?, feature_slug, asset, &prefix)?;
    if profile.family == Family::Building && canvas == [32, 32] && !name.ends_with("_small") {
        name.push_str("_small");
    }
    Ok(Some(name))
}

/// What the mod's script writes so the game finds the installed art, with the names filled in.
pub fn script_usage(profile: &Profile, canvas: [u32; 2], feature_slug: &str, asset: &str, number: Option<u32>) -> Result<String> {
    let sprite = main_sprite_name(profile, canvas, feature_slug, asset, number)?.unwrap_or_default();
    let mut text = lookup_of(profile.family).script.replace("{sprite}", &sprite).replace("{asset}", asset);
    if let Some(n) = number {
        text = text.replace("{n}", &n.to_string());
    }
    if profile.family == Family::Technology {
        text.push_str(". ");
        text.push_str(&crate::lookup::tech_box_note(canvas));
    }
    Ok(text)
}

/// Sprite entries and the suggested `.gfx` file (project-relative, under `interface/`).
/// Returns `None` for families the engine loads by file name (flags, achievements).
///
/// `number` is the division template symbol number of a division emblem (`GFX_div_templ_<n>_large`); other
/// families pass `None`.
pub fn sprite_entries(
    profile: &Profile,
    feature_slug: &str,
    asset: &str,
    files: &[FinalFile],
    number: Option<u32>,
) -> Result<Option<(String, Vec<SpriteEntry>)>> {
    let Some(spec) = &profile.imagegen.sprite else { return Ok(None) };
    let main = files.first().ok_or_else(|| Error::Invalid("no final files".into()))?;
    let frames = profile.frames.max(1);
    let canvas = [main.canvas[0] / frames, main.canvas[1]];
    let prefix = resolved_prefix(profile, feature_slug, asset)?;
    let name = main_sprite_name(profile, canvas, feature_slug, asset, number)?.unwrap_or_default();
    let gfx_file = expand(&spec.gfx_file, feature_slug, asset, &prefix)?;
    let mut entries = vec![match &profile.imagegen.animation {
        Some(anim) => animated_sprite(&name, &main.rel, frames, anim)?,
        None if profile.frames > 1 => framed_sprite(&name, &main.rel, profile.frames),
        None => plain_sprite(&name, &main.rel),
    }];
    for companion in &spec.companions {
        match companion.as_str() {
            "shine" => entries.push(shine_sprite(&name, &main.rel)?),
            other => return Err(Error::Config(format!("unknown sprite companion '{other}'"))),
        }
    }
    if profile.family == Family::BopSide {
        // `GFX_bop_<name>_side` has the stages `GFX_bop_<name>_improved_side`, `GFX_bop_<name>_bad_side` and `GFX_bop_<name>_bad_improved_side`
        for f in files.iter().filter(|f| f.variant.is_some()) {
            let key = f.variant.as_deref().unwrap_or_default();
            entries.push(plain_sprite(&format!("GFX_{prefix}{asset}{key}{BOP_SIDE_SUFFIX}"), &f.rel));
        }
    }
    for d in derived_companions(profile.family, canvas) {
        let file = files
            .iter()
            .find(|f| f.variant.as_deref() == Some(d.key))
            .ok_or_else(|| Error::Invalid(format!("the {} companion file of {asset} is missing", d.key)))?;
        let cname = expand(&with_number(d.sprite, number)?, feature_slug, asset, &prefix)?;
        entries.push(if d.legacy_lazy_load_no {
            framed_sprite_eager(&cname, &file.rel, d.frames)
        } else if d.frames > 1 {
            framed_sprite(&cname, &file.rel, d.frames)
        } else {
            plain_sprite(&cname, &file.rel)
        });
    }
    Ok(Some((gfx_file, entries)))
}

/// Encode the processed art for one final file.
///
/// `base` is the full-strip size the processed art must have (canvas x frames). Only size variants of a
/// multi-file asset (the medium and small flag) are resampled from it; any other mismatch is an error, so a
/// wrong-sized image is never silently stretched into the game.
pub fn export_bytes(processed: &Rgba, file: &FinalFile, base: [u32; 2]) -> Result<Vec<u8>> {
    if processed.width != base[0] || processed.height != base[1] {
        return Err(Error::Invalid(format!(
            "processed art is {}x{} but {} is made from {}x{} art",
            processed.width, processed.height, file.rel, base[0], base[1]
        )));
    }
    let img = if processed.width == file.canvas[0] && processed.height == file.canvas[1] {
        processed.clone()
    } else if file.variant.is_some() {
        resize_lanczos3(processed, file.canvas[0], file.canvas[1])?
    } else {
        return Err(Error::Invalid(format!("{} needs {}x{} art", file.rel, file.canvas[0], file.canvas[1])));
    };
    Ok(match file.container {
        Container::Dds => dds::write_a8r8g8b8(&img),
        Container::Tga => tga::write_truecolor32(&img, tga::Origin::BottomLeft),
    })
}

/// Decode what [`export_bytes`] wrote, to prove the round trip.
pub fn decode_exported(bytes: &[u8], container: Container) -> Result<Rgba> {
    match container {
        Container::Dds => dds::decode(bytes),
        Container::Tga => tga::decode(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileSet;

    fn p(c: Family) -> Profile {
        ProfileSet::builtin().for_family(c).unwrap().clone()
    }

    #[test]
    fn focus_icon_paths_and_sprites_follow_kit_conventions() {
        let prof = p(Family::Focus);
        let files = final_files(&prof, prof.canvas, "iron_pact", "steel_will").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel, "gfx/interface/goals/iron_pact/focus_steel_will.dds");
        assert_eq!(files[0].canvas, [100, 88]);
        let (gfx, entries) = sprite_entries(&prof, "iron_pact", "steel_will", &files, None).unwrap().unwrap();
        assert_eq!(gfx, "interface/iron_pact_goals.gfx");
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["GFX_focus_steel_will", "GFX_focus_steel_will_shine"]);
        assert!(entries[0].text.contains("gfx/interface/goals/iron_pact/focus_steel_will.dds"));
        assert!(entries[1].text.contains("animationrotationoffset = { x = 0.0 y = 0.0 }"), "the vanilla-exact shine block");
        assert_eq!(entries[1].text.matches("animation = {").count(), 2);
    }

    #[test]
    fn flags_produce_three_tgas_with_tag_names() {
        let prof = p(Family::Flag);
        let files = final_files(&prof, prof.canvas, "iron_pact", "IRP_fascism").unwrap();
        let rels: Vec<_> = files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, vec!["gfx/flags/IRP_fascism.tga", "gfx/flags/medium/IRP_fascism.tga", "gfx/flags/small/IRP_fascism.tga"]);
        assert_eq!(files[2].canvas, [10, 7]);
        assert!(final_files(&prof, prof.canvas, "iron_pact", "irp").is_err(), "flag names must be tags");
        assert!(sprite_entries(&prof, "iron_pact", "IRP", &files, None).unwrap().is_none());
        let img = Rgba::filled(82, 52, [200, 20, 20, 255]);
        let small = export_bytes(&img, &files[2], [82, 52]).unwrap();
        let info = tga::parse_header(&small).unwrap();
        assert_eq!((info.width, info.height, info.top_origin), (10, 7, false));
        let wrong = Rgba::filled(80, 52, [0, 0, 0, 255]);
        assert!(export_bytes(&wrong, &files[0], [82, 52]).is_err());
    }

    #[test]
    fn achievements_are_named_by_key_in_the_root_folder_and_have_no_sprite() {
        let prof = p(Family::Achievement);
        let files = final_files(&prof, prof.canvas, "iron_pact", "hold_the_line").unwrap();
        assert_eq!(files[0].rel, "gfx/achievements/hold_the_line.dds");
        let rels: Vec<_> = files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(
            rels,
            vec!["gfx/achievements/hold_the_line.dds", "gfx/achievements/hold_the_line_grey.dds", "gfx/achievements/hold_the_line_not_eligible.dds"]
        );
        assert_eq!(files[1].variant.as_deref(), Some("_grey"));
        assert!(sprite_entries(&prof, "iron_pact", "hold_the_line", &files, None).unwrap().is_none());
    }

    #[test]
    fn sprite_names_follow_the_family_patterns() {
        let name_of = |f: Family, asset: &str| {
            let prof = p(f);
            let canvas = if prof.needs_mod_canvas() { [64, 64] } else { prof.canvas };
            let files = final_files(&prof, canvas, "iron_pact", asset).unwrap();
            sprite_entries(&prof, "iron_pact", asset, &files, None).unwrap().unwrap().1[0].name.clone()
        };
        assert_eq!(name_of(Family::IdeaSpirit, "iron_will"), "GFX_idea_iron_will");
        assert_eq!(name_of(Family::AdvisorCard, "gen_smith"), "GFX_idea_gen_smith");
        assert_eq!(name_of(Family::OfficerCorpsSpirit, "grit"), "GFX_idea_grit");
        assert_eq!(name_of(Family::Decision, "rally"), "GFX_decision_rally");
        assert_eq!(name_of(Family::StateModifier, "bunkers"), "GFX_modifiers_bunkers");
        assert_eq!(name_of(Family::Technology, "rifle_tech"), "GFX_rifle_tech_medium");
        assert_eq!(name_of(Family::AgencyLogo, "north"), "GFX_intelligence_agency_logo_north");
        assert_eq!(name_of(Family::CounterSmall, "inf"), "GFX_unit_inf_icon_medium_white");
    }

    #[test]
    fn frame_strips_are_registered_with_their_frame_count_and_strip_width() {
        let prof = p(Family::CounterLarge);
        let files = final_files(&prof, prof.canvas, "iron_pact", "inf").unwrap();
        assert_eq!(files[0].canvas, [152, 42]);
        let (_, entries) = sprite_entries(&prof, "iron_pact", "inf", &files, None).unwrap().unwrap();
        assert!(entries[0].text.contains("noOfFrames = 2"));
        let prof = p(Family::Focus);
        let files = final_files(&prof, prof.canvas, "iron_pact", "x").unwrap();
        assert!(!sprite_entries(&prof, "iron_pact", "x", &files, None).unwrap().unwrap().1[0].text.contains("noOfFrames"));
    }

    #[test]
    fn animated_families_register_a_frame_animated_sprite_type() {
        let prof = p(Family::AnimBlendGlow);
        let files = final_files(&prof, [120, 60], "iron_pact", "alert").unwrap();
        assert_eq!(files[0].canvas, [240, 60], "a plan's own canvas is one frame");
        let (gfx, entries) = sprite_entries(&prof, "iron_pact", "alert", &files, None).unwrap().unwrap();
        assert_eq!(gfx, "interface/iron_pact_animated.gfx");
        let t = &entries[0].text;
        assert!(t.starts_with("\tframeAnimatedSpriteType = {"), "{t}");
        for line in [
            "noOfFrames = 2",
            "animation_rate_fps = 1",
            "looping = yes",
            "play_on_show = yes",
            "alwaystransparent = yes",
            "effectFile = \"gfx/FX/buttonstate_blendframes.lua\"",
        ] {
            assert!(t.contains(line), "{line} missing in {t}");
        }
        let prof = p(Family::AnimSpin);
        let files = final_files(&prof, prof.canvas, "iron_pact", "cog").unwrap();
        let t = sprite_entries(&prof, "iron_pact", "cog", &files, None).unwrap().unwrap().1[0].text.clone();
        assert!(
            t.contains("noOfFrames = 20") && t.contains("animation_rate_fps = 7") && t.contains("loadType = \"INGAME\"") && !t.contains("effectFile"),
            "{t}"
        );
        // the block parses and registers like any sprite
        let bytes = crate::sprite::register_bytes(None, &[entries[0].clone()]).unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains("GFX_glow_alert"));
    }

    /// One row per family: asset name, the division template number, then every sprite the install registers as
    /// (name, texture, noOfFrames), main first. The patterns are the vanilla ones (see `lookup.rs` for the
    /// evidence); a family loaded by file name lists its files with an empty sprite name.
    #[allow(clippy::type_complexity)]
    fn expected(f: Family) -> (&'static str, Option<u32>, Vec<(&'static str, &'static str, u32)>) {
        use Family::*;
        let one = |name, tex| vec![(name, tex, 1)];
        match f {
            Focus => (
                "steel_will",
                None,
                vec![
                    ("GFX_focus_steel_will", "gfx/interface/goals/iron_pact/focus_steel_will.dds", 1),
                    ("GFX_focus_steel_will_shine", "gfx/interface/goals/iron_pact/focus_steel_will.dds", 1),
                ],
            ),
            IdeaSpirit => ("iron_will", None, one("GFX_idea_iron_will", "gfx/interface/ideas/iron_pact/idea_iron_will.dds")),
            IdeaConcern => ("steelworks", None, one("GFX_idea_steelworks", "gfx/interface/ideas/iron_pact/concerns/idea_steelworks.dds")),
            OfficerCorpsSpirit => {
                ("grit_spirit", None, one("GFX_idea_grit_spirit", "gfx/interface/officer_corp/spirits/iron_pact/spirit_grit_spirit.dds"))
            }
            Decision => ("rally", None, one("GFX_decision_rally", "gfx/interface/decisions/iron_pact/decision_rally.dds")),
            DecisionCategoryIcon => (
                "council",
                None,
                one("GFX_decision_category_council", "gfx/interface/decisions/iron_pact/category_icons/decision_category_council.dds"),
            ),
            DecisionCategoryPicture => (
                "council",
                None,
                one("GFX_decision_cat_picture_council", "gfx/interface/decisions/iron_pact/category_pictures/decision_cat_picture_council.dds"),
            ),
            StateModifier => ("bunkers", None, one("GFX_modifiers_bunkers", "gfx/interface/state_modifiers/iron_pact/modifiers_bunkers.dds")),
            SpecialProjectIcon => ("rail_gun", None, one("GFX_sp_rail_gun", "gfx/interface/special_project/project_icons/iron_pact/sp_rail_gun.dds")),
            Raid => {
                ("bridge_raid", None, one("GFX_raid_type_icon_bridge_raid", "gfx/interface/military_raids/iron_pact/raid_type_icon_bridge_raid.dds"))
            }
            MioDepartment => (
                "arms",
                None,
                one("GFX_mio_department_arms", "gfx/interface/military_industrial_organization/department_icons/iron_pact/mio_department_arms.dds"),
            ),
            MioTrait => (
                "rugged",
                None,
                one("GFX_mio_trait_rugged", "gfx/interface/military_industrial_organization/trait_icons/iron_pact/mio_trait_rugged.dds"),
            ),
            MioPolicy => (
                "quotas",
                None,
                one("GFX_mio_policy_quotas", "gfx/interface/military_industrial_organization/policy_icons/iron_pact/mio_policy_quotas.dds"),
            ),
            BopSide => (
                "hawks",
                None,
                vec![
                    ("GFX_bop_hawks_side", "gfx/interface/bop/iron_pact/bop_hawks_side.dds", 1),
                    ("GFX_bop_hawks_improved_side", "gfx/interface/bop/iron_pact/bop_hawks_improved_side.dds", 1),
                    ("GFX_bop_hawks_bad_side", "gfx/interface/bop/iron_pact/bop_hawks_bad_side.dds", 1),
                    ("GFX_bop_hawks_bad_improved_side", "gfx/interface/bop/iron_pact/bop_hawks_bad_improved_side.dds", 1),
                ],
            ),
            Medal => (
                "valour",
                None,
                vec![
                    ("GFX_medal_icon_valour_large", "gfx/interface/medals/iron_pact/medal_icon_valour.dds", 1),
                    ("GFX_medal_icon_valour", "gfx/interface/medals/iron_pact/medal_icon_valour_small.dds", 1),
                ],
            ),
            AgencyUpgrade => ("codebook", None, one("GFX_agency_codebook", "gfx/interface/operatives/icons/iron_pact/agency_codebook.dds")),
            OperationIcon => (
                "night_drop",
                None,
                vec![
                    ("GFX_operations_night_drop", "gfx/interface/operations/iron_pact/night_drop.dds", 1),
                    ("GFX_operations_night_drop_map", "gfx/interface/operations/iron_pact/map/night_drop_map.dds", 1),
                ],
            ),
            OperationPicture => ("night_drop", None, one("GFX_phase_night_drop", "gfx/interface/operations/iron_pact/pictures/phase_night_drop.dds")),
            CommanderTrait => ("stubborn", None, one("GFX_trait_stubborn", "gfx/interface/traits/iron_pact/trait_stubborn.dds")),
            PersonalTrait => ("gambler", None, one("GFX_trait_gambler", "gfx/interface/traits/iron_pact/personal/trait_gambler.dds")),
            Doctrine => ("deep_battle", None, one("GFX_doctrine_deep_battle", "gfx/interface/doctrines/iron_pact/doctrine_deep_battle.dds")),
            ReportEvent => ("border_clash", None, one("GFX_report_event_border_clash", "gfx/event_pictures/iron_pact/report_event_border_clash.dds")),
            NewsEvent => ("border_clash", None, one("GFX_news_event_border_clash", "gfx/event_pictures/iron_pact/news_event_border_clash.dds")),
            Technology => ("rifle_tech", None, one("GFX_rifle_tech_medium", "gfx/interface/technologies/iron_pact/rifle_tech.dds")),
            Achievement => (
                "hold_the_line",
                None,
                vec![
                    ("", "gfx/achievements/hold_the_line.dds", 1),
                    ("", "gfx/achievements/hold_the_line_grey.dds", 1),
                    ("", "gfx/achievements/hold_the_line_not_eligible.dds", 1),
                ],
            ),
            LeaderPortrait => ("marshal", None, one("GFX_portrait_marshal", "gfx/leaders/iron_pact/portrait_marshal.dds")),
            CommanderPortrait => ("marshal", None, one("GFX_portrait_marshal", "gfx/leaders/iron_pact/commanders/portrait_marshal.dds")),
            OperativePortrait => ("agent", None, one("GFX_portrait_agent", "gfx/leaders/iron_pact/operatives/portrait_agent.dds")),
            ScientistPortrait => ("doctor", None, one("GFX_portrait_doctor", "gfx/leaders/iron_pact/scientists/portrait_doctor.dds")),
            AdvisorCard => ("minister", None, one("GFX_idea_minister", "gfx/interface/ideas/iron_pact/advisors/idea_minister.dds")),
            Flag => ("IRP", None, vec![("", "gfx/flags/IRP.tga", 1), ("", "gfx/flags/medium/IRP.tga", 1), ("", "gfx/flags/small/IRP.tga", 1)]),
            CounterLarge => (
                "mountain_rangers",
                None,
                vec![("GFX_unit_mountain_rangers_icon_medium", "gfx/interface/counters/divisions_large/iron_pact/unit_mountain_rangers.dds", 2)],
            ),
            CounterSmall => (
                "mountain_rangers",
                None,
                vec![
                    (
                        "GFX_unit_mountain_rangers_icon_medium_white",
                        "gfx/interface/counters/divisions_small/iron_pact/onmap_unit_mountain_rangers.dds",
                        2,
                    ),
                    ("GFX_unit_mountain_rangers_icon_small", "gfx/texticons/iron_pact/unit_mountain_rangers_icon_small.dds", 2),
                ],
            ),
            DivisionEmblem => (
                "wolf",
                Some(122),
                vec![
                    ("GFX_div_templ_122_large", "gfx/interface/counters/division_templates_large/iron_pact/custom_wolf.dds", 1),
                    ("GFX_div_templ_122_small", "gfx/interface/counters/division_templates_small/iron_pact/custom_wolf.dds", 1),
                ],
            ),
            Building => (
                "iron_gate",
                None,
                vec![
                    ("GFX_iron_gate_icon", "gfx/interface/buildings/iron_pact/building_iron_gate.dds", 2),
                    ("GFX_iron_gate_icon_small", "gfx/interface/buildings/iron_pact/building_iron_gate_small.dds", 2),
                ],
            ),
            FactionLogo => (
                "iron_pact",
                None,
                vec![
                    ("GFX_faction_logo_iron_pact", "gfx/interface/factions/faction_logos/iron_pact/faction_logo_iron_pact.dds", 2),
                    (
                        "GFX_faction_logo_iron_pact_miniature",
                        "gfx/interface/factions/faction_logos/iron_pact/faction_logo_iron_pact_miniature.dds",
                        1,
                    ),
                ],
            ),
            AgencyLogo => {
                ("north", None, vec![("GFX_intelligence_agency_logo_north", "gfx/interface/operatives/agencies/iron_pact/agency_logo_north.dds", 2)])
            }
            ModifierStatIcon => ("army_speed", None, one("GFX_MODIFIER_ARMY_SPEED", "gfx/interface/modifiers/iron_pact/modifier_army_speed.dds")),
            GuiTexture => ("panel", None, one("GFX_gui_panel", "gfx/interface/iron_pact/gui_panel.dds")),
            SuperEvent => ("victory", None, one("GFX_super_event_victory", "gfx/super_events/iron_pact/super_event_victory.dds")),
            // not game art: no sprite, and install refuses it (the file list is what a final file would be named)
            ModelReference => ("knight", None, one("", "docs/assets/iron_pact/model_reference/model_reference_knight.dds")),
            AnimBlendGlow => ("alert", None, vec![("GFX_glow_alert", "gfx/interface/animated/iron_pact/glows/glow_alert.dds", 2)]),
            AnimBlendKeyframes => ("beacon", None, vec![("GFX_anim_beacon", "gfx/interface/animated/iron_pact/keyframes/anim_beacon.dds", 3)]),
            AnimScroll => ("research", None, vec![("GFX_scroll_research", "gfx/interface/animated/iron_pact/scroll/scroll_research.dds", 9)]),
            AnimSpin => ("cog", None, vec![("GFX_spin_cog", "gfx/interface/animated/iron_pact/spin/spin_cog.dds", 20)]),
        }
    }

    #[test]
    fn every_family_registers_the_vanilla_name_frames_and_path() {
        for f in Family::ALL.iter().copied() {
            let prof = p(f);
            let canvas = if prof.needs_mod_canvas() { [64, 64] } else { prof.canvas };
            let (asset, number, want) = expected(f);
            let files = final_files(&prof, canvas, "iron_pact", asset).unwrap();
            match sprite_entries(&prof, "iron_pact", asset, &files, number).unwrap() {
                Some((gfx, entries)) => {
                    assert!(gfx.starts_with("interface/") && gfx.ends_with(".gfx"), "{f}: {gfx}");
                    let got: Vec<(String, String, u32)> = entries
                        .iter()
                        .map(|e| {
                            let tex = e.text.split("texturefile = \"").nth(1).and_then(|t| t.split('"').next()).unwrap_or("").to_string();
                            let frames =
                                e.text.split("noOfFrames = ").nth(1).and_then(|t| t.split_whitespace().next()).map_or(1, |n| n.parse().unwrap());
                            (e.name.clone(), tex, frames)
                        })
                        .collect();
                    let want: Vec<(String, String, u32)> = want.iter().map(|(n, t, fr)| (n.to_string(), t.to_string(), *fr)).collect();
                    assert_eq!(got, want, "{f}");
                    // every file the install writes is registered by a sprite, and every sprite's texture is written
                    let mut rels: Vec<&str> = files.iter().map(|x| x.rel.as_str()).collect();
                    rels.sort_unstable();
                    rels.dedup();
                    let mut texs: Vec<&str> = want.iter().map(|w| w.1.as_str()).collect();
                    texs.sort_unstable();
                    texs.dedup();
                    assert_eq!(rels, texs, "{f}");
                    // the strip of every file matches the frames its sprite declares
                    for (file, w) in files.iter().zip(want.iter().filter(|w| !w.0.ends_with("_shine"))) {
                        assert_eq!(file.canvas[0] % w.2, 0, "{f}: {} does not divide into {} frames", file.rel, w.2);
                    }
                    // the script text names the main sprite or the short key, fully expanded
                    let script = script_usage(&prof, canvas, "iron_pact", asset, number).unwrap();
                    assert!(script.contains(&want[0].0) || script.contains(asset), "{f}: {script}");
                    assert!(!script.contains('{'), "{f}: unexpanded placeholder in {script}");
                }
                None => {
                    let rels: Vec<&str> = files.iter().map(|x| x.rel.as_str()).collect();
                    let want: Vec<&str> = want.iter().map(|w| w.1).collect();
                    assert_eq!(rels, want, "{f}");
                }
            }
        }
    }

    #[test]
    fn legacy_lazy_load_is_written_where_vanilla_uses_it_and_nowhere_else() {
        for f in Family::ALL.iter().copied() {
            let prof = p(f);
            let canvas = if prof.needs_mod_canvas() { [64, 64] } else { prof.canvas };
            let (asset, number, _) = expected(f);
            let files = final_files(&prof, canvas, "iron_pact", asset).unwrap();
            let Some((_, entries)) = sprite_entries(&prof, "iron_pact", asset, &files, number).unwrap() else { continue };
            for e in &entries {
                // vanilla: 201 of 201 text icons and 2,930 of 2,930 focus shines have the line, no other kind reaches 2%
                let want = e.name.ends_with("_icon_small") && e.name.starts_with("GFX_unit_") || e.name.ends_with("_shine");
                assert_eq!(
                    e.text.contains("legacy_lazy_load = no"),
                    want,
                    "{f}: {}
{}",
                    e.name,
                    e.text
                );
            }
        }
        let prof = p(Family::CounterSmall);
        let files = final_files(&prof, prof.canvas, "iron_pact", "mountain_rangers").unwrap();
        let (_, entries) = sprite_entries(&prof, "iron_pact", "mountain_rangers", &files, None).unwrap().unwrap();
        let text_icon = entries.iter().find(|e| e.name == "GFX_unit_mountain_rangers_icon_small").unwrap();
        assert!(text_icon.text.contains("legacy_lazy_load = no") && text_icon.text.contains("noOfFrames = 2"), "{}", text_icon.text);
    }

    #[test]
    fn the_short_key_families_hand_the_agent_the_short_key() {
        let usage = |f: Family, asset: &str| {
            let prof = p(f);
            script_usage(&prof, prof.canvas, "iron_pact", asset, None).unwrap()
        };
        assert_eq!(usage(Family::IdeaSpirit, "iron_will"), "idea: picture = iron_will");
        assert_eq!(usage(Family::Decision, "rally"), "decision: icon = rally");
        assert_eq!(usage(Family::DecisionCategoryIcon, "council"), "decision category: icon = council");
        assert_eq!(usage(Family::DecisionCategoryPicture, "council"), "decision category: picture = council");
        assert!(usage(Family::CounterLarge, "mountain_rangers").contains("name the sub-unit mountain_rangers"));
        assert!(usage(Family::Technology, "rifle_tech").contains("name the technology rifle_tech"));
        let tech = usage(Family::Technology, "rifle_tech");
        assert!(tech.contains("countrytechtreeview.gui") && tech.contains("tech_support_tree"), "{tech}");
        assert!(tech.contains("146x54") && tech.contains("183x84") && tech.contains("The game picks the item from the technology"), "{tech}");
        let small = script_usage(&p(Family::Technology), [64, 64], "iron_pact", "rifle_tech", None).unwrap();
        assert!(small.contains("64x64") && small.contains("72x72") && small.contains("neither equipment nor sub-units"), "{small}");
        assert!(tech.contains("Found no grid box for tech") && tech.contains("path from an existing vanilla technology"), "{tech}");
        let landmark = usage(Family::Building, "iron_gate");
        assert!(
            landmark.contains("gfx/entities") && landmark.contains("building_<building key>") && landmark.contains("building_landmark_eiffel_tower"),
            "{landmark}"
        );
        assert!(landmark.contains("doesn't have an entity") && landmark.contains("does not write entities"), "{landmark}");
        assert!(usage(Family::Focus, "steel_will").contains("icon = GFX_focus_steel_will"));
        assert!(usage(Family::Medal, "valour").contains("icon = GFX_medal_icon_valour with frame = 1"));
        let prof = p(Family::DivisionEmblem);
        assert!(script_usage(&prof, prof.canvas, "iron_pact", "wolf", Some(130)).unwrap().contains("symbol 130"));
    }

    #[test]
    fn a_small_landmark_is_the_small_sprite_alone_and_an_emblem_needs_its_number() {
        let prof = p(Family::Building);
        let files = final_files(&prof, [32, 32], "iron_pact", "iron_gate").unwrap();
        assert_eq!(files.len(), 1);
        let (_, entries) = sprite_entries(&prof, "iron_pact", "iron_gate", &files, None).unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "GFX_iron_gate_icon_small");
        assert!(entries[0].text.contains("noOfFrames = 2"));
        let prof = p(Family::DivisionEmblem);
        let files = final_files(&prof, prof.canvas, "iron_pact", "wolf").unwrap();
        assert!(sprite_entries(&prof, "iron_pact", "wolf", &files, None).is_err(), "no number, no symbol name");
    }

    #[test]
    fn export_round_trips_pixels() {
        let prof = p(Family::IdeaSpirit);
        let mut img = Rgba::new(60, 68);
        img.set_px(10, 10, [9, 8, 7, 200]);
        let files = final_files(&prof, prof.canvas, "f", "a").unwrap();
        let bytes = export_bytes(&img, &files[0], [60, 68]).unwrap();
        let back = decode_exported(&bytes, files[0].container).unwrap();
        assert_eq!(back.pixels_sha256(), img.pixels_sha256());
    }

    #[test]
    fn bad_names_are_rejected() {
        let prof = p(Family::IdeaSpirit);
        for bad in ["Bad", "a b", "../x", ""] {
            assert!(final_files(&prof, prof.canvas, "f", bad).is_err(), "{bad}");
        }
        assert!(final_files(&prof, prof.canvas, "Bad Slug", "a").is_err());
    }
}
