//! Automatic, mod-first style references per asset family, as the engine uses them.
//!
//! The selection itself lives in `imagegen_core::stylesrc`. This file adds what needs the engine: the project's
//! pinned styles, the game folder and the family's profiles.

use imagegen_core::family::Family;
use imagegen_core::profile::{Profile, ProfileSet};
use imagegen_core::style::{self, StyleProfile};
use imagegen_core::stylesrc::{self, Resolved, SourceCtx};
use imagegen_core::validate::StyleModel;

use super::Engine;

impl Engine {
    /// The pinned style that is tier 1 for a family: the one the caller named, or else the project's style that
    /// lists the family (a style for all families ranks below one that names it), by name.
    pub(crate) fn pinned_style_for(&self, family: Family, explicit: Option<&StyleProfile>) -> Option<StyleProfile> {
        if let Some(s) = explicit {
            return Some(s.clone());
        }
        let mut found: Vec<(bool, StyleProfile)> = style::list(&self.fs)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|name| style::load(&self.fs, &name).ok())
            .filter(|s| s.applies_to(family) && !s.references.is_empty() && s.verify_references(&self.fs).is_ok())
            .map(|s| (s.families.is_empty(), s))
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.cmp(&b.1.name)));
        found.into_iter().next().map(|(_, s)| s)
    }

    /// Resolve the style sources of a family (tiers, chosen references, statistics, drift model).
    pub(crate) fn resolve_style(&self, profile: &Profile, pinned: Option<&StyleProfile>, max_send: usize, scan_all: bool) -> Resolved {
        let game = self.game_dir();
        let builtin = ProfileSet::builtin();
        stylesrc::resolve(&SourceCtx {
            fs: &self.fs,
            game_dir: game.as_deref(),
            profile,
            builtin: builtin.for_family(profile.family),
            pinned,
            max_send,
            scan_all,
        })
    }

    /// The style distribution the `style` check compares with, when some tier has enough images.
    pub(crate) fn style_model(&self, profile: &Profile) -> Option<StyleModel> {
        let pinned = self.pinned_style_for(profile.family, None);
        let r = self.resolve_style(profile, pinned.as_ref(), 1, false);
        let (tier, drift) = (r.tier?, r.drift?);
        Some(StyleModel { drift, tier: tier.number(), tier_name: tier.label().to_string(), subject: r.subject.unwrap_or_default() })
    }
}
