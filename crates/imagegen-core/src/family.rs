//! Asset families. A family names one HOI4 surface (a focus icon, an idea picture, a report card) and is the
//! only thing a caller chooses: the family alone selects canvas, frames, naming, references, processing and
//! validation (see `builtin.rs` and `profile.rs`). A family is fixed from the plan onward, so an asset made as
//! one family can never be validated, reviewed or installed as another.

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// A group of families that share a generation tool and a rubric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// Single-frame transparent icons.
    Icon,
    /// Opaque pictures and card art.
    Picture,
    /// Character portraits and advisor cards.
    Portrait,
    /// Country flags.
    Flag,
    /// Frame strips and unit-related art.
    Unit,
    /// Interface art with a mod-defined or unusual shape.
    Interface,
    /// Animated multi-frame sprites (`frameAnimatedSpriteType`).
    Animation,
}

impl Group {
    /// Every group, in documentation order.
    pub const ALL: [Group; 7] = [Group::Icon, Group::Picture, Group::Portrait, Group::Flag, Group::Unit, Group::Interface, Group::Animation];

    /// Stable snake_case id.
    pub fn as_str(self) -> &'static str {
        match self {
            Group::Icon => "icon",
            Group::Picture => "picture",
            Group::Portrait => "portrait",
            Group::Flag => "flag",
            Group::Unit => "unit",
            Group::Interface => "interface",
            Group::Animation => "animation",
        }
    }

    /// Name of the MCP generation tool of the group.
    pub fn tool_name(self) -> &'static str {
        match self {
            Group::Icon => "hoi4.image_icon",
            Group::Picture => "hoi4.image_picture",
            Group::Portrait => "hoi4.image_portrait",
            Group::Flag => "hoi4.image_flag",
            Group::Unit => "hoi4.image_unit",
            Group::Interface => "hoi4.image_interface",
            Group::Animation => "hoi4.image_animation",
        }
    }

    /// Families of the group, in a stable order.
    pub fn families(self) -> Vec<Family> {
        Family::ALL.iter().copied().filter(|f| f.group() == self).collect()
    }

    /// Ids of the families of the group (what a tool's `family` enum lists).
    pub fn family_ids(self) -> Vec<&'static str> {
        self.families().into_iter().map(Family::as_str).collect()
    }
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

macro_rules! families {
    ($( $variant:ident => ($id:literal, $group:ident, $label:literal), )+) => {
        /// A HOI4 graphical asset family.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum Family {
            $(
                #[doc = $label]
                $variant,
            )+
        }

        impl Family {
            /// Every family, in a stable order (grouped as the tools are).
            pub const ALL: &'static [Family] = &[$(Family::$variant),+];

            /// Stable snake_case id.
            pub fn as_str(self) -> &'static str {
                match self { $(Family::$variant => $id,)+ }
            }

            /// The group (and so the generation tool) the family belongs to.
            pub fn group(self) -> Group {
                match self { $(Family::$variant => Group::$group,)+ }
            }

            /// Short human label used in prompts.
            pub fn label(self) -> &'static str {
                match self { $(Family::$variant => $label,)+ }
            }
        }
    };
}

families! {
    Focus => ("focus", Icon, "national focus icon"),
    IdeaSpirit => ("idea_spirit", Icon, "idea and national spirit picture"),
    IdeaConcern => ("idea_concern", Icon, "industrial concern or manufacturer idea picture"),
    OfficerCorpsSpirit => ("officer_corps_spirit", Icon, "officer corps spirit icon"),
    Decision => ("decision", Icon, "decision or mission icon"),
    DecisionCategoryIcon => ("decision_category_icon", Icon, "decision category icon"),
    StateModifier => ("state_modifier", Icon, "state or dynamic modifier icon"),
    SpecialProjectIcon => ("special_project_icon", Icon, "special project icon"),
    Raid => ("raid", Icon, "raid icon"),
    MioDepartment => ("mio_department", Icon, "military industrial organization department icon"),
    MioTrait => ("mio_trait", Icon, "military industrial organization trait icon"),
    MioPolicy => ("mio_policy", Icon, "military industrial organization policy icon"),
    BopSide => ("bop_side", Icon, "balance of power side icon"),
    Medal => ("medal", Icon, "unit medal icon"),
    AgencyUpgrade => ("agency_upgrade", Icon, "intelligence agency upgrade icon"),
    OperationIcon => ("operation_icon", Icon, "intelligence operation icon"),
    CommanderTrait => ("commander_trait", Icon, "commander trait icon"),
    PersonalTrait => ("personal_trait", Icon, "personal trait icon"),
    Doctrine => ("doctrine", Icon, "doctrine icon"),
    ReportEvent => ("report_event", Picture, "report event picture"),
    NewsEvent => ("news_event", Picture, "news event picture"),
    DecisionCategoryPicture => ("decision_category_picture", Picture, "decision category picture"),
    OperationPicture => ("operation_picture", Picture, "intelligence operation picture"),
    Technology => ("technology", Picture, "technology picture"),
    Achievement => ("achievement", Picture, "achievement icon"),
    LeaderPortrait => ("leader_portrait", Portrait, "leader portrait"),
    CommanderPortrait => ("commander_portrait", Portrait, "commander portrait"),
    OperativePortrait => ("operative_portrait", Portrait, "operative portrait"),
    ScientistPortrait => ("scientist_portrait", Portrait, "scientist portrait"),
    AdvisorCard => ("advisor_card", Portrait, "advisor portrait card"),
    Flag => ("flag", Flag, "country flag"),
    CounterLarge => ("counter_large", Unit, "large division counter"),
    CounterSmall => ("counter_small", Unit, "small division counter"),
    DivisionEmblem => ("division_emblem", Unit, "division template emblem"),
    Building => ("building", Unit, "landmark building icon"),
    FactionLogo => ("faction_logo", Unit, "faction logo"),
    AgencyLogo => ("agency_logo", Unit, "intelligence agency logo"),
    ModifierStatIcon => ("modifier_stat_icon", Interface, "modifier tooltip stat icon"),
    GuiTexture => ("gui_texture", Interface, "GUI texture"),
    SuperEvent => ("super_event", Interface, "super event picture"),
    AnimBlendGlow => ("anim_blend_glow", Animation, "two-frame pulsing glow"),
    AnimBlendKeyframes => ("anim_blend_keyframes", Animation, "three-frame blended animation"),
    AnimScroll => ("anim_scroll", Animation, "scrolling animated strip"),
    AnimSpin => ("anim_spin", Animation, "spinning animated icon"),
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Family {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        Family::ALL.iter().copied().find(|c| c.as_str() == s).ok_or_else(|| Error::Invalid(format!("unknown family '{s}'")))
    }
}

/// Whether agents may produce the family with a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Automation {
    /// Generated, processed, validated and installed by this tool.
    Generate,
    /// Another step produces the art (a frame strip, a card built from a portrait); this tool only imports,
    /// validates and installs it.
    ImportOnly,
    /// A person runs the generator. Allowed providers are limited by project policy; a ComfyUI
    /// endpoint the maintainer configured is allowed for every asset (decision D3).
    UserRunOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for c in Family::ALL {
            assert_eq!(c.as_str().parse::<Family>().unwrap(), *c);
            assert!(seen.insert(c.as_str()));
            let json = serde_json::to_string(c).unwrap();
            assert_eq!(json, format!("\"{}\"", c.as_str()));
        }
        assert!("nope".parse::<Family>().is_err());
        assert!("focus_icon".parse::<Family>().is_err(), "the 0.1 category ids are gone");
    }

    #[test]
    fn groups_partition_the_families() {
        assert_eq!(Family::ALL.len(), 44);
        let mut total = 0;
        for g in Group::ALL {
            let fams = g.families();
            assert!(!fams.is_empty(), "{g}");
            assert!(fams.iter().all(|f| f.group() == g));
            total += fams.len();
        }
        assert_eq!(total, Family::ALL.len());
        assert_eq!(Group::Icon.families().len(), 19);
        assert_eq!(Group::Picture.families().len(), 6);
        assert_eq!(Group::Portrait.families().len(), 5);
        assert_eq!(Group::Flag.families(), vec![Family::Flag]);
        assert_eq!(Group::Unit.families().len(), 6);
        assert_eq!(Group::Interface.families().len(), 3);
        assert_eq!(Group::Animation.families().len(), 4);
        assert_eq!(Group::Animation.tool_name(), "hoi4.image_animation");
        assert_eq!(Family::Focus.group(), Group::Icon);
        assert_eq!(Family::Achievement.group(), Group::Picture);
        assert_eq!(Family::AdvisorCard.group(), Group::Portrait);
        assert_eq!(Group::Unit.tool_name(), "hoi4.image_unit");
    }
}
