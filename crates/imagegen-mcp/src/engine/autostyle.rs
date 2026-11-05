//! Automatic, mod-first style references per asset family, as the engine uses them.
//!
//! The selection itself lives in `imagegen_core::stylesrc`. This file adds what needs the engine: the project's
//! pinned styles, the game folder and the family's profiles.

use imagegen_core::family::{Family, GuiTextureKind};
use imagegen_core::profile::{Profile, ProfileSet};
use imagegen_core::style::{self, StyleProfile, StyleWhy};
use imagegen_core::stylesrc::{self, Resolved, SourceCtx};
use imagegen_core::validate::StyleModel;

use super::Engine;

impl Engine {
    /// The pinned style that is tier 1 for a family, with the reason it was chosen: the one the caller named
    /// (`explicit`), else the project's style that is applied automatically to the family. A style is applied
    /// automatically only when its non-empty `families` list names the family or it has `default: true`
    /// (see [`StyleProfile::auto_applies_to`]); a style with no families and no `default` is never applied by
    /// itself. A style that names the family ranks above a `default: true` style for all families, then by name.
    pub(crate) fn applied_style_for(&self, family: Family, explicit: Option<&StyleProfile>) -> Option<(StyleProfile, StyleWhy)> {
        if let Some(s) = explicit {
            return Some((s.clone(), StyleWhy::Explicit));
        }
        let mut found: Vec<(u8, StyleProfile, StyleWhy)> = style::list(&self.fs)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|name| style::load(&self.fs, &name).ok())
            .filter(|s| !s.references.is_empty() && s.verify_references(&self.fs).is_ok())
            .filter_map(|s| {
                let why = s.auto_applies_to(family)?;
                Some((u8::from(s.families.is_empty()), s, why))
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.cmp(&b.1.name)));
        found.into_iter().next().map(|(_, s, why)| (s, why))
    }

    /// [`Engine::applied_style_for`] without the reason.
    pub(crate) fn pinned_style_for(&self, family: Family, explicit: Option<&StyleProfile>) -> Option<StyleProfile> {
        self.applied_style_for(family, explicit).map(|(s, _)| s)
    }

    /// Resolve the style sources of a family (tiers, chosen references, statistics, drift model).
    pub(crate) fn resolve_style(
        &self,
        profile: &Profile,
        pinned: Option<&StyleProfile>,
        max_send: usize,
        scan_all: bool,
        gui_kind: Option<GuiTextureKind>,
    ) -> Resolved {
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
            gui_kind,
        })
    }

    /// The style distribution the `style` check compares with, when some tier has enough images.
    pub(crate) fn style_model(&self, profile: &Profile) -> Option<StyleModel> {
        let pinned = self.pinned_style_for(profile.family, None);
        let r = self.resolve_style(profile, pinned.as_ref(), 1, false, None);
        let (tier, drift) = (r.tier?, r.drift?);
        Some(StyleModel { drift, tier: tier.number(), tier_name: tier.label().to_string(), subject: r.subject.unwrap_or_default() })
    }
}
