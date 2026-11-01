//! Final paths, sprite names and file bytes for an installed asset.

use crate::error::{Error, Result};
use crate::family::Family;
use crate::naming::{expand, validate_flag_name, validate_snake};
use crate::pixels::Rgba;
use crate::profile::{Container, Profile};
use crate::resize::resize_lanczos3;
use crate::sprite::{framed_sprite, plain_sprite, shine_sprite, SpriteEntry};
use crate::{dds, tga};

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
    let mut files =
        vec![FinalFile { rel: format!("{folder}/{prefix}{asset}.{ext}"), variant: None, canvas: strip, container: profile.imagegen.container }];
    // companions the recipe makes next to the main file (the achievement states): same folder, suffixed name
    for key in crate::chain::recipe_of(profile).companion_keys().iter().filter(|k| k.starts_with('_')) {
        files.push(FinalFile {
            rel: format!("{folder}/{prefix}{asset}{key}.{ext}"),
            variant: Some((*key).to_string()),
            canvas: strip,
            container: profile.imagegen.container,
        });
    }
    Ok(files)
}

/// Sprite entries and the suggested `.gfx` file (project-relative, under `interface/`).
/// Returns `None` for families the engine loads by file name (flags, achievements).
pub fn sprite_entries(profile: &Profile, feature_slug: &str, asset: &str, files: &[FinalFile]) -> Result<Option<(String, Vec<SpriteEntry>)>> {
    let Some(spec) = &profile.imagegen.sprite else { return Ok(None) };
    let main = files.first().ok_or_else(|| Error::Invalid("no final files".into()))?;
    let prefix = resolved_prefix(profile, feature_slug, asset)?;
    let name = expand(&spec.name, feature_slug, asset, &prefix)?;
    let gfx_file = expand(&spec.gfx_file, feature_slug, asset, &prefix)?;
    let mut entries = vec![if profile.frames > 1 { framed_sprite(&name, &main.rel, profile.frames) } else { plain_sprite(&name, &main.rel) }];
    for companion in &spec.companions {
        match companion.as_str() {
            "shine" => entries.push(shine_sprite(&name, &main.rel)?),
            other => return Err(Error::Config(format!("unknown sprite companion '{other}'"))),
        }
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
        let (gfx, entries) = sprite_entries(&prof, "iron_pact", "steel_will", &files).unwrap().unwrap();
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
        assert!(sprite_entries(&prof, "iron_pact", "IRP", &files).unwrap().is_none());
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
        assert!(sprite_entries(&prof, "iron_pact", "hold_the_line", &files).unwrap().is_none());
    }

    #[test]
    fn sprite_names_follow_the_family_patterns() {
        let name_of = |f: Family, asset: &str| {
            let prof = p(f);
            let canvas = if prof.needs_mod_canvas() { [64, 64] } else { prof.canvas };
            let files = final_files(&prof, canvas, "iron_pact", asset).unwrap();
            sprite_entries(&prof, "iron_pact", asset, &files).unwrap().unwrap().1[0].name.clone()
        };
        assert_eq!(name_of(Family::IdeaSpirit, "iron_will"), "GFX_idea_iron_will");
        assert_eq!(name_of(Family::AdvisorCard, "gen_smith"), "GFX_idea_gen_smith");
        assert_eq!(name_of(Family::OfficerCorpsSpirit, "grit"), "GFX_idea_grit");
        assert_eq!(name_of(Family::Decision, "rally"), "GFX_decision_rally");
        assert_eq!(name_of(Family::StateModifier, "bunkers"), "GFX_modifiers_bunkers");
        assert_eq!(name_of(Family::Technology, "rifle_tech"), "GFX_rifle_tech_medium");
        assert_eq!(name_of(Family::AgencyLogo, "north"), "GFX_intelligence_agency_logo_north");
        assert_eq!(name_of(Family::CounterSmall, "inf"), "GFX_unit_inf_icon_small");
    }

    #[test]
    fn frame_strips_are_registered_with_their_frame_count_and_strip_width() {
        let prof = p(Family::CounterLarge);
        let files = final_files(&prof, prof.canvas, "iron_pact", "inf").unwrap();
        assert_eq!(files[0].canvas, [152, 42]);
        let (_, entries) = sprite_entries(&prof, "iron_pact", "inf", &files).unwrap().unwrap();
        assert!(entries[0].text.contains("noOfFrames = 2"));
        let prof = p(Family::Focus);
        let files = final_files(&prof, prof.canvas, "iron_pact", "x").unwrap();
        assert!(!sprite_entries(&prof, "iron_pact", "x", &files).unwrap().unwrap().1[0].text.contains("noOfFrames"));
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
