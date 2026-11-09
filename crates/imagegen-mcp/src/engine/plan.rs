//! `hoi4.image_plan`: a dry run that costs nothing and fixes everything a generation will use.

use imagegen_core::error::RetryClass;
use imagegen_core::export;
use imagegen_core::family::{Family, Group};
use imagegen_core::naming::{self, asset_id};
use imagegen_core::pack;
use imagegen_core::plan::{
    aspect_of, build_prompt_with_look, plan_path, FormPrecedence, GenerationPlan, PackReference, PackReport, Plan, PlanEstimate, PlanPaths,
    PlanRequest, StyleContribution, StyleMode, StyleSelection,
};
use imagegen_core::records::{kind, RefSummary};
use imagegen_core::refs::{self, LoadedRef, RefRole, ReferenceInput};
use imagegen_core::reuse;
use imagegen_core::style;
use imagegen_core::style::{StyleProfile, StyleWhy};
use imagegen_core::stylesrc;
use imagegen_providers::types::{AlphaSupport, ImageRequest, ModelStatus};
use serde_json::json;

use crate::api::{GenerateParams, GroupFollowUp};

use super::{EResult, Engine, EngineError};

/// Default provider order when a plan names none (decision D-12a and the maintainer's answer of 2026-11-01).
pub const PROVIDER_ORDER: [&str; 5] = ["chatgpt_login", "openai", "gemini", "fal", "comfyui"];

impl Engine {
    /// Resolve and store a plan.
    pub fn plan(&self, req: PlanRequest) -> EResult<Plan> {
        export::validate_asset_name(req.family, &req.asset)?;
        naming::validate_snake("feature slug", &req.feature_slug)?;
        let brief = req.brief.trim().to_string();
        if brief.is_empty() || brief.chars().count() > 4000 || brief.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
            return Err(EngineError::invalid("brief must be 1 to 4000 characters of plain text"));
        }
        let base_profile = self
            .profiles
            .for_family(req.family)
            .ok_or_else(|| EngineError::new("not_found", format!("no profile for {}", req.family), RetryClass::Terminal))?
            .clone();
        // the style pack: the named one, else the project default. Its entry for the family (narrowed to the texture
        // kind) sets the canvas, the processing switches, the references, the look line and the form override; a
        // family it does not cover keeps the normal tiers
        let pack_choice = self.choose_pack(req.style_pack.as_deref())?;
        let pack_entry = pack_choice.as_ref().and_then(|c| c.pack.families.get(&req.family));
        let pack_resolved = pack_choice.as_ref().and_then(|c| c.pack.resolve(req.family, req.texture_kind));
        let profile = match (&pack_choice, pack_entry) {
            (Some(c), Some(e)) => pack::apply_profile(&base_profile, &c.pack.name, e),
            _ => base_profile,
        };
        // a model reference's backdrop: grey (the profile as it is) or real alpha, which changes the prompt rule,
        // the recipe and the checks. Recorded on the profile ref, so processing and validation apply it again
        if req.transparent.is_some() && req.family == Family::ModelReference {
            return Err(EngineError::invalid("a model reference's background is set with backdrop (grey or transparent), not transparent"));
        }
        let profile = profile.with_backdrop_opt(req.backdrop)?;
        if let (Some(c), Some(r)) = (&pack_choice, &pack_resolved) {
            pack::verify_references(&self.fs, &c.pack.name, &r.references)?;
        }

        let canvas = match (profile.canvas, req.canvas) {
            (_, Some(c)) if profile.free_canvas() && (1..=4096).contains(&c[0]) && (1..=4096).contains(&c[1]) => c,
            (_, Some(_)) if profile.free_canvas() => return Err(EngineError::invalid("canvas sides must be 1 to 4096")),
            (_, Some(c)) if profile.canvas_choices().contains(&c) => c,
            ([0, 0], None) => return Err(EngineError::invalid(format!("{} has no fixed canvas: pass canvas [width, height]", req.family))),
            (p, None) => p,
            (p, Some(c)) if c == p => p,
            (p, Some(_)) => {
                return Err(EngineError::invalid(format!(
                    "{} has a fixed canvas of {}x{}; to use another, override the profile in .hoi4-imagegen/config.toml or the kit's profiles.json{}",
                    req.family,
                    p[0],
                    p[1],
                    if profile.canvas_choices().is_empty() {
                        String::new()
                    } else {
                        format!(
                            " (or pick one of {})",
                            profile.canvas_choices().iter().map(|c| format!("{}x{}", c[0], c[1])).collect::<Vec<_>>().join(", ")
                        )
                    }
                )))
            }
        };

        // only a GUI texture has kinds
        if req.texture_kind.is_some() && req.family != Family::GuiTexture {
            return Err(EngineError::invalid(format!("texture_kind is for gui_texture, not {}", req.family)));
        }

        let style_profile = match &req.style {
            Some(name) => {
                let s = style::load(&self.fs, name)?;
                if !s.applies_to(req.family) {
                    let listed: Vec<&str> = s.families.iter().map(|f| f.as_str()).collect();
                    return Err(EngineError::invalid(format!(
                        "style '{name}' lists only these families: {}. {} is not one of them. Add it to the style's families, or empty the list to make the style available to any family",
                        listed.join(", "),
                        req.family
                    )));
                }
                s.verify_references(&self.fs)?;
                Some(s)
            }
            None => None,
        };

        // provider: named, the style's, the project default, then the first ready one in the fixed order (a
        // signed-in ChatGPT login first, decision D-12a). The choice is reported in the plan.
        let provider_id = req
            .provider
            .clone()
            .or_else(|| style_profile.as_ref().and_then(|s| s.provider.clone()))
            .or_else(|| self.config.defaults.provider.clone())
            .or_else(|| {
                let configured = self.registry.configured_ids();
                PROVIDER_ORDER.iter().find(|id| configured.iter().any(|c| c == *id)).map(|s| s.to_string())
            })
            .ok_or_else(|| {
                EngineError::invalid(
                    "no image provider is ready: run `hoi4-imagegen login` to sign in with your ChatGPT account, or set an API key (OPENAI_API_KEY, GEMINI_API_KEY, FAL_KEY) or a ComfyUI endpoint",
                )
            })?;
        self.check_family_policy(&profile, &provider_id)?;
        let provider = self.registry.get(&provider_id).ok_or_else(|| {
            let why =
                self.registry.problems().get(&provider_id).cloned().unwrap_or_else(|| format!("known providers: {}", self.registry.ids().join(", ")));
            EngineError::new("provider_not_found", format!("provider '{provider_id}' is not available ({why})"), RetryClass::Terminal)
        })?;

        // references, mod first (maintainer decision of 2026-11-01): the pinned style, the mod's copy of the asset
        // skill's shelf, the mod's own assets, then vanilla. The tier is chosen from the files alone, so the
        // provider's limit (which depends on the model) only trims the list afterwards.
        let game = self.game_dir();
        let applied = self.applied_style_for(req.family, style_profile.as_ref());
        let pinned = applied.as_ref().map(|(s, _)| s.clone());
        // the sub-family: named in the request, else inferred from the brief's words
        let (subfamily, sub_how): (Option<&str>, &str) = match req.subfamily.as_deref() {
            Some(id) => {
                let fam = imagegen_core::subfamily::for_family(req.family)
                    .ok_or_else(|| EngineError::invalid(format!("{} has no sub-families; leave subfamily out", req.family)))?;
                let known = fam
                    .get(id)
                    .ok_or_else(|| EngineError::invalid(format!("{} has no sub-family '{id}'; it has {}", req.family, fam.ids().join(", "))))?;
                (Some(known.id.as_str()), "named")
            }
            None => (imagegen_core::subfamily::infer(req.family, &brief, &req.asset), "inferred"),
        };
        let request = stylesrc::RequestKey { asset: &req.asset, brief: &brief, subfamily };
        let mut sources = self.resolve_style(&profile, pinned.as_ref(), stylesrc::DEFAULT_SEND, false, req.texture_kind, Some(request));
        let explicit_model = req.model.clone().or_else(|| style_profile.as_ref().and_then(|s| s.model.clone()));
        let explicit_refs = req.references.len();
        // the caller's own style anchors show the form to match, and vanilla art beside them pulls the image back to
        // the vanilla form. They also win over a style pack's references
        let caller_style = req.references.iter().any(|r| r.role == RefRole::StyleAnchor);
        let pack_refs: Vec<imagegen_core::style::StyleRef> =
            if caller_style { Vec::new() } else { pack_resolved.as_ref().map(|r| r.references.clone()).unwrap_or_default() };
        let model_class = imagegen_core::ModelClass::of(req.family, canvas);
        let wants_refs = explicit_refs > 0 || !pack_refs.is_empty() || !sources.send_paths().is_empty();
        let mut model_id = explicit_model.clone().unwrap_or_else(|| provider.default_model_for(model_class, wants_refs));
        let model_refs = |m: &str| provider.descriptor().model(m).map(|x| if x.supports_edit { usize::from(x.max_refs) } else { 0 }).unwrap_or(0);
        let pack_cap = model_refs(&model_id).min(refs::MAX_REFERENCES).saturating_sub(explicit_refs).min(pack::MAX_SEND);
        let pack_sent: Vec<imagegen_core::style::StyleRef> = pack_refs.iter().take(pack_cap).cloned().collect();
        let cap = model_refs(&model_id).saturating_sub(explicit_refs + pack_sent.len()).min(stylesrc::DEFAULT_SEND);
        sources.limit_send(cap, &format!("{provider_id} {model_id} takes at most {} reference images", model_refs(&model_id)));
        // the caller's own style anchors, or a style pack's references in their place, show the form to match, and
        // vanilla or shelf art beside them pulls the image back to that art's frame. With style_mode replace (the
        // default) only the anchors are sent: every automatic tier is left out, and a pinned style too unless the
        // request named it. style_mode add sends the automatic tiers beside them
        let style_mode = req.style_mode.unwrap_or_default();
        let anchors = caller_style || !pack_sent.is_empty();
        if anchors && style_mode == StyleMode::Replace {
            let why = match (&pack_choice, &pack_resolved) {
                (Some(c), Some(r)) if !caller_style => {
                    format!("the style pack '{}' sends its own references for {} (style_mode replace)", c.pack.name, r.entry)
                }
                _ => "the request sends its own style anchors (style_mode replace)".to_string(),
            };
            for tier in 2..=4 {
                sources.drop_tier(tier, &why);
            }
            if req.style.is_none() {
                sources.drop_tier(1, &format!("{why} and names no style"));
            }
            for t in sources.report.tiers.iter_mut().filter(|t| t.scanned && (t.tier > 1 || req.style.is_none())) {
                t.note = format!("{}; nothing sent: {why}", t.note);
            }
        } else if !anchors {
            if let (Some(c), Some(r)) = (&pack_choice, &pack_resolved) {
                if !r.form.is_empty() {
                    sources.drop_tier(4, &format!("the style pack '{}' replaces the vanilla form of {}", c.pack.name, r.entry));
                }
            }
        }
        if explicit_model.is_none() && explicit_refs == 0 && pack_sent.is_empty() && sources.send_paths().is_empty() {
            model_id = provider.default_model_for(model_class, false);
        }

        let mut loaded: Vec<LoadedRef> = Vec::new();
        // the pack's references go first; a pinned file that cannot be read fails the plan, as a caller's would
        for r in &pack_sent {
            loaded.push(refs::load_reference(&self.fs, game.as_deref(), &ReferenceInput { path: r.path.clone(), role: RefRole::StyleAnchor })?);
        }
        let mut inputs: Vec<ReferenceInput> =
            sources.send_paths().into_iter().map(|path| ReferenceInput { path, role: RefRole::StyleAnchor }).collect();
        inputs.extend(req.references.iter().cloned());
        for input in &inputs {
            let is_auto = !req.references.contains(input);
            match refs::load_reference(&self.fs, game.as_deref(), input) {
                Ok(l) => {
                    if is_auto {
                        if let Some(c) = sources.report.chosen.iter_mut().find(|c| c.sent_to_provider && c.path == input.path) {
                            c.sha256 = l.sha256.clone();
                        }
                    }
                    loaded.push(l);
                }
                Err(e) if is_auto => {
                    // an automatic reference that cannot be read is dropped; only the caller's own references fail a plan
                    if let Some(c) = sources.report.chosen.iter_mut().find(|c| c.sent_to_provider && c.path == input.path) {
                        c.sent_to_provider = false;
                        c.reason = format!("{} (not sent: {e})", c.reason);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        refs::check_set_caps(&loaded)?;
        // a reference that is the candidate itself must not be another family's installed art
        for r in loaded.iter().filter(|r| r.role == RefRole::EditTarget) {
            let (img, _) = imagegen_core::pixels::decode(&r.png, None, imagegen_core::pixels::DecodeLimits::default())?;
            reuse::ensure_no_cross_family_reuse(
                &self.fs,
                &self.profiles,
                req.family,
                &reuse::pixel_hash(&img),
                &format!("the edit target {}", r.path),
            )?;
        }

        if provider_id == "chatgpt_login" && model_id != provider_chatgpt_login::MODEL {
            return Err(EngineError::invalid(format!(
                "the ChatGPT-login route runs only {} at {} (maintainer decision); '{model_id}' cannot be used with it",
                provider_chatgpt_login::MODEL,
                provider_chatgpt_login::EFFORT_POLICY
            )));
        }
        let quality = req.quality.clone().or_else(|| self.config.provider(&provider_id).quality);
        let transparent = req.transparent.unwrap_or(profile.source_transparent());
        // what the model is asked for: the canvas, or a different shape when the recipe needs one (a tall portrait for the advisor card)
        // a canvas wider than the models' 3:1 limit (the 285x56 technology picture) is drawn at 3:1 and fitted by the chain
        let too_wide = (canvas[0] > 3 * canvas[1]).then_some([3 * canvas[1], canvas[1]]);
        let source_canvas = profile.imagegen.source_canvas.or(too_wide).filter(|c| *c != canvas);
        let ask = source_canvas.unwrap_or(canvas);
        // finishing that needs the mod's templates is refused now, before anything is spent
        self.check_finishing_resources(imagegen_core::chain::recipe_of(&profile))?;
        let prompt_style = style_profile.as_ref().or(pinned.as_ref());
        // the mod's own references (tier 2 or 3) show the mod's form: it wins over the vanilla form the rules state
        let mod_form_refs = sources.report.chosen.iter().any(|c| c.sent_to_provider && (c.tier == 2 || c.tier == 3));
        let form = if caller_style || !pack_sent.is_empty() {
            FormPrecedence::CallerReferences
        } else if mod_form_refs {
            FormPrecedence::ModReferences
        } else {
            FormPrecedence::VanillaRule
        };
        // the measured paragraph describes the tier's images, which are not sent beside the caller's anchors when vanilla
        // it stays when nothing is attached at all (a model that takes no references), and goes when the request sends
        // references and none of them is automatic
        let automatic_sent = sources.report.chosen.iter().any(|c| c.sent_to_provider);
        let measured = if !loaded.is_empty() && !automatic_sent { None } else { sources.report.style_paragraph.as_deref() };
        // a GUI texture's kind adds the kind's vanilla form to the prompt (the profile itself, and its hash, stay as
        // they are). A style pack's form and negatives replace the vanilla ones, the kind's form included
        let mut prompt_profile = profile.clone();
        let pack_form = pack_resolved.as_ref().is_some_and(|r| !r.form.is_empty());
        if let Some(r) = &pack_resolved {
            let (rules, negative) = pack::prompt_rules(&profile, r);
            prompt_profile.imagegen.prompt.rules = rules;
            prompt_profile.imagegen.prompt.negative = negative;
        }
        if let Some(kind) = req.texture_kind {
            if !pack_form {
                let at = 1.min(prompt_profile.imagegen.prompt.rules.len());
                prompt_profile.imagegen.prompt.rules.insert(at, kind.form_rule().to_string());
            }
        }
        // the sub-family's measured form note follows the family's form sentence, except beside anchors (the caller's
        // or a pack's) and where a pack's form replaces the family's
        let sub_note = subfamily
            .and_then(|id| imagegen_core::subfamily::for_family(req.family)?.get(id)?.form_note.clone())
            .filter(|_| form != FormPrecedence::CallerReferences && !pack_form);
        if let Some(note) = &sub_note {
            let at = prompt_profile.imagegen.prompt.rules.len().min(1 + usize::from(req.texture_kind.is_some()));
            prompt_profile.imagegen.prompt.rules.insert(at, note.clone());
        }
        let subfamily_choice = subfamily.map(|id| {
            let used = sources.report.subfamily.as_ref().is_some_and(|u| u.used) && sources.report.chosen.iter().any(|c| c.sent_to_provider);
            let note = match (sub_how, sources.report.subfamily.as_ref()) {
                _ if !pack_sent.is_empty() && !used => {
                    format!("the sub-family {id} is recorded, and the style pack decides the references")
                }
                (_, Some(u)) => {
                    format!("{} sub-family {id}: {}", if sub_how == "named" { "the request names the" } else { "the brief points to the" }, u.note)
                }
                _ => format!("the sub-family {id} applies to the form note only: no automatic reference tier was used"),
            };
            imagegen_core::plan::SubfamilyChoice {
                id: id.to_string(),
                how: sub_how.to_string(),
                references_from_subfamily: used,
                form_note: sub_note.clone(),
                note,
            }
        });
        let sub_warning =
            (req.subfamily.is_none() && subfamily.is_none() && pack_sent.is_empty() && imagegen_core::subfamily::for_family(req.family).is_some())
                .then(|| {
                    format!(
                        "the brief points to no sub-family of {}, so references come from the whole family (name one with subfamily: {})",
                        req.family,
                        imagegen_core::subfamily::for_family(req.family).map(|f| f.ids().join(", ")).unwrap_or_default()
                    )
                });
        // references the tool chose itself (tiers 2 to 4) show the style only; anchors (the caller's or a pack's) are matched
        let automatic_refs = !anchors && sources.report.chosen.iter().any(|c| c.sent_to_provider && c.tier >= 2);
        let look = pack_resolved.as_ref().and_then(|r| r.look.as_deref());
        let prompt = build_prompt_with_look(&prompt_profile, ask, &brief, prompt_style, &loaded, transparent, measured, form, automatic_refs, look);
        let mut style_selection = describe_selection(applied.as_ref(), &sources.report, form, req.family);
        style_selection.style_mode = anchors.then_some(style_mode);
        let pack_report = pack_choice.as_ref().map(|c| {
            let sent: Vec<&str> = pack_sent.iter().map(|r| r.path.as_str()).collect();
            let all = pack_resolved.as_ref().map(|r| r.references.clone()).unwrap_or_default();
            let references: Vec<PackReference> =
                all.iter().map(|r| PackReference { path: r.path.clone(), sha256: r.sha256.clone(), sent_to_provider: sent.contains(&r.path.as_str()) }).collect();
            let why = if c.why == "explicit" { "named in the request" } else { "the project's default ([defaults] style_pack)" };
            let detail = match &pack_resolved {
                None => format!("style pack '{}' ({why}) has no entry for {}, so the normal style tiers apply", c.pack.name, req.family),
                Some(r) if caller_style && !r.references.is_empty() => format!(
                    "style pack '{}' ({why}) applies its entry {}: the request's own style anchors win, so the pack's references are not sent; its look, form and processing still apply",
                    c.pack.name, r.entry
                ),
                Some(r) => format!(
                    "style pack '{}' ({why}) applies its entry {}: {} of its {} references are sent as style anchors{}{}",
                    c.pack.name,
                    r.entry,
                    pack_sent.len(),
                    r.references.len(),
                    if r.look.is_some() { ", its look line is added to the style" } else { "" },
                    if r.form.is_empty() { "" } else { ", and its form rules replace the vanilla form rules" }
                ),
            };
            PackReport {
                name: c.pack.name.clone(),
                sha256: c.sha256.clone(),
                why: c.why.to_string(),
                covered: pack_resolved.is_some(),
                entry: pack_resolved.as_ref().map(|r| r.entry.clone()),
                look: pack_resolved.as_ref().and_then(|r| r.look.clone()),
                form_override: pack_form,
                negative_override: pack_resolved.as_ref().is_some_and(|r| !r.negative.is_empty()),
                canvas: pack_entry.and_then(|e| e.canvas),
                processing: profile.processing().names().into_iter().map(str::to_string).collect(),
                references,
                detail,
            }
        });
        if let (Some(c), Some(r)) = (&pack_choice, &pack_resolved) {
            if !pack_sent.is_empty() {
                style_selection.contributions.insert(
                    0,
                    StyleContribution {
                        source: format!("style pack '{}' ({})", c.pack.name, r.entry),
                        tier: 0,
                        references: pack_sent.iter().map(|x| x.path.clone()).collect(),
                    },
                );
                style_selection.form_note = format!(
                    "the style pack '{}' sends its references for {} as style anchors: the prompt says their form and framing win{}, {}",
                    c.pack.name,
                    r.entry,
                    if r.form.is_empty() { " over the vanilla form rule" } else { ", and the pack's form rules replace the vanilla ones" },
                    if style_mode == StyleMode::Add {
                        "and style_mode add sends the automatic references beside them"
                    } else {
                        "and no automatic reference is sent beside them"
                    }
                );
            }
        }

        let probe = ImageRequest {
            model: model_id.clone(),
            prompt: prompt.text.clone(),
            negative: prompt.negative.clone(),
            width: ask[0],
            height: ask[1],
            transparent,
            quality: quality.clone(),
            refs: loaded
                .iter()
                .map(|r| imagegen_providers::types::ProviderRef { role: r.role, png: r.png.clone(), sha256: r.sha256.clone() })
                .collect(),
            edit: None,
            idempotency_key: None,
            seed: None,
        };
        let (gw, gh) = provider.prepare(&probe)?;
        let est = provider.estimate(&probe)?;
        let variants = req.variants.unwrap_or(1);
        if !(1..=4).contains(&variants) {
            return Err(EngineError::invalid("variants must be 1 to 4"));
        }
        let total = (est.usd * f64::from(variants) * 1e6).round() / 1e6;
        if let Some(budget) = req.budget_usd {
            if total > budget {
                return Err(EngineError::refused(
                    "budget_exceeded",
                    format!("the estimate is ${total:.4} for {variants} call(s) ({}), above the budget of ${budget:.4}", est.basis),
                    RetryClass::RequiresAuthorization,
                ));
            }
        }

        // notes the caller should read
        let mut warnings: Vec<String> = sub_warning.into_iter().collect();
        let mut approvals: Vec<String> = Vec::new();
        if let Some(model) = provider.descriptor().model(&model_id) {
            if let ModelStatus::Deprecated { shutdown } = &model.status {
                warnings.push(format!("model {model_id} is deprecated and shuts down on {shutdown}"));
            }
            // the unit art and line art recipes read the subject from a flat background themselves, so no alpha is
            // needed there
            let keys_itself = matches!(
                imagegen_core::chain::recipe_of(&profile),
                imagegen_core::chain::Recipe::UnitCounter | imagegen_core::chain::Recipe::DivisionSymbol | imagegen_core::chain::Recipe::LineArt
            );
            if transparent && model.alpha == AlphaSupport::None && !keys_itself {
                warnings.push(format!(
                    "{model_id} has no documented native transparency. A prompt cannot prove it: the result is validated from pixels and an opaque result is rejected. Use a model with native alpha or a matte workflow if this icon must be transparent"
                ));
            }
            if transparent && model.alpha == AlphaSupport::Preview && !keys_itself {
                warnings.push(format!("{model_id} documents transparent backgrounds as a preview; the pixels are checked either way"));
            }
        }
        if profile.family.group() == Group::Portrait && profile.family != Family::AdvisorCard {
            warnings.push(
                "portraits of real people: providers may refuse or treat images of real people differently, and the Steam warranty covers personal rights. Prefer stylised portraits and never edit photographs of identifiable living people (docs/rules-and-terms.md)".into(),
            );
            if !self.face_model_available() {
                warnings.push(
                    "the YuNet face model was not found in the project (.claude/skills/*/tools/models/), so processing needs portrait_crop to choose the crop by hand".into(),
                );
            }
        }
        if profile.family == Family::AdvisorCard {
            warnings.push("advisor portraits of real people: the same likeness caution applies (docs/rules-and-terms.md)".into());
        }
        if profile.source.starts_with("LOW CONFIDENCE") {
            warnings.push("this profile's canvas has low confidence; confirm the surface before installing".into());
        }
        if profile.differs_from_vanilla() {
            warnings.push(format!(
                "the canvas {}x{} comes from {:?} and differs from the vanilla-measured default {}x{}",
                profile.canvas[0],
                profile.canvas[1],
                profile.origin,
                profile.vanilla.as_ref().map(|v| v.canvas[0]).unwrap_or(0),
                profile.vanilla.as_ref().map(|v| v.canvas[1]).unwrap_or(0)
            ));
        }
        if !provider.is_configured() {
            approvals.push(format!(
                "provider {provider_id} has no credentials: set {} in your environment before generating",
                imagegen_providers::registry::key_env_names(&provider_id, &self.config.provider(&provider_id)).join(" or ")
            ));
        }
        if req.style.is_none() {
            if let Some((p, why)) = &applied {
                let because = match why {
                    StyleWhy::Default => "it is marked default".to_string(),
                    _ => format!("its families list names {}", req.family),
                };
                warnings.push(format!(
                    "the project's pinned style '{}' for {} was applied automatically because {because} (references and style text)",
                    p.name, req.family
                ));
            }
        }
        if let Some(rep) = &pack_report {
            warnings.push(rep.detail.clone());
            if pack_cap < pack_refs.len() {
                warnings.push(format!(
                    "{provider_id} {model_id} takes {} reference images in all here, so only the first {} of the pack's {} references are sent",
                    model_refs(&model_id),
                    pack_cap,
                    pack_refs.len()
                ));
            }
        }
        if est.usd > self.config.policy.max_usd_per_call {
            approvals.push(format!(
                "one call is estimated at ${:.4}, above the project's per-call limit of ${:.2}: pass approve_cost to hoi4.image_generate",
                est.usd, self.config.policy.max_usd_per_call
            ));
        }
        if est.usd > 0.0 {
            approvals.push(format!("spend: about ${total:.4} in total on {provider_id} ({}), funded by {}", est.basis, est.funding.as_str()));
        }

        let finals = export::final_files(&profile, canvas, &req.feature_slug, &req.asset)?;
        let slug = &req.feature_slug;
        let paths = PlanPaths {
            candidate_dir: format!("docs/assets/{slug}/source_png"),
            processed_dir: format!("docs/assets/{slug}/processed_png"),
            preview_dir: format!("docs/assets/{slug}/contact_sheets"),
            prompts_dir: format!("docs/assets/{slug}/prompts"),
            final_files: finals.iter().map(|f| f.rel.clone()).collect(),
            handoff: format!("docs/assets/{slug}/gfx_handoff.md"),
        };

        let mut normalized = req.clone();
        normalized.brief = brief;
        normalized.provider = Some(provider_id.clone());
        normalized.model = Some(model_id.clone());
        normalized.canvas = profile.canvas_overridden(canvas).then_some(canvas);
        normalized.transparent = Some(transparent);
        normalized.variants = Some(variants);
        normalized.quality = quality.clone();

        let mut plan = Plan {
            plan_id: String::new(),
            plan_hash: String::new(),
            created: self.now(),
            request: normalized,
            asset_id: asset_id(&req.feature_slug, &req.asset),
            profile: self.profile_ref(&profile),
            canvas,
            source_canvas,
            frames: profile.frames.max(1),
            provider: provider_id,
            model: model_id,
            quality,
            generation: GenerationPlan { width: gw, height: gh, transparent, aspect: aspect_of(ask) },
            prompt,
            references: loaded
                .iter()
                .map(|r| RefSummary { path: r.path.clone(), role: r.role, origin: r.origin, sha256: r.sha256.clone() })
                .collect(),
            style: req.style.clone(),
            style_sources: Some(sources.report.clone()),
            style_selection: Some(style_selection),
            subfamily: subfamily_choice,
            style_pack: pack_report,
            style_sha256: match &req.style {
                Some(n) => Some(imagegen_core::hashing::sha256_hex(&self.fs.read(&style::style_path(n)?, 256 * 1024)?)),
                None => None,
            },
            estimate: PlanEstimate {
                usd_per_call: est.usd,
                calls: u32::from(variants),
                usd: total,
                basis: est.basis.clone(),
                usage_unknown: est.usage_unknown,
                funding: est.funding.as_str().to_string(),
            },
            paths,
            variants,
            approvals,
            warnings,
        };
        plan.seal();
        self.store_plan(&plan)?;
        Ok(plan)
    }

    /// The group tools (`hoi4.image_icon` and the others): plan, and generate when `approve` is set.
    ///
    /// The family must belong to the tool's group. This is checked here as well as in the tool's schema, so a
    /// caller that ignores the schema still cannot make an asset through the wrong tool.
    pub fn group_request(&self, group: Group, req: PlanRequest, follow: GroupFollowUp) -> EResult<serde_json::Value> {
        if req.family.group() != group {
            return Err(EngineError::new(
                "family_not_in_group",
                format!(
                    "{} is a {} family and cannot be made with {}. Use {} (this tool makes: {})",
                    req.family,
                    req.family.group(),
                    group.tool_name(),
                    req.family.group().tool_name(),
                    group.family_ids().join(", ")
                ),
                RetryClass::Terminal,
            ));
        }
        let plan = self.plan(req)?;
        let plan_json = serde_json::to_value(&plan).map_err(|e| EngineError::new("internal", e.to_string(), RetryClass::Terminal))?;
        if !follow.approve {
            return Ok(json!({
                "generated": false,
                "plan_id": plan.plan_id,
                "plan": plan_json,
                "next": "Nothing was spent. Call this tool again with approve = true and an idempotency_key to generate, or pass the plan_id to hoi4.image_generate."
            }));
        }
        let key = follow
            .idempotency_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| EngineError::invalid("approve = true needs an idempotency_key"))?;
        let generation =
            self.generate(GenerateParams { plan_id: plan.plan_id.clone(), idempotency_key: key, approve_cost: Some(follow.approve_cost) })?;
        Ok(json!({"generated": true, "plan_id": plan.plan_id, "plan": plan_json, "generation": generation}))
    }

    fn store_plan(&self, plan: &Plan) -> EResult<()> {
        let rel = plan_path(&plan.plan_id);
        if self.fs.exists(&rel) {
            return Ok(());
        }
        let mut body = serde_json::to_vec_pretty(plan).map_err(|e| EngineError::new("internal", e.to_string(), RetryClass::Terminal))?;
        body.push(b'\n');
        match self.fs.write_new(&rel, &body) {
            Ok(_) => {
                self.ledger.append(kind::PLAN, Some(&plan.asset_id), None, json!({"plan_id": plan.plan_id, "plan_hash": plan.plan_hash, "provider": plan.provider, "model": plan.model, "estimate_usd": plan.estimate.usd}))?;
                Ok(())
            }
            Err(imagegen_core::Error::Exists(_)) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Load a stored plan and check its seal.
    pub(crate) fn load_plan(&self, plan_id: &str) -> EResult<Plan> {
        if !imagegen_core::plan::valid_plan_id(plan_id) {
            return Err(EngineError::invalid(format!("'{plan_id}' is not a plan id (plan- and 16 hex digits)")));
        }
        let bytes = self.fs.read(&plan_path(plan_id), 4 * 1024 * 1024).map_err(|e| match e {
            imagegen_core::Error::NotFound(_) => {
                EngineError::new("not_found", format!("no stored plan {plan_id}; run hoi4.image_plan first"), RetryClass::Terminal)
            }
            other => other.into(),
        })?;
        let plan: Plan = serde_json::from_slice(&bytes)
            .map_err(|e| EngineError::new("plan_unreadable", format!("plan {plan_id} is not readable: {e}"), RetryClass::Terminal))?;
        if !plan.is_intact() || plan.plan_id != plan_id {
            return Err(EngineError::refused(
                "plan_changed",
                format!("plan {plan_id} was edited after it was made (its hash no longer matches). Make a new plan"),
                RetryClass::RefreshPreconditions,
            ));
        }
        Ok(plan)
    }
}

/// What the plan says about the style it applied, what each source contributed and which form wins.
fn describe_selection(
    applied: Option<&(StyleProfile, StyleWhy)>,
    report: &stylesrc::StyleReport,
    form: FormPrecedence,
    family: Family,
) -> StyleSelection {
    let (name, why) = match applied {
        Some((s, why)) => (Some(s.name.clone()), *why),
        None => (None, StyleWhy::None),
    };
    let detail = match (&name, why) {
        (Some(n), StyleWhy::Explicit) => format!("style '{n}' was named in the request"),
        (Some(n), StyleWhy::Default) => format!("style '{n}' is marked default, so it applies to {family} without being named"),
        (Some(n), _) => format!("style '{n}' lists {family} in its families, so it applies without being named"),
        (None, _) => "no style profile applies: none was named, and no profile is marked default or lists this family".to_string(),
    };
    let mut contributions: Vec<StyleContribution> = Vec::new();
    for c in report.chosen.iter().filter(|c| c.sent_to_provider) {
        let source = match (&name, c.tier) {
            (Some(n), 1) => format!("style '{n}'"),
            _ => stylesrc::Tier::ALL.iter().find(|t| t.number() == c.tier).map(|t| t.label().to_string()).unwrap_or_default(),
        };
        match contributions.iter_mut().find(|x| x.tier == c.tier) {
            Some(x) => x.references.push(c.path.clone()),
            None => contributions.push(StyleContribution { source, tier: c.tier, references: vec![c.path.clone()] }),
        }
    }
    let form_note = match form {
        FormPrecedence::ModReferences => {
            "the style references come from the mod itself (tier 2 or 3): the prompt says their form and framing win over the vanilla form rule"
                .to_string()
        }
        FormPrecedence::CallerReferences => {
            "the request sends its own style anchors: the prompt takes the form and framing from them in place of the family's form sentence, and with style_mode replace (the default) no automatic reference is sent beside them"
                .to_string()
        }
        FormPrecedence::VanillaRule => "no mod reference (tier 2 or 3) is sent, so the family's vanilla form rule stands alone".to_string(),
    };
    StyleSelection { applied: name, why: why.as_str().to_string(), detail, contributions, form_precedence: form, form_note, style_mode: None }
}

/// Convenience for tests and the CLI: a family by id string.
pub fn parse_family(s: &str) -> EResult<Family> {
    s.parse::<Family>().map_err(EngineError::from)
}
