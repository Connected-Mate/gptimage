//! `hoi4.image_plan`: a dry run that costs nothing and fixes everything a generation will use.

use imagegen_core::category::Category;
use imagegen_core::error::RetryClass;
use imagegen_core::export;
use imagegen_core::naming::{self, asset_id};
use imagegen_core::plan::{aspect_of, build_prompt, plan_path, GenerationPlan, Plan, PlanEstimate, PlanPaths, PlanRequest};
use imagegen_core::profile::Background;
use imagegen_core::records::{kind, RefSummary};
use imagegen_core::refs::{self, LoadedRef, RefRole, ReferenceInput};
use imagegen_core::style;
use imagegen_providers::types::{AlphaSupport, ImageRequest, ModelStatus};
use serde_json::json;

use super::{EResult, Engine, EngineError};

/// Default provider order when a plan names none (decision D-12a and the maintainer's answer of 2026-11-01).
pub const PROVIDER_ORDER: [&str; 5] = ["chatgpt_login", "openai", "gemini", "fal", "comfyui"];

impl Engine {
    /// Resolve and store a plan.
    pub fn plan(&self, req: PlanRequest) -> EResult<Plan> {
        export::validate_asset_name(req.category, &req.asset)?;
        naming::validate_snake("feature slug", &req.feature_slug)?;
        let brief = req.brief.trim().to_string();
        if brief.is_empty() || brief.chars().count() > 4000 || brief.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
            return Err(EngineError::invalid("brief must be 1 to 4000 characters of plain text"));
        }
        let profile = self
            .profiles
            .for_category(req.category)
            .ok_or_else(|| EngineError::new("not_found", format!("no profile for {}", req.category), RetryClass::Terminal))?
            .clone();

        let canvas = match (profile.canvas, req.canvas) {
            ([0, 0], Some(c)) if (1..=4096).contains(&c[0]) && (1..=4096).contains(&c[1]) => c,
            ([0, 0], Some(_)) => return Err(EngineError::invalid("canvas sides must be 1 to 4096")),
            ([0, 0], None) => return Err(EngineError::invalid(format!("{} has no fixed canvas: pass canvas [width, height]", req.category))),
            (p, None) => p,
            (p, Some(c)) if c == p => p,
            (p, Some(_)) => {
                return Err(EngineError::invalid(format!(
                    "{} has a fixed canvas of {}x{}; to use another, override the profile in .hoi4-imagegen/config.toml or the kit's profiles.json",
                    req.category, p[0], p[1]
                )))
            }
        };

        let style_profile = match &req.style {
            Some(name) => {
                let s = style::load(&self.fs, name)?;
                if !s.applies_to(req.category) {
                    return Err(EngineError::invalid(format!("style '{name}' does not apply to {}", req.category)));
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
        self.check_category_policy(&profile, &provider_id)?;
        let provider = self.registry.get(&provider_id).ok_or_else(|| {
            let why =
                self.registry.problems().get(&provider_id).cloned().unwrap_or_else(|| format!("known providers: {}", self.registry.ids().join(", ")));
            EngineError::new("provider_not_found", format!("provider '{provider_id}' is not available ({why})"), RetryClass::Terminal)
        })?;

        // references: the style's pinned ones first, then the caller's
        let game = self.game_dir();
        let mut inputs: Vec<ReferenceInput> = Vec::new();
        if let Some(s) = &style_profile {
            inputs.extend(s.references.iter().map(|r| ReferenceInput { path: r.path.clone(), role: RefRole::StyleAnchor }));
        }
        inputs.extend(req.references.iter().cloned());
        let mut loaded: Vec<LoadedRef> = Vec::new();
        for input in &inputs {
            loaded.push(refs::load_reference(&self.fs, game.as_deref(), &self.config.policy, input)?);
        }
        refs::check_set_caps(&loaded)?;

        let model_id = req
            .model
            .clone()
            .or_else(|| style_profile.as_ref().and_then(|s| s.model.clone()))
            .unwrap_or_else(|| provider.default_model(!loaded.is_empty()));
        if provider_id == "chatgpt_login" && model_id != provider_chatgpt_login::MODEL {
            return Err(EngineError::invalid(format!(
                "the ChatGPT-login route runs only {} at {} reasoning effort (maintainer decision); '{model_id}' cannot be used with it",
                provider_chatgpt_login::MODEL,
                provider_chatgpt_login::REASONING_EFFORT
            )));
        }
        let quality = req.quality.clone().or_else(|| self.config.provider(&provider_id).quality);
        let transparent = req.transparent.unwrap_or(profile.imagegen.background == Background::Transparent);
        let prompt = build_prompt(&profile, canvas, &brief, style_profile.as_ref(), &loaded, transparent);

        let probe = ImageRequest {
            model: model_id.clone(),
            prompt: prompt.text.clone(),
            negative: prompt.negative.clone(),
            width: canvas[0],
            height: canvas[1],
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
            if transparent && model.alpha == AlphaSupport::None {
                warnings.push(format!(
                    "{model_id} has no documented native transparency. A prompt cannot prove it: the result is validated from pixels and an opaque result is rejected. Use a model with native alpha or a matte workflow if this icon must be transparent"
                ));
            }
            if transparent && model.alpha == AlphaSupport::Preview {
                warnings.push(format!("{model_id} documents transparent backgrounds as a preview; the pixels are checked either way"));
            }
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
        if loaded.iter().any(|r| r.origin.is_vanilla()) {
            approvals.push("references include Paradox art; this project opted in with policy.allow_vanilla_references (decision D4)".into());
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
        normalized.canvas = (profile.canvas == [0, 0]).then_some(canvas);
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
            frames: profile.frames.max(1),
            provider: provider_id,
            model: model_id,
            quality,
            generation: GenerationPlan { width: gw, height: gh, transparent, aspect: aspect_of(canvas) },
            prompt,
            references: loaded
                .iter()
                .map(|r| RefSummary { path: r.path.clone(), role: r.role, origin: r.origin, sha256: r.sha256.clone() })
                .collect(),
            style: req.style.clone(),
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

/// Convenience for tests and the CLI: a category by id string.
pub fn parse_category(s: &str) -> EResult<Category> {
    s.parse::<Category>().map_err(EngineError::from)
}
