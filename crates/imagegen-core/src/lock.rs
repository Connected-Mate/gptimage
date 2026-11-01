//! The family lock: an image made for one family can never be passed off as another.
//!
//! Two rules live here. First, a size rule for import: an image whose native size is not one the family
//! accepts but is the canonical or an accepted size of another family is refused with `GFX-FAMILY-MISMATCH`
//! (a 65x67 advisor card must not become a 100x88 focus icon by being fitted). Second, the codes the pixel
//! reuse guard in `reuse.rs` reports. The rest of the lock (every record carries the family, no tool takes a
//! family for an artifact) is structural and lives in the records and the engine.

use crate::error::{Error, Result, RetryClass};
use crate::family::Family;
use crate::profile::ProfileSet;

/// Error code: the image's size belongs to another family.
pub const MISMATCH_CODE: &str = "GFX-FAMILY-MISMATCH";

/// Error code: the image's pixels are an asset already installed for another family.
pub const REUSE_CODE: &str = "GFX-FAMILY-REUSE";

/// Families other than `family` for which `native` (the size of a whole image, all frames) is the canonical
/// or an accepted size, in a stable order.
pub fn other_families_with_size(set: &ProfileSet, family: Family, native: [u32; 2]) -> Vec<Family> {
    Family::ALL
        .iter()
        .copied()
        .filter(|f| *f != family)
        .filter(|f| set.for_family(*f).map(|p| p.accepted_strips().contains(&native)).unwrap_or(false))
        .collect()
}

/// Refuse an import whose native size is another family's but not its own.
///
/// An image of an unfamiliar size passes: the family's own chain fits it (within its upscale limit). A family
/// that has no fixed canvas (`gui_texture`, `super_event`) accepts any size.
pub fn check_native_size(set: &ProfileSet, family: Family, native: [u32; 2]) -> Result<()> {
    let Some(profile) = set.for_family(family) else {
        return Err(Error::NotFound(format!("no profile for family {family}")));
    };
    if profile.needs_mod_canvas() || profile.accepted_strips().contains(&native) {
        return Ok(());
    }
    let others = other_families_with_size(set, family, native);
    if others.is_empty() {
        return Ok(());
    }
    let list: Vec<&str> = others.iter().map(|f| f.as_str()).collect();
    let own: Vec<String> = profile.accepted_strips().iter().map(|s| format!("{}x{}", s[0], s[1])).collect();
    Err(Error::refused(
        MISMATCH_CODE,
        format!(
            "this image is {}x{}, which is the size of {} art, not of {family} ({}). An asset is made for one family only: import it as {}, or bring art made for {family}",
            native[0],
            native[1],
            list.join(" or "),
            own.join(" or "),
            if list.len() == 1 { list[0].to_string() } else { format!("one of {}", list.join(", ")) },
        ),
        RetryClass::Terminal,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_advisor_card_size_is_refused_as_a_focus_icon() {
        let set = ProfileSet::builtin();
        let err = check_native_size(&set, Family::Focus, [65, 67]).unwrap_err();
        assert_eq!(err.code(), MISMATCH_CODE);
        let msg = err.to_string();
        assert!(msg.contains("advisor_card") && msg.contains("65x67") && msg.contains("100x88"), "{msg}");
        assert!(check_native_size(&set, Family::AdvisorCard, [65, 67]).is_ok());
    }

    #[test]
    fn own_accepted_sizes_and_unfamiliar_sizes_pass() {
        let set = ProfileSet::builtin();
        assert!(check_native_size(&set, Family::Focus, [102, 90]).is_ok());
        assert!(check_native_size(&set, Family::Focus, [100, 88]).is_ok());
        assert!(check_native_size(&set, Family::Focus, [1024, 1024]).is_ok(), "a provider-sized image is fitted by the chain");
        assert!(check_native_size(&set, Family::Decision, [48, 48]).is_err(), "48x48 is a MIO icon size");
        assert!(check_native_size(&set, Family::GuiTexture, [65, 67]).is_ok(), "a mod-defined canvas accepts any size");
    }

    #[test]
    fn strips_are_compared_whole_so_a_single_frame_is_not_a_strip() {
        let set = ProfileSet::builtin();
        // 152x42 is the large counter strip; a 76x42 frame is a division emblem
        assert!(check_native_size(&set, Family::CounterLarge, [152, 42]).is_ok());
        let err = check_native_size(&set, Family::CounterLarge, [76, 42]).unwrap_err();
        assert!(err.to_string().contains("division_emblem"));
        assert_eq!(other_families_with_size(&set, Family::Focus, [152, 42]), vec![Family::CounterLarge]);
    }

    #[test]
    fn the_mods_own_profile_decides_what_each_family_accepts() {
        use crate::profile::RowOverride;
        let mut project = std::collections::BTreeMap::new();
        project.insert("focus".to_string(), RowOverride { canvas: Some([94, 86]), ..Default::default() });
        project.insert("idea_spirit".to_string(), RowOverride { canvas: Some([94, 86]), ..Default::default() });
        let set = ProfileSet::load(None, &project).unwrap();
        // a mod that gives two families the same size cannot be told apart by size, so nothing is refused for it
        assert!(check_native_size(&set, Family::Focus, [94, 86]).is_ok());
        // but the vanilla focus size now belongs to nobody else, and 65x67 still belongs to advisor cards
        assert!(check_native_size(&set, Family::Focus, [65, 67]).is_err());
    }
}
