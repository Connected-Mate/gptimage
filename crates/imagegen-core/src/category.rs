//! Asset categories. A category names a HOI4 surface; a profile (see `profile.rs`) says how to make it.

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// A HOI4 graphical asset category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// National focus icon (`gfx/interface/goals`).
    FocusIcon,
    /// Idea and national spirit picture (`gfx/interface/ideas`).
    IdeaIcon,
    /// Decision icon (`gfx/interface/decisions`).
    DecisionIcon,
    /// Small decision category icon (52x40).
    DecisionCategoryIcon,
    /// Decision category picture (114x101).
    DecisionCategoryPicture,
    /// Small technology icon.
    TechIconSmall,
    /// Medium technology icon.
    TechIconMedium,
    /// Report event picture (210x176).
    EventReport,
    /// News event picture (397x153).
    EventNews,
    /// Super event picture (457x328, defined by the optional Super Events runtime).
    EventSuper,
    /// Country flag in three TGA sizes.
    FlagSet,
    /// Military industrial organization icon.
    MioIcon,
    /// Intelligence agency logo.
    AgencyLogo,
    /// Officer corps spirit icon.
    OfficerSpirit,
    /// Leader, commander or operative portrait (156x210).
    LeaderPortrait,
    /// Advisor portrait icon (65x67). Owned by another kit tool: import only.
    AdvisorIcon,
    /// Achievement triplet member (64x64). Owned by another kit tool: import only.
    AchievementIcon,
    /// Large division counter. Needs an exact vanilla reference: import only.
    CounterLarge,
    /// Small division counter. Needs an exact vanilla reference: import only.
    CounterSmall,
    /// A general GUI texture whose size comes from its consumer.
    GuiTexture,
    /// Anything else; the canvas must be given explicitly.
    Custom,
}

impl Category {
    /// Every category, in a stable order.
    pub const ALL: [Category; 21] = [
        Category::FocusIcon,
        Category::IdeaIcon,
        Category::DecisionIcon,
        Category::DecisionCategoryIcon,
        Category::DecisionCategoryPicture,
        Category::TechIconSmall,
        Category::TechIconMedium,
        Category::EventReport,
        Category::EventNews,
        Category::EventSuper,
        Category::FlagSet,
        Category::MioIcon,
        Category::AgencyLogo,
        Category::OfficerSpirit,
        Category::LeaderPortrait,
        Category::AdvisorIcon,
        Category::AchievementIcon,
        Category::CounterLarge,
        Category::CounterSmall,
        Category::GuiTexture,
        Category::Custom,
    ];

    /// Stable snake_case id.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::FocusIcon => "focus_icon",
            Category::IdeaIcon => "idea_icon",
            Category::DecisionIcon => "decision_icon",
            Category::DecisionCategoryIcon => "decision_category_icon",
            Category::DecisionCategoryPicture => "decision_category_picture",
            Category::TechIconSmall => "tech_icon_small",
            Category::TechIconMedium => "tech_icon_medium",
            Category::EventReport => "event_report",
            Category::EventNews => "event_news",
            Category::EventSuper => "event_super",
            Category::FlagSet => "flag_set",
            Category::MioIcon => "mio_icon",
            Category::AgencyLogo => "agency_logo",
            Category::OfficerSpirit => "officer_spirit",
            Category::LeaderPortrait => "leader_portrait",
            Category::AdvisorIcon => "advisor_icon",
            Category::AchievementIcon => "achievement_icon",
            Category::CounterLarge => "counter_large",
            Category::CounterSmall => "counter_small",
            Category::GuiTexture => "gui_texture",
            Category::Custom => "custom",
        }
    }

    /// Short human label used in prompts.
    pub fn label(self) -> &'static str {
        match self {
            Category::FocusIcon => "national focus icon",
            Category::IdeaIcon => "idea / national spirit picture",
            Category::DecisionIcon => "decision icon",
            Category::DecisionCategoryIcon => "decision category icon",
            Category::DecisionCategoryPicture => "decision category picture",
            Category::TechIconSmall => "technology icon",
            Category::TechIconMedium => "wide technology icon",
            Category::EventReport => "report event picture",
            Category::EventNews => "news event picture",
            Category::EventSuper => "super event picture",
            Category::FlagSet => "country flag",
            Category::MioIcon => "military industrial organization icon",
            Category::AgencyLogo => "intelligence agency logo",
            Category::OfficerSpirit => "officer corps spirit icon",
            Category::LeaderPortrait => "leader portrait",
            Category::AdvisorIcon => "advisor portrait icon",
            Category::AchievementIcon => "achievement icon",
            Category::CounterLarge => "large division counter",
            Category::CounterSmall => "small division counter",
            Category::GuiTexture => "GUI texture",
            Category::Custom => "custom image",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Category {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        Category::ALL
            .iter()
            .copied()
            .find(|c| c.as_str() == s)
            .ok_or_else(|| Error::Invalid(format!("unknown category '{s}'")))
    }
}

/// Whether agents may produce the category with a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Automation {
    /// Generated, processed, validated and installed by this tool.
    Generate,
    /// Another kit tool produces the art; this tool only imports, validates and installs it.
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
        for c in Category::ALL {
            assert_eq!(c.as_str().parse::<Category>().unwrap(), c);
            assert!(seen.insert(c.as_str()));
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{}\"", c.as_str()));
        }
        assert!("nope".parse::<Category>().is_err());
    }
}
