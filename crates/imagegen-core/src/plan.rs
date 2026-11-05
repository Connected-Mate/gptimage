//! Plans: a dry run that resolves everything a generation needs and costs nothing.
//!
//! A plan fixes the profile, the prompt text, the references (by hash), the provider and model, the size,
//! the estimate and every output path. Its hash covers all of that, so `generate` can prove the request it
//! sends is the one that was planned, and the install gate can prove the plan was not edited afterwards.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::family::Family;
use crate::hashing::{canonical_hash, sha256_hex};
use crate::profile::Profile;
use crate::records::{ProfileRef, RefSummary};
use crate::refs::{LoadedRef, ReferenceInput};
use crate::style::StyleProfile;
use crate::stylesrc::StyleReport;

/// What a caller asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanRequest {
    /// Asset family. The only thing that selects canvas, frames, naming, references, processing and validation.
    pub family: Family,
    /// Feature folder name (snake_case).
    pub feature_slug: String,
    /// Asset name (snake_case; a country tag for flags).
    pub asset: String,
    /// What to depict, in plain words (at most 4000 characters).
    pub brief: String,
    /// Project style profile to apply.
    #[serde(default)]
    pub style: Option<String>,
    /// Reference images.
    #[serde(default)]
    pub references: Vec<ReferenceInput>,
    /// Provider id; defaults to the style's or the project's default.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model id; defaults to the provider's recommended model.
    #[serde(default)]
    pub model: Option<String>,
    /// Override the profile's background rule.
    #[serde(default)]
    pub transparent: Option<bool>,
    /// Candidates to produce in one go (1 to 4); each is a separate provider call.
    #[serde(default)]
    pub variants: Option<u8>,
    /// Refuse the plan if the estimate exceeds this many USD.
    #[serde(default)]
    pub budget_usd: Option<f64>,
    /// Quality tier where the model has tiers.
    #[serde(default)]
    pub quality: Option<String>,
    /// Canvas for families that have none (`gui_texture`, `super_event`): the mod defines it.
    #[serde(default)]
    pub canvas: Option<[u32; 2]>,
    /// The kind of a `gui_texture` (button, frame, panel, background): adds the kind's vanilla form to the prompt and
    /// picks vanilla references of that kind. Refused for other families. Left out when unset, so plans made before
    /// the field existed keep their hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_kind: Option<crate::family::GuiTextureKind>,
}

/// The prompt a plan will send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PromptPlan {
    /// Prompt text.
    pub text: String,
    /// Things to avoid, kept separate for providers that take a negative prompt.
    pub negative: Vec<String>,
    /// SHA-256 of the text.
    pub sha256: String,
}

/// Concrete generation settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GenerationPlan {
    /// Requested pixel width.
    pub width: u32,
    /// Requested pixel height.
    pub height: u32,
    /// Whether the provider is asked for real transparency.
    pub transparent: bool,
    /// Aspect ratio as `w:h` of the target canvas.
    pub aspect: String,
}

/// Cost estimate as stored in the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanEstimate {
    /// Estimated USD for one provider call.
    pub usd_per_call: f64,
    /// Calls the plan makes (variants).
    pub calls: u32,
    /// Total estimate.
    pub usd: f64,
    /// How the number was derived.
    pub basis: String,
    /// True when the provider cannot report usage; the reserve then stands.
    pub usage_unknown: bool,
    /// Who pays: `user_key`, `local` or `plan_quota`.
    pub funding: String,
}

/// Where outputs go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanPaths {
    /// Raw candidates (`source_png`).
    pub candidate_dir: String,
    /// Processed art (`processed_png`).
    pub processed_dir: String,
    /// Previews and contact sheets.
    pub preview_dir: String,
    /// Prompt records.
    pub prompts_dir: String,
    /// Files an install would write.
    pub final_files: Vec<String>,
    /// Handoff note.
    pub handoff: String,
}

/// A complete plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    /// `plan-` and the first 16 hex digits of the hash.
    pub plan_id: String,
    /// SHA-256 over the plan's content.
    pub plan_hash: String,
    /// When the plan was made (not part of the hash).
    pub created: String,
    /// Normalized request.
    pub request: PlanRequest,
    /// `feature.asset`
    pub asset_id: String,
    /// Profile used.
    pub profile: ProfileRef,
    /// Canvas per frame.
    pub canvas: [u32; 2],
    /// Size the model is asked for when it is not the canvas (an advisor card is made from a tall portrait).
    #[serde(default)]
    pub source_canvas: Option<[u32; 2]>,
    /// Frames in the strip.
    pub frames: u32,
    /// Provider id.
    pub provider: String,
    /// Exact model id.
    pub model: String,
    /// Quality tier.
    pub quality: Option<String>,
    /// Generation settings.
    pub generation: GenerationPlan,
    /// Prompt.
    pub prompt: PromptPlan,
    /// References (hashes of the clean PNGs that will be sent).
    pub references: Vec<RefSummary>,
    /// Style used and its hash.
    pub style: Option<String>,
    /// Hash of the style file content.
    pub style_sha256: Option<String>,
    /// Where the automatic style references came from: tiers, chosen references with reasons, statistics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_sources: Option<StyleReport>,
    /// Which style was applied and why, what each source contributed and which form wins (new in 0.3.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_selection: Option<StyleSelection>,
    /// Estimate.
    pub estimate: PlanEstimate,
    /// Output paths.
    pub paths: PlanPaths,
    /// Candidates to produce.
    pub variants: u8,
    /// Approvals the caller must give before spending.
    pub approvals: Vec<String>,
    /// Warnings.
    pub warnings: Vec<String>,
}

/// The references one source contributed to a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StyleContribution {
    /// `style 'name'` for a pinned style, else the tier name.
    pub source: String,
    /// Tier (1 to 4).
    pub tier: u8,
    /// Paths of the references this source sends to the provider.
    pub references: Vec<String>,
}

/// Which form rule wins when the prompt states a vanilla form and the references show another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FormPrecedence {
    /// The references come from the mod itself (tier 2 or 3): their form wins over the vanilla form rule.
    ModReferences,
    /// The request sends its own style anchors: their form wins over the vanilla form rule, and no vanilla reference
    /// is sent beside them.
    CallerReferences,
    /// No mod reference is sent: the family's vanilla form rule stands alone.
    VanillaRule,
}

impl FormPrecedence {
    /// The sentence added to the prompt, or `None` when the vanilla rule stands alone.
    pub fn prompt_line(self) -> Option<&'static str> {
        match self {
            FormPrecedence::ModReferences => Some(
                "The reference images come from this mod. Match the form and framing of the mod's reference images; the vanilla form described under Requirements applies only where they do not show one.",
            ),
            FormPrecedence::CallerReferences => Some(
                "The style anchor images were chosen for this image. Match their form, framing, materials and palette; the vanilla form described under Requirements applies only where they do not show one.",
            ),
            FormPrecedence::VanillaRule => None,
        }
    }
}

/// Which style a plan applied and why, what each source contributed, and which form wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StyleSelection {
    /// Name of the applied style profile, if any.
    pub applied: Option<String>,
    /// `explicit`, `default`, `family_list` or `none`.
    pub why: String,
    /// One sentence for a person.
    pub detail: String,
    /// References each source contributed (a pinned style and the tier that was used).
    pub contributions: Vec<StyleContribution>,
    /// Which form wins when references and the vanilla form rule disagree.
    pub form_precedence: FormPrecedence,
    /// One sentence for a person.
    pub form_note: String,
}

impl Plan {
    /// Hash over everything except `plan_id`, `plan_hash` and `created`.
    pub fn compute_hash(&self) -> String {
        let mut v: Value = serde_json::to_value(self).expect("plan serializes");
        if let Some(o) = v.as_object_mut() {
            o.remove("plan_id");
            o.remove("plan_hash");
            o.remove("created");
        }
        canonical_hash(&v)
    }

    /// Fill `plan_hash` and `plan_id`.
    pub fn seal(&mut self) {
        self.plan_hash = self.compute_hash();
        self.plan_id = format!("plan-{}", &self.plan_hash[..16]);
    }

    /// True when the stored hash still matches the content.
    pub fn is_intact(&self) -> bool {
        self.plan_hash == self.compute_hash() && self.plan_id == format!("plan-{}", &self.plan_hash[..16])
    }
}

/// Path of a stored plan.
pub fn plan_path(plan_id: &str) -> String {
    format!(".hoi4-imagegen/plans/{plan_id}.json")
}

/// Check a plan id before using it in a path.
pub fn valid_plan_id(id: &str) -> bool {
    id.len() == 21 && id.starts_with("plan-") && id[5..].chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Assemble the prompt deterministically from the profile, the brief, the style and the references.
pub fn build_prompt(
    profile: &Profile,
    canvas: [u32; 2],
    brief: &str,
    style: Option<&StyleProfile>,
    refs: &[LoadedRef],
    transparent: bool,
    measured_style: Option<&str>,
) -> PromptPlan {
    build_prompt_with_form(profile, canvas, brief, style, refs, transparent, measured_style, FormPrecedence::VanillaRule)
}

/// [`build_prompt`] with the form precedence: when the references come from the mod itself, the prompt says their
/// form wins over the vanilla form the family's rules state.
#[allow(clippy::too_many_arguments)]
pub fn build_prompt_with_form(
    profile: &Profile,
    canvas: [u32; 2],
    brief: &str,
    style: Option<&StyleProfile>,
    refs: &[LoadedRef],
    transparent: bool,
    measured_style: Option<&str>,
    form: FormPrecedence,
) -> PromptPlan {
    let subst = |s: &str| s.replace("{w}", &canvas[0].to_string()).replace("{h}", &canvas[1].to_string());
    let mut text = format!("Create one Hearts of Iron IV {}: {}\n", profile.family.label(), brief.trim());
    text.push_str("\nRequirements:\n");
    for rule in &profile.imagegen.prompt.rules {
        text.push_str(&format!("- {}\n", subst(rule)));
    }
    if transparent && !profile.imagegen.prompt.rules.iter().any(|r| r.to_ascii_lowercase().contains("transparent")) {
        text.push_str("- Real transparent background (alpha), not a drawn checkerboard.\n");
    }
    let note = style.filter(|s| !s.description.is_empty() || !s.prompt_fragments.is_empty() || !s.palette_notes.is_empty());
    if note.is_some() || measured_style.is_some() {
        text.push_str("\nStyle:\n");
    }
    if let Some(s) = note {
        if !s.description.is_empty() {
            text.push_str(&format!("- {}\n", s.description));
        }
        for f in &s.prompt_fragments {
            text.push_str(&format!("- {f}\n"));
        }
        if !s.palette_notes.is_empty() {
            text.push_str(&format!("- Palette: {}\n", s.palette_notes));
        }
    }
    if let Some(m) = measured_style {
        text.push_str(&format!("- {m}\n"));
    }
    if !refs.is_empty() {
        text.push_str("\nReference images, in the order attached:\n");
        for (i, r) in refs.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", i + 1, r.role.phrase()));
        }
        if let Some(line) = form.prompt_line() {
            text.push_str(&format!("{line}\n"));
        }
    }
    let mut negative: Vec<String> = profile.imagegen.prompt.negative.clone();
    if let Some(s) = style {
        negative.extend(s.negative_rules.iter().cloned());
    }
    if !negative.is_empty() {
        text.push_str(&format!("\nAvoid: {}.\n", negative.join("; ")));
    }
    text.push_str("\nProduce exactly one image.\n");
    PromptPlan { sha256: sha256_hex(text.as_bytes()), text, negative }
}

/// Aspect ratio of a canvas reduced to lowest terms, as `w:h`.
pub fn aspect_of(canvas: [u32; 2]) -> String {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    let g = gcd(canvas[0].max(1), canvas[1].max(1));
    format!("{}:{}", canvas[0] / g, canvas[1] / g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileSet;
    use crate::refs::{RefOrigin, RefRole};

    fn plan() -> Plan {
        let set = ProfileSet::builtin();
        let prof = set.for_family(Family::Focus).unwrap();
        let prompt = build_prompt(prof, [100, 88], "an iron wall", None, &[], true, None);
        let mut p = Plan {
            plan_id: String::new(),
            plan_hash: String::new(),
            created: "2026-10-31T00:00:00Z".into(),
            request: PlanRequest {
                family: Family::Focus,
                feature_slug: "iron".into(),
                asset: "wall".into(),
                brief: "an iron wall".into(),
                style: None,
                references: vec![],
                provider: Some("openai".into()),
                model: None,
                transparent: None,
                variants: None,
                budget_usd: None,
                quality: None,
                canvas: None,
                texture_kind: None,
            },
            asset_id: "iron.wall".into(),
            profile: ProfileRef { id: "focus".into(), hash: "h".into(), kit_sha256: None },
            canvas: [100, 88],
            source_canvas: None,
            frames: 1,
            provider: "openai".into(),
            model: "gpt-image-2.5-flare".into(),
            quality: Some("low".into()),
            generation: GenerationPlan { width: 1024, height: 912, transparent: true, aspect: aspect_of([100, 88]) },
            prompt,
            references: vec![],
            style: None,
            style_sha256: None,
            style_sources: None,
            style_selection: None,
            estimate: PlanEstimate {
                usd_per_call: 0.006,
                calls: 1,
                usd: 0.006,
                basis: "test".into(),
                usage_unknown: false,
                funding: "user_key".into(),
            },
            paths: PlanPaths {
                candidate_dir: "docs/assets/iron/source_png".into(),
                processed_dir: "docs/assets/iron/processed_png".into(),
                preview_dir: "docs/assets/iron/contact_sheets".into(),
                prompts_dir: "docs/assets/iron/prompts".into(),
                final_files: vec!["gfx/interface/goals/iron/goal_wall.dds".into()],
                handoff: "docs/assets/iron/gfx_handoff.md".into(),
            },
            variants: 1,
            approvals: vec![],
            warnings: vec![],
        };
        p.seal();
        p
    }

    #[test]
    fn hash_is_stable_and_ignores_the_timestamp() {
        let a = plan();
        let mut b = plan();
        b.created = "2030-01-01T00:00:00Z".into();
        b.seal();
        assert_eq!(a.plan_hash, b.plan_hash);
        assert_eq!(a.plan_id, b.plan_id);
        assert!(valid_plan_id(&a.plan_id));
        assert!(a.is_intact());
    }

    #[test]
    fn any_edit_breaks_the_seal() {
        let mut p = plan();
        p.model = "other".into();
        assert!(!p.is_intact());
        let mut q = plan();
        q.prompt.text.push_str(" and also a dragon");
        assert!(!q.is_intact());
        let mut r = plan();
        r.estimate.usd = 0.0;
        assert!(!r.is_intact());
    }

    #[test]
    fn plan_ids_are_path_safe() {
        assert!(valid_plan_id("plan-0123456789abcdef"));
        for bad in ["plan-../../x", "plan-0123456789ABCDEF", "plan-0123", "x", "plan-0123456789abcdeg"] {
            assert!(!valid_plan_id(bad), "{bad}");
        }
    }

    #[test]
    fn prompt_includes_rules_style_references_and_negatives() {
        let set = ProfileSet::builtin();
        let prof = set.for_family(Family::Focus).unwrap();
        let style = StyleProfile {
            schema: 1,
            name: "oil".into(),
            description: "Aged oil painting.".into(),
            prompt_fragments: vec!["Visible brush strokes.".into()],
            negative_rules: vec!["neon colours".into()],
            palette_notes: "earth tones".into(),
            families: vec![],
            default: None,
            references: vec![],
            provider: None,
            model: None,
            seed_policy: None,
            approved_by: None,
            approved_on: None,
        };
        let r = LoadedRef {
            path: "references/a.png".into(),
            role: RefRole::StyleAnchor,
            origin: RefOrigin::Project,
            source_sha256: "s".into(),
            source_format: "png".into(),
            sha256: "x".into(),
            width: 1,
            height: 1,
            notes: vec![],
            png: vec![],
        };
        let p = build_prompt(prof, [100, 88], "a steel gate", Some(&style), &[r], true, Some("Match the mod's focus art: muted palette."));
        assert!(p.text.contains("national focus icon: a steel gate"));
        assert!(p.text.contains("100x88"));
        assert!(p.text.contains("Aged oil painting.") && p.text.contains("Visible brush strokes.") && p.text.contains("earth tones"));
        assert!(p.text.contains("1. style anchor"));
        assert!(p.text.contains("- Palette: earth tones\n- Match the mod's focus art: muted palette.\n"));
        assert!(p.text.contains("neon colours") && p.text.contains("a drawn checkerboard pattern"));
        assert_eq!(p.sha256, sha256_hex(p.text.as_bytes()));
        assert_eq!(
            p,
            build_prompt(
                prof,
                [100, 88],
                "a steel gate",
                Some(&style),
                &[LoadedRef {
                    path: "references/a.png".into(),
                    role: RefRole::StyleAnchor,
                    origin: RefOrigin::Project,
                    source_sha256: "s".into(),
                    source_format: "png".into(),
                    sha256: "x".into(),
                    width: 1,
                    height: 1,
                    notes: vec![],
                    png: vec![]
                }],
                true,
                Some("Match the mod's focus art: muted palette.")
            )
        );
    }

    #[test]
    fn aspect_is_reduced() {
        assert_eq!(aspect_of([100, 88]), "25:22");
        assert_eq!(aspect_of([397, 153]), "397:153");
        assert_eq!(aspect_of([64, 64]), "1:1");
    }
}
