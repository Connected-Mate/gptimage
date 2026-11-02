//! Achievement icon triplet: the supplied background sits beneath the three 64x64 state layers.
//!
//! Port of the kit's `process_achievement_icons.py`. The completed state goes over the "completed" background; the
//! grey and not-eligible states both go over the "grey" background. Each state layer keeps its native canvas and
//! position, and the composite uses Pillow's integer `alpha_composite`, so the result is byte-identical to the kit.
//!
//! The two template PNGs are Paradox-derived and are not part of this repository; callers pass their pixels in
//! (or paths via [`load_templates_from_files`]). The kit pins the SHA-256 of its own copies and checks them when
//! they are used unmodified; [`load_templates_from_files`] does the same when asked to.
//!
//! Not ported: the directory scan that pairs `<id>`, `<id>_grey` and `<id>_not_eligible` files, the staging
//! directory, the review PNGs and the audit mode. Those are file orchestration and belong to the MCP engine.

use std::path::Path;

use crate::error::{Error, Result};
use crate::hashing::sha256_hex;
use crate::pixels::{decode, DecodeLimits, Rgba};
use crate::recipes::pillow::alpha_composite;

/// Achievement icons are 64x64.
pub const IMAGE_SIZE: u32 = 64;
/// Width of the outer ring that must differ from the template for a source to count as "not yet templated".
pub const TEMPLATE_BORDER_GUARD: u32 = 6;
/// SHA-256 (upper case) of the kit's `achievement_template.png`.
pub const COMPLETED_TEMPLATE_SHA256: &str = "248DB006611EB3942550C43DF83802AA6FB24761035FC928B5D34586C0C4C5BA";
/// SHA-256 (upper case) of the kit's `achievement_template_grey.png`.
pub const GREY_TEMPLATE_SHA256: &str = "70E073694C1A7D9FE40C63B1EB2E987A8A45B3FFD15CCF789EEAA5B843B90022";

/// The two backgrounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AchievementTemplates {
    /// Background under the completed state.
    pub completed: Rgba,
    /// Background under the grey and not-eligible states.
    pub grey: Rgba,
}

/// The three composited states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AchievementTriplet {
    /// `<id>.dds` pixels.
    pub completed: Rgba,
    /// `<id>_grey.dds` pixels.
    pub grey: Rgba,
    /// `<id>_not_eligible.dds` pixels.
    pub not_eligible: Rgba,
}

/// File name stems of the three outputs for a base id: `id`, `id_grey`, `id_not_eligible`.
pub fn state_stems(achievement_id: &str) -> Result<[String; 3]> {
    validate_id(achievement_id)?;
    Ok([achievement_id.to_string(), format!("{achievement_id}_grey"), format!("{achievement_id}_not_eligible")])
}

/// The id must be one plain filename stem and must not already be a state variant.
pub fn validate_id(id: &str) -> Result<()> {
    let bad = id.is_empty() || id == "." || id == ".." || id.contains(['/', '\\', ':']);
    if bad {
        return Err(Error::Invalid(format!("achievement id must be a single exact filename stem: {id:?}")));
    }
    let lower = id.to_ascii_lowercase();
    if lower.ends_with("_grey") || lower.ends_with("_not_eligible") {
        return Err(Error::Invalid(format!("achievement id must be the base id, not a grey or not-eligible variant: {id:?}")));
    }
    Ok(())
}

fn validate_template(img: &Rgba, label: &str) -> Result<()> {
    if (img.width, img.height) != (IMAGE_SIZE, IMAGE_SIZE) {
        return Err(Error::Invalid(format!("{label} must be exactly {IMAGE_SIZE}x{IMAGE_SIZE}; got {}x{}", img.width, img.height)));
    }
    let (min, max) = img.data.chunks_exact(4).fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[3]), hi.max(p[3])));
    if min < 254 || max != 255 {
        return Err(Error::Invalid(format!("{label} must have alpha range 254..255; got {min}..{max}")));
    }
    Ok(())
}

/// Wrap two background images, checking size and alpha range as the kit does.
pub fn templates_from_pixels(completed: Rgba, grey: Rgba) -> Result<AchievementTemplates> {
    validate_template(&completed, "completed achievement background")?;
    validate_template(&grey, "grey achievement background")?;
    Ok(AchievementTemplates { completed, grey })
}

/// Read the two background files (any container [`decode`] accepts). With `verify_pinned_hashes` the files must be
/// byte-identical to the kit's copies.
pub fn load_templates_from_files(completed: &Path, grey: &Path, verify_pinned_hashes: bool) -> Result<AchievementTemplates> {
    let read = |path: &Path, label: &str, pinned: &str| -> Result<Rgba> {
        let bytes = std::fs::read(path).map_err(|e| Error::io(path.display(), e))?;
        if verify_pinned_hashes && sha256_hex(&bytes).to_ascii_uppercase() != pinned {
            return Err(Error::Invalid(format!("{label} hash changed: {}. Expected SHA-256 {pinned}", path.display())));
        }
        let name = path.file_name().and_then(|n| n.to_str());
        Ok(decode(&bytes, name, DecodeLimits::default())?.0)
    };
    let c = read(completed, "completed achievement background", COMPLETED_TEMPLATE_SHA256)?;
    let g = read(grey, "grey achievement background", GREY_TEMPLATE_SHA256)?;
    templates_from_pixels(c, g)
}

fn check_state(img: &Rgba, label: &str) -> Result<()> {
    if (img.width, img.height) != (IMAGE_SIZE, IMAGE_SIZE) {
        return Err(Error::Invalid(format!("source {label} must be exactly 64x64; got {}x{}", img.width, img.height)));
    }
    Ok(())
}

fn matches_template_border(img: &Rgba, template: &Rgba) -> bool {
    let g = TEMPLATE_BORDER_GUARD;
    for y in 0..IMAGE_SIZE {
        for x in 0..IMAGE_SIZE {
            let on_ring = x < g || x >= IMAGE_SIZE - g || y < g || y >= IMAGE_SIZE - g;
            if on_ring && img.px(x, y) != template.px(x, y) {
                return false;
            }
        }
    }
    true
}

/// Compose the triplet. Unless `allow_templated_sources` is set, a source whose outer ring equals its template's
/// ring is refused: it has almost certainly been through this step already.
pub fn achievement_triplet(
    templates: &AchievementTemplates,
    completed: &Rgba,
    grey: &Rgba,
    not_eligible: &Rgba,
    allow_templated_sources: bool,
) -> Result<AchievementTriplet> {
    check_state(completed, "completed state")?;
    check_state(grey, "grey state")?;
    check_state(not_eligible, "not-eligible state")?;
    if !allow_templated_sources {
        let hits: Vec<&str> =
            [("completed", completed, &templates.completed), ("grey", grey, &templates.grey), ("not-eligible", not_eligible, &templates.grey)]
                .into_iter()
                .filter(|(_, img, tpl)| matches_template_border(img, tpl))
                .map(|(label, _, _)| label)
                .collect();
        if !hits.is_empty() {
            return Err(Error::Invalid(format!(
                "source triplet appears already templated ({} state border matches the supplied template); \
                 pass allow_templated_sources only when reprocessing is intentional",
                hits.join(", ")
            )));
        }
    }
    let compose = |template: &Rgba, state: &Rgba| {
        let mut out = template.clone();
        alpha_composite(&mut out, state, 0, 0);
        out
    };
    Ok(AchievementTriplet {
        completed: compose(&templates.completed, completed),
        grey: compose(&templates.grey, grey),
        not_eligible: compose(&templates.grey, not_eligible),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn templates() -> AchievementTemplates {
        let mut c = Rgba::filled(64, 64, [200, 180, 40, 255]);
        let mut g = Rgba::filled(64, 64, [90, 90, 90, 254]);
        // distinct frames so the ring check has something to match
        for i in 0..64 {
            c.set_px(i, 0, [255, 255, 255, 255]);
            g.set_px(0, i, [10, 10, 10, 255]);
        }
        templates_from_pixels(c, g).unwrap()
    }

    fn state(seed: u8) -> Rgba {
        let mut img = Rgba::new(64, 64);
        for y in 10..54 {
            for x in 10..54 {
                img.set_px(x, y, [seed, (x * 4) as u8, (y * 4) as u8, 255]);
            }
        }
        // a soft edge ring
        for x in 10..54 {
            img.set_px(x, 10, [seed, 0, 0, 100]);
        }
        img
    }

    #[test]
    fn states_go_over_their_own_background() {
        let t = templates();
        let out = achievement_triplet(&t, &state(1), &state(2), &state(3), false).unwrap();
        // opaque source pixels replace the background
        assert_eq!(out.completed.px(30, 30), [1, 120, 120, 255]);
        assert_eq!(out.grey.px(30, 30)[0], 2);
        assert_eq!(out.not_eligible.px(30, 30)[0], 3);
        // outside the source the background shows through; grey and not-eligible share the grey one
        assert_eq!(out.completed.px(2, 30), t.completed.px(2, 30));
        assert_eq!(out.grey.px(2, 30), t.grey.px(2, 30));
        assert_eq!(out.not_eligible.px(2, 30), t.grey.px(2, 30));
        // a translucent pixel blends with the template under it
        let blended = out.completed.px(30, 10);
        assert!(blended[0] < 200 && blended[3] == 255);
        // the result is opaque wherever the template is
        assert!(out.completed.data.chunks_exact(4).all(|p| p[3] >= 254));
    }

    #[test]
    fn an_already_templated_source_is_refused_unless_allowed() {
        let t = templates();
        let templated = {
            let mut img = t.completed.clone();
            img.paste_over(&state(9), 0, 0); // ring untouched, centre differs
            img
        };
        let err = achievement_triplet(&t, &templated, &state(2), &state(3), false).unwrap_err();
        assert!(err.to_string().contains("already templated"), "{err}");
        assert!(achievement_triplet(&t, &templated, &state(2), &state(3), true).is_ok());
    }

    #[test]
    fn wrong_sizes_and_bad_templates_fail() {
        let t = templates();
        assert!(achievement_triplet(&t, &Rgba::new(32, 32), &state(2), &state(3), false).is_err());
        assert!(templates_from_pixels(Rgba::filled(64, 64, [1, 1, 1, 200]), t.grey.clone()).is_err());
        assert!(templates_from_pixels(Rgba::filled(32, 64, [1, 1, 1, 255]), t.grey.clone()).is_err());
    }

    #[test]
    fn ids_and_stems() {
        assert_eq!(state_stems("iron_wall").unwrap(), ["iron_wall", "iron_wall_grey", "iron_wall_not_eligible"]);
        assert!(validate_id("a/b").is_err());
        assert!(validate_id("..").is_err());
        assert!(validate_id("x_GREY").is_err());
        assert!(validate_id("").is_err());
    }
}
