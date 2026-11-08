//! Model class of a family: which tier of a provider's model line it is generated with.
//!
//! A provider that has a fast and a higher-quality model (the OpenAI API route: `gpt-image-2.6-flare` and
//! `gpt-image-2.6-sunburst`) uses this to pick one per family. The maintainer's rule: small sprites and icons run on
//! the fast model, and everything that shows more detail runs on the quality model. An explicit model in the request,
//! the style or the provider config always wins over the class.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::family::{Family, Group};

/// A canvas with at least this many pixels is "clearly larger than icon size". The biggest icon-sized canvas is the
/// 161x98 special project icon (15 778 pixels), the smallest picture is the 210x176 report event (36 960).
pub const LARGE_CANVAS_PIXELS: u32 = 20_000;

/// Which tier of a provider's models a family is made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelClass {
    /// The provider's fast everyday model: icons and other small sprites.
    Fast,
    /// The provider's higher-quality model: large images, portraits, animation frames, 3D model references, edits.
    Quality,
}

impl ModelClass {
    /// Stable snake_case id.
    pub fn as_str(self) -> &'static str {
        match self {
            ModelClass::Fast => "fast",
            ModelClass::Quality => "quality",
        }
    }

    /// The class of a family drawn on `canvas` (the planned size per frame; `[0, 0]` when nobody has set one).
    ///
    /// Explicit entries: every portrait, every animated family, `super_event` and `model_reference` are `Quality`.
    /// Any other family is `Quality` when its canvas has [`LARGE_CANVAS_PIXELS`] or more pixels and `Fast` otherwise.
    pub fn of(family: Family, canvas: [u32; 2]) -> ModelClass {
        match family {
            Family::SuperEvent | Family::ModelReference => ModelClass::Quality,
            _ if matches!(family.group(), Group::Portrait | Group::Animation) => ModelClass::Quality,
            _ if canvas[0].saturating_mul(canvas[1]) >= LARGE_CANVAS_PIXELS => ModelClass::Quality,
            _ => ModelClass::Fast,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::profiles;

    fn class(f: Family) -> ModelClass {
        let p = profiles().into_iter().find(|p| p.family == f).unwrap();
        ModelClass::of(f, p.canvas)
    }

    #[test]
    fn large_families_portraits_animations_and_model_references_are_quality() {
        for f in [
            Family::SuperEvent,
            Family::ModelReference,
            Family::ReportEvent,
            Family::NewsEvent,
            Family::OperationPicture,
            Family::LeaderPortrait,
            Family::CommanderPortrait,
            Family::OperativePortrait,
            Family::ScientistPortrait,
            Family::AdvisorCard,
            Family::AnimBlendGlow,
            Family::AnimBlendKeyframes,
            Family::AnimScroll,
            Family::AnimSpin,
        ] {
            assert_eq!(class(f), ModelClass::Quality, "{f}");
        }
    }

    #[test]
    fn icons_and_small_sprites_are_fast() {
        for f in [
            Family::Focus,
            Family::Decision,
            Family::IdeaSpirit,
            Family::Technology,
            Family::SpecialProjectIcon,
            Family::Flag,
            Family::CounterLarge,
            Family::AgencyLogo,
            Family::DecisionCategoryPicture,
            Family::Doctrine,
            Family::ModifierStatIcon,
        ] {
            assert_eq!(class(f), ModelClass::Fast, "{f}");
        }
    }

    #[test]
    fn a_free_canvas_decides_gui_textures_by_size() {
        assert_eq!(ModelClass::of(Family::GuiTexture, [60, 40]), ModelClass::Fast);
        assert_eq!(ModelClass::of(Family::GuiTexture, [400, 300]), ModelClass::Quality);
    }

    #[test]
    fn every_builtin_family_has_a_class() {
        let quality: Vec<_> =
            profiles().into_iter().filter(|p| ModelClass::of(p.family, p.canvas) == ModelClass::Quality).map(|p| p.family).collect();
        assert_eq!(quality.len(), 14, "{quality:?}");
    }
}
