//! `hoi4.image_plan`: a dry run that costs nothing and fixes everything a generation will use.

use imagegen_core::error::RetryClass;
use imagegen_core::export;
use imagegen_core::family::{Family, Group};
use imagegen_core::naming::{self, asset_id};
use imagegen_core::plan::{aspect_of, build_prompt, plan_path, GenerationPlan, Plan, PlanEstimate, PlanPaths, PlanRequest};
use imagegen_core::records::{kind, RefSummary};
use imagegen_core::refs::{self, LoadedRef, RefRole, ReferenceInput};
use imagegen_core::reuse;
use imagegen_core::style;
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
        let profile = self
            .profiles
            .for_family(req.family)
            .ok_or_else(|| EngineError::new("not_found", format!("no profile for {}", req.family), RetryClass::Terminal))?
            .clone();

        let canvas = match (profile.canvas, req.canvas) {
            (_, Some(c)) if profile.free_canvas() && (1..=4096).contains(&c[0]) && (1..=4096).contains(&c[1]) => c,
            (_, Some(_)) if profile.free_canvas() => return Err(EngineError::invalid("canvas sides must be 1 to 4096")),
            ([0, 0], None) => return Err(EngineError::invalid(format!("{} has no fixed canvas: pass canvas [width, height]", req.family))),
            (p, None) => p,
            (p, Some(c)) if c == p => p,
            (p, Some(_)) => {
                return Err(EngineError::invalid(format!(
                    "{} has a fixed canvas of {}x{}; to use another, override the profile in .hoi4-imagegen/config.toml or the kit's profiles.json",
                    req.family, p[0], p[1]
                )))
            }
        };

        let style_profile = match &req.style {
            Some(name) => {
                let s = style::load(&self.fs, name)?;
                if !s.applies_to(req.family) {
                    return Err(EngineError::invalid(format!("style '{name}' does not apply to {}", req.family)));
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
        let pinned = self.pinned_style_for(req.family, style_profile.as_ref());
        let mut sources = self.resolve_style(&profile, pinned.as_ref(), stylesrc::DEFAULT_SEND, false);
        let explicit_model = req.model.clone().or_else(|| style_profile.as_ref().and_then(|s| s.model.clone()));
        let explicit_refs = req.references.len();
        let wants_refs = explicit_refs > 0 || !sources.send_paths().is_empty();
        let mut model_id = explicit_model.clone().unwrap_or_else(|| provider.default_model(wants_refs));
        let model_refs = |m: &str| provider.descriptor().model(m).map(|x| if x.supports_edit { usize::from(x.max_refs) } else { 0 }).unwrap_or(0);
        let cap = model_refs(&model_id).saturating_sub(explicit_refs).min(stylesrc::DEFAULT_SEND);
        sources.limit_send(cap, &format!("{provider_id} {model_id} takes at most {} reference images", model_refs(&model_id)));
        if explicit_model.is_none() && explicit_refs == 0 && sources.send_paths().is_empty() {
            model_id = provider.default_model(false);
        }

        let mut inputs: Vec<ReferenceInput> =
            sources.send_paths().into_iter().map(|path| ReferenceInput { path, role: RefRole::StyleAnchor }).collect();
        inputs.extend(req.references.iter().cloned());
        let mut loaded: Vec<LoadedRef> = Vec::new();
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
        let source_canvas = profile.imagegen.source_canvas.filter(|c| *c != canvas);
        let ask = source_canvas.unwrap_or(canvas);
        // finishing that needs the mod's templates is refused now, before anything is spent
        self.check_finishing_resources(imagegen_core::chain::recipe_of(&profile))?;
        let prompt_style = style_profile.as_ref().or(pinned.as_ref());
        let prompt = build_prompt(&profile, ask, &brief, prompt_style, &loaded, transparent, sources.report.style_paragraph.as_deref());

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
        let mut warnings: Vec<String> = Vec::new();
        let mut approvals: Vec<String> = Vec::new();
        if let Some(model) = provider.descriptor().model(&model_id) {
            if let ModelStatus::Deprecated { shutdown } = &model.status {
                warnings.push(format!("model {model_id} is deprecated and shuts down on {shutdown}"));
            }
            // the unit art recipes read a silhouette from a flat background themselves, so no alpha is needed there
            let keys_itself = matches!(
                imagegen_core::chain::recipe_of(&profile),
                imagegen_core::chain::Recipe::UnitCounter | imagegen_core::chain::Recipe::DivisionSymbol
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
            if let Some(p) = &pinned {
                warnings.push(format!(
                    "the project's pinned style '{}' for {} was applied automatically (references and style text)",
                    p.name, req.family
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
        normalized.canvas = (profile.free_canvas() && canvas != profile.canvas).then_some(canvas);
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

/// Convenience for tests and the CLI: a family by id string.
pub fn parse_family(s: &str) -> EResult<Family> {
    s.parse::<Family>().map_err(EngineError::from)
}
