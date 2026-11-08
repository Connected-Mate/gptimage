//! `hoi4.image_generate`, `hoi4.image_edit` and `hoi4.image_cancel`.
//!
//! The order of events around a paid call is fixed: gates, an intent entry in the ledger, the call, then the
//! candidate and its record. A crash between the call and the record leaves an unfinished intent, which blocks a
//! blind retry with the same key and can be reconciled through the provider where it supports job ids.

use std::sync::Arc;

use imagegen_core::error::RetryClass;
use imagegen_core::hashing::{canonical_hash, sha256_hex};
use imagegen_core::naming::{candidate_id, ArtifactId};
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, SourceFormat};
use imagegen_core::records::{kind, CandidateRecord, CostRecord, ProfileRef, RefSummary};
use imagegen_core::refs::{self, RefRole, ReferenceInput};
use imagegen_providers::types::{CallContext, EditSpec, ImageProvider, ImageRequest, JobRef, JobSink, ProviderOutput, ProviderRef, Reconciled};
use serde_json::{json, Value};

use crate::api::{CancelParams, EditParams, GenerateParams};
use imagegen_core::ledger::Entry;

use super::{ActiveJob, EResult, Engine, EngineError};

/// Everything that identifies one paid call before it is made.
struct CallSpec {
    template: CandidateRecord,
    provider_id: String,
    request: ImageRequest,
    idempotency_key: String,
    usd_per_call: f64,
    approve_cost: bool,
    candidate_dir: String,
    prompts_dir: String,
    editing: bool,
}

struct LedgerSink {
    engine_ledger: imagegen_core::ledger::Ledger,
    jobs: Arc<super::JobTable>,
    job_id: String,
    asset_id: String,
}

impl JobSink for LedgerSink {
    fn submitted(&self, job: &JobRef) {
        let _ = self.engine_ledger.append(kind::JOB_SUBMITTED, Some(&self.asset_id), None, json!({"job_id": self.job_id, "remote": job}));
        self.jobs.set_remote(&self.job_id, job.clone());
    }
}

fn job_id_for(key: &str, asset_id: &str) -> String {
    format!("job-{}", &sha256_hex(format!("{asset_id}|{key}").as_bytes())[..12])
}

impl Engine {
    /// Run a stored plan: one provider call per variant.
    pub fn generate(&self, p: GenerateParams) -> EResult<Value> {
        validate_key(&p.idempotency_key)?;
        let plan = self.load_plan(&p.plan_id)?;
        let profile = self.profile_with_pack(plan.request.family, plan.profile.style_pack.as_deref())?;
        if plan.profile.id != plan.request.family.as_str() {
            return Err(EngineError::refused("GFX-FAMILY-MISMATCH", "the plan's profile is not its family's profile", RetryClass::Terminal));
        }
        self.check_family_policy(&profile, &plan.provider)?;
        if self.profile_ref(&profile).hash != plan.profile.hash {
            return Err(EngineError::refused(
                "profile_changed",
                "the profile changed after this plan was made (the kit's profiles.json, the project config or the style pack was edited). Make a new plan",
                RetryClass::RefreshPreconditions,
            ));
        }
        let provider = self
            .registry
            .get(&plan.provider)
            .ok_or_else(|| EngineError::new("provider_not_found", format!("provider '{}' is not available", plan.provider), RetryClass::Terminal))?;

        // references must still be what the plan hashed
        let game = self.game_dir();
        let mut provider_refs = Vec::new();
        for r in &plan.references {
            let loaded = refs::load_reference(&self.fs, game.as_deref(), &ReferenceInput { path: r.path.clone(), role: r.role })?;
            if loaded.sha256 != r.sha256 {
                return Err(EngineError::refused(
                    "reference_changed",
                    format!("reference {} no longer matches the plan; make a new plan", r.path),
                    RetryClass::RefreshPreconditions,
                ));
            }
            provider_refs.push(ProviderRef { role: loaded.role, png: loaded.png, sha256: loaded.sha256 });
        }

        let mut outputs = Vec::new();
        let mut total = 0.0;
        for i in 0..plan.variants {
            let key = if plan.variants == 1 { p.idempotency_key.clone() } else { format!("{}#{}", p.idempotency_key, i + 1) };
            let request = ImageRequest {
                model: plan.model.clone(),
                prompt: plan.prompt.text.clone(),
                negative: plan.prompt.negative.clone(),
                width: plan.source_canvas.unwrap_or(plan.canvas)[0],
                height: plan.source_canvas.unwrap_or(plan.canvas)[1],
                transparent: plan.generation.transparent,
                quality: plan.quality.clone(),
                refs: provider_refs.clone(),
                edit: None,
                idempotency_key: Some(key.clone()),
                seed: None,
            };
            let spec = CallSpec {
                template: CandidateRecord {
                    candidate_id: String::new(),
                    asset_id: plan.asset_id.clone(),
                    family: plan.request.family,
                    feature_slug: plan.request.feature_slug.clone(),
                    asset: plan.request.asset.clone(),
                    profile: plan.profile.clone(),
                    canvas: plan.canvas,
                    version: 0,
                    path: String::new(),
                    sha256: String::new(),
                    pixels_sha256: String::new(),
                    size: [0, 0],
                    has_transparency: false,
                    provider: plan.provider.clone(),
                    model: plan.model.clone(),
                    plan_id: Some(plan.plan_id.clone()),
                    plan_hash: Some(plan.plan_hash.clone()),
                    request_hash: String::new(),
                    idempotency_key: Some(key.clone()),
                    prompt_sha256: plan.prompt.sha256.clone(),
                    references: plan.references.clone(),
                    cost: CostRecord {
                        estimated_usd: plan.estimate.usd_per_call,
                        reserved_usd: plan.estimate.usd_per_call,
                        actual_usd: None,
                        usage_unknown: plan.estimate.usage_unknown,
                        funding: plan.estimate.funding.clone(),
                        basis: plan.estimate.basis.clone(),
                    },
                    parent: None,
                    provider_request_id: None,
                    usage: None,
                    note: None,
                    subfamily: plan.subfamily.as_ref().map(|x| x.id.clone()),
                },
                provider_id: plan.provider.clone(),
                request,
                idempotency_key: key,
                usd_per_call: plan.estimate.usd_per_call,
                approve_cost: p.approve_cost.unwrap_or(false),
                candidate_dir: plan.paths.candidate_dir.clone(),
                prompts_dir: plan.paths.prompts_dir.clone(),
                editing: false,
            };
            let (record, replayed) = self.execute(&*provider, spec)?;
            if !replayed {
                total += record.cost.actual_usd.unwrap_or(record.cost.reserved_usd);
            }
            outputs.push(candidate_json(&record, replayed, &self.fs));
        }
        Ok(json!({
            "plan_id": plan.plan_id,
            "candidates": outputs,
            "spent_usd_estimate": (total * 1e6).round() / 1e6,
            "next": "Run hoi4.image_process on a candidate, then validate, preview, review and install. Raw candidates are expected to fail the size check until they are processed."
        }))
    }

    /// Refine a candidate with a provider edit.
    pub fn edit(&self, p: EditParams) -> EResult<Value> {
        validate_key(&p.idempotency_key)?;
        let instruction = p.instruction.trim().to_string();
        if instruction.is_empty() || instruction.chars().count() > 4000 {
            return Err(EngineError::invalid("instruction must be 1 to 4000 characters"));
        }
        let artifact = self.artifact(&p.candidate_id)?;
        let profile = self.profile_of(&artifact)?;
        let (bytes, sha) = self.read_artifact(&artifact)?;
        let (base_img, _) = decode(&bytes, Some(&artifact.path), DecodeLimits::default())?;
        let base_png = encode_png(&base_img)?;

        let provider_id = p
            .provider
            .clone()
            .or_else(|| (artifact.provider != "import" && self.registry.get(&artifact.provider).is_some()).then(|| artifact.provider.clone()))
            .or_else(|| self.config.defaults.provider.clone())
            .or_else(|| {
                let c = self.registry.configured_ids();
                (c.len() == 1).then(|| c[0].clone())
            })
            .ok_or_else(|| EngineError::invalid("name a provider for this edit"))?;
        self.check_family_policy(&profile, &provider_id)?;
        let provider = self
            .registry
            .get(&provider_id)
            .ok_or_else(|| EngineError::new("provider_not_found", format!("provider '{provider_id}' is not available"), RetryClass::Terminal))?;
        let model = p.model.clone().unwrap_or_else(|| provider.default_model(true));
        let quality = p.quality.clone().or_else(|| self.config.provider(&provider_id).quality);

        let game = self.game_dir();
        let mut loaded = Vec::new();
        for r in &p.extra_references {
            loaded.push(refs::load_reference(&self.fs, game.as_deref(), r)?);
        }
        refs::check_set_caps(&loaded)?;
        let mask_png = match &p.mask {
            Some(m) => {
                let raw = self.fs.read(m, 24 * 1024 * 1024)?;
                let (img, _) = decode(&raw, Some(m), DecodeLimits::default())?;
                if img.width != base_img.width || img.height != base_img.height {
                    return Err(EngineError::invalid(format!(
                        "the mask is {}x{} but the image is {}x{}; they must match",
                        img.width, img.height, base_img.width, base_img.height
                    )));
                }
                Some(encode_png(&img)?)
            }
            None => None,
        };

        let mut text = format!("Edit the first image. {instruction}\nKeep everything else exactly as it is: the same subject, composition, style, palette and background.\n");
        if !loaded.is_empty() {
            text.push_str("\nAdditional reference images, in the order attached after the first:\n");
            for (i, r) in loaded.iter().enumerate() {
                text.push_str(&format!("{}. {}\n", i + 2, r.role.phrase()));
            }
        }
        text.push_str("\nThe result must still meet these requirements:\n");
        let subst = |s: &str| imagegen_core::plan::fill_rule(&profile, artifact.canvas, s);
        let (rules, negative) = self.artifact_prompt_rules(&artifact, &profile);
        for rule in &rules {
            text.push_str(&format!("- {}\n", subst(rule)));
        }
        text.push_str("\nProduce exactly one image.\n");

        let transparent = profile.source_transparent();
        let request = ImageRequest {
            model: model.clone(),
            prompt: text.clone(),
            negative,
            width: artifact.canvas[0],
            height: artifact.canvas[1],
            transparent,
            quality,
            refs: loaded.iter().map(|r| ProviderRef { role: r.role, png: r.png.clone(), sha256: r.sha256.clone() }).collect(),
            edit: Some(EditSpec { base_png, mask_png }),
            idempotency_key: Some(p.idempotency_key.clone()),
            seed: None,
        };
        let est = provider.estimate(&request)?;
        // lineage: the edit keeps the original plan, so the install gate still checks it
        let root_plan = self.root_plan(&artifact.root_id)?;
        let mut references: Vec<RefSummary> = vec![RefSummary {
            path: artifact.path.clone(),
            role: RefRole::EditTarget,
            origin: refs::RefOrigin::PreviousCandidate,
            sha256: sha.clone(),
        }];
        references.extend(loaded.iter().map(|r| RefSummary { path: r.path.clone(), role: r.role, origin: r.origin, sha256: r.sha256.clone() }));
        let spec = CallSpec {
            template: CandidateRecord {
                candidate_id: String::new(),
                asset_id: artifact.asset_id.clone(),
                family: artifact.family,
                feature_slug: artifact.feature_slug.clone(),
                asset: artifact.asset.clone(),
                profile: artifact.profile.clone(),
                canvas: artifact.canvas,
                version: 0,
                path: String::new(),
                sha256: String::new(),
                pixels_sha256: String::new(),
                size: [0, 0],
                has_transparency: false,
                provider: provider_id.clone(),
                model,
                plan_id: root_plan.0,
                plan_hash: root_plan.1,
                request_hash: String::new(),
                idempotency_key: Some(p.idempotency_key.clone()),
                prompt_sha256: sha256_hex(text.as_bytes()),
                references,
                cost: CostRecord {
                    estimated_usd: est.usd,
                    reserved_usd: est.usd,
                    actual_usd: None,
                    usage_unknown: est.usage_unknown,
                    funding: est.funding.as_str().to_string(),
                    basis: est.basis.clone(),
                },
                parent: Some(artifact.id.clone()),
                provider_request_id: None,
                usage: None,
                note: Some(format!("edit: {instruction}")),
                subfamily: artifact.subfamily.clone(),
            },
            provider_id,
            request,
            idempotency_key: p.idempotency_key.clone(),
            usd_per_call: est.usd,
            approve_cost: p.approve_cost.unwrap_or(false),
            candidate_dir: format!("docs/assets/{}/source_png", artifact.feature_slug),
            prompts_dir: format!("docs/assets/{}/prompts", artifact.feature_slug),
            editing: true,
        };
        let (record, replayed) = self.execute(&*provider, spec)?;
        Ok(
            json!({"candidates": [candidate_json(&record, replayed, &self.fs)], "next": "Process the new candidate, then validate, preview, review and install."}),
        )
    }

    /// Plan id and hash of the root candidate's plan, if it came from one.
    fn root_plan(&self, root_id: &str) -> EResult<(Option<String>, Option<String>)> {
        let entries = self.ledger.entries()?;
        let rec = entries
            .iter()
            .filter(|e| matches!(e.kind.as_str(), kind::GENERATED | kind::EDITED | kind::IMPORTED))
            .find(|e| e.artifact_id.as_deref() == Some(root_id))
            .and_then(|e| serde_json::from_value::<CandidateRecord>(e.data.clone()).ok());
        Ok(rec.map(|r| (r.plan_id, r.plan_hash)).unwrap_or((None, None)))
    }

    /// The shared path of generate and edit. Returns the record and whether it was a replay.
    fn execute(&self, provider: &dyn ImageProvider, spec: CallSpec) -> EResult<(CandidateRecord, bool)> {
        let CallSpec { mut template, provider_id, request, idempotency_key, usd_per_call, approve_cost, candidate_dir, prompts_dir, editing } = spec;

        // the request fingerprint: same key with a different request is a caller bug, not a replay
        let request_hash = canonical_hash(&json!({
            "provider": provider_id, "model": request.model, "prompt": sha256_hex(request.prompt.as_bytes()),
            "w": request.width, "h": request.height, "transparent": request.transparent, "quality": request.quality,
            "refs": request.refs.iter().map(|r| r.sha256.clone()).collect::<Vec<_>>(),
            "edit": request.edit.as_ref().map(|e| sha256_hex(&e.base_png)),
            "mask": request.edit.as_ref().and_then(|e| e.mask_png.as_ref()).map(|m| sha256_hex(m)),
            "asset": template.asset_id, "parent": template.parent,
        }));
        template.request_hash = request_hash.clone();

        if let Some(prev) = self.ledger.find_idempotency(&idempotency_key)? {
            let rec: CandidateRecord =
                serde_json::from_value(prev.data.clone()).map_err(|e| EngineError::new("ledger_error", e.to_string(), RetryClass::Terminal))?;
            if rec.request_hash != request_hash {
                return Err(EngineError::refused(
                    "idempotency_key_reused",
                    format!(
                        "the key '{idempotency_key}' was used for a different request (candidate {}). Use a new key for a new request",
                        rec.candidate_id
                    ),
                    RetryClass::Terminal,
                ));
            }
            return Ok((rec, true));
        }

        // an earlier attempt that never finished may or may not have been charged: reconcile before spending again
        if let Some(open) = self.unfinished_for_key(&idempotency_key)? {
            return Err(EngineError::refused(
                "uncertain_previous_attempt",
                format!(
                    "an earlier attempt with the key '{idempotency_key}' (job {open}) never finished, so it may have been charged. Run hoi4.image_cancel with reconcile = true for that job, or check the provider's account and then either use a new key or retire this one with hoi4.image_cancel {{idempotency_key, abandon: true}}"
                ),
                RetryClass::ReconcileFirst,
            ));
        }

        if usd_per_call > self.config.policy.max_usd_per_call && !approve_cost {
            return Err(EngineError::refused(
                "requires_authorization",
                format!(
                    "this call is estimated at ${usd_per_call:.4}, above the project's limit of ${:.2} per call. Pass approve_cost = true to confirm",
                    self.config.policy.max_usd_per_call
                ),
                RetryClass::RequiresAuthorization,
            ));
        }
        if !provider.is_configured() {
            return Err(EngineError::new(
                "provider_not_configured",
                format!("provider {provider_id} has no credentials in the environment (keys are read from environment variables only)"),
                RetryClass::RequiresAuthorization,
            ));
        }

        let job_id = job_id_for(&idempotency_key, &template.asset_id);
        let cancel = imagegen_providers::CancelToken::new();
        self.jobs.insert(
            &job_id,
            ActiveJob { cancel: cancel.clone(), provider: provider_id.clone(), idempotency_key: idempotency_key.clone(), remote: None },
        );
        let intent = json!({
            "job_id": job_id, "provider": provider_id, "model": request.model, "asset_id": template.asset_id,
            "idempotency_key": idempotency_key, "request_hash": request_hash, "estimated_usd": usd_per_call,
            "editing": editing, "candidate_dir": candidate_dir, "prompts_dir": prompts_dir,
            "prompt": request.prompt, "template": template,
        });
        if let Err(e) = self.ledger.append(kind::JOB_INTENT, Some(&template.asset_id), None, intent) {
            self.jobs.remove(&job_id);
            return Err(e.into());
        }

        let ctx = CallContext {
            cancel,
            sleeper: self.sleeper.clone(),
            jobs: Some(Arc::new(LedgerSink {
                engine_ledger: self.ledger.clone(),
                jobs: self.jobs.clone(),
                job_id: job_id.clone(),
                asset_id: template.asset_id.clone(),
            })),
        };
        let result = if editing { provider.edit(&request, &ctx) } else { provider.generate(&request, &ctx) };
        self.jobs.remove(&job_id);
        let out = match result {
            Ok(o) => o,
            Err(e) => {
                let outcome = if e.retry == RetryClass::ReconcileFirst { "uncertain" } else { "failed" };
                let _ = self.ledger.append(
                    kind::JOB_ENDED,
                    Some(&template.asset_id),
                    None,
                    json!({"job_id": job_id, "outcome": outcome, "code": format!("{:?}", e.kind), "retry_class": e.retry, "message": e.message, "job": e.job}),
                );
                return Err(e.into());
            }
        };
        match self.finish_candidate(template, &out, &candidate_dir, &prompts_dir, &request.prompt, editing) {
            Ok(rec) => {
                let _ = self.ledger.append(
                    kind::JOB_ENDED,
                    Some(&rec.asset_id),
                    Some(&rec.candidate_id),
                    json!({"job_id": job_id, "outcome": "completed", "candidate_id": rec.candidate_id}),
                );
                Ok((rec, false))
            }
            Err(e) => {
                let _ = self.ledger.append(kind::JOB_ENDED, None, None, json!({"job_id": job_id, "outcome": "unusable_output", "message": e.message, "note": "the provider returned an image this tool could not store; the call may have been charged"}));
                Err(e)
            }
        }
    }

    /// Store a provider output as the next candidate version and record it.
    fn finish_candidate(
        &self,
        mut rec: CandidateRecord,
        out: &ProviderOutput,
        candidate_dir: &str,
        prompts_dir: &str,
        prompt_text: &str,
        editing: bool,
    ) -> EResult<CandidateRecord> {
        let (img, format) = decode(&out.image, None, DecodeLimits::default()).map_err(|e| {
            EngineError::new(
                "provider_bad_image",
                format!("the provider returned an image that cannot be read: {e}. The call may have been charged"),
                RetryClass::ReconcileFirst,
            )
        })?;
        let ext = match format {
            SourceFormat::Png => "png",
            SourceFormat::Jpeg => "jpg",
            SourceFormat::WebP => "webp",
            _ => "png",
        };
        // a provider that hands back another family's installed art (a reference echoed as the answer) is refused
        imagegen_core::reuse::ensure_no_cross_family_reuse(&self.fs, &self.profiles, rec.family, &img.pixels_sha256(), "the provider's image")?;
        let min = self.ledger.highest_version(&rec.asset_id)?;
        let written = self.fs.write_next_version(candidate_dir, &rec.asset, ext, min, &out.image)?;
        rec.version = written.version;
        rec.candidate_id = candidate_id(&rec.asset_id, written.version);
        rec.path = written.rel.clone();
        rec.sha256 = sha256_hex(&out.image);
        rec.pixels_sha256 = img.pixels_sha256();
        rec.size = [img.width, img.height];
        rec.has_transparency = img.has_transparency();
        rec.provider_request_id = out.provider_request_id.clone();
        rec.usage = out.usage.clone();
        rec.cost.actual_usd = out.actual_usd;
        rec.cost.usage_unknown = out.usage_unknown;
        if !out.notes.is_empty() {
            let joined = out.notes.join("; ");
            rec.note = Some(match rec.note.take() {
                Some(n) => format!("{n}; {joined}"),
                None => joined,
            });
        }
        // a readable prompt record next to the candidate (best effort: an existing file is left alone)
        let prompt_file = format!(
            "# {} (candidate {})\n\nprovider: {}\nmodel: {}\nrequest_hash: {}\nprompt_sha256: {}\nkind: {}\n\n## Prompt\n\n{}\n",
            rec.asset_id,
            rec.candidate_id,
            rec.provider,
            rec.model,
            rec.request_hash,
            rec.prompt_sha256,
            if editing { "edit" } else { "generate" },
            prompt_text
        );
        let _ = self.fs.write_new(&format!("{prompts_dir}/{}-v{}.md", rec.asset, rec.version), prompt_file.as_bytes());
        let ledger_kind = if editing { kind::EDITED } else { kind::GENERATED };
        self.ledger.append(ledger_kind, Some(&rec.asset_id), Some(&rec.candidate_id), &rec)?;
        Ok(rec)
    }

    /// An intent for this key with no matching end entry.
    fn unfinished_for_key(&self, key: &str) -> EResult<Option<String>> {
        let entries = self.ledger.entries()?;
        // the end must come after the intent: a retry with the same key reuses the job id, so an earlier end entry
        // (an abandoned or failed first attempt) does not close the second attempt
        Ok(entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == kind::JOB_INTENT && e.data.get("idempotency_key").and_then(Value::as_str) == Some(key))
            .filter_map(|(i, e)| e.data.get("job_id").and_then(Value::as_str).map(|id| (i, id)))
            .find(|(i, id)| !job_ended_after(&entries, *i, id) && self.jobs.find(Some(id), None).is_none())
            .map(|(_, id)| id.to_string()))
    }

    /// Retire an uncertain idempotency key: close the unfinished attempt(s) with the outcome `abandoned`, record when
    /// and the warning that the first attempt may have been charged, and let the key proceed.
    ///
    /// Refused when the key's attempt is known to have completed (a candidate exists for it) or is still running in
    /// this process. An attempt that already ended in any way has nothing left to abandon.
    fn abandon(&self, p: &CancelParams) -> EResult<Value> {
        if let Some((id, _)) = self.jobs.find(p.job_id.as_deref(), p.idempotency_key.as_deref()) {
            return Err(EngineError::refused(
                "job_still_running",
                format!(
                    "job {id} is still running in this process. Cancel it first (without abandon); abandon is only for an attempt that was cut off"
                ),
                RetryClass::Terminal,
            ));
        }
        let entries = self.ledger.entries()?;
        let matches = |e: &Entry| {
            e.kind == kind::JOB_INTENT
                && ((p.job_id.is_some() && e.data.get("job_id").and_then(Value::as_str) == p.job_id.as_deref())
                    || (p.idempotency_key.is_some() && e.data.get("idempotency_key").and_then(Value::as_str) == p.idempotency_key.as_deref()))
        };
        if !entries.iter().any(matches) {
            return Err(EngineError::new("not_found", "no recorded attempt matches that job id or idempotency key", RetryClass::Terminal));
        }
        let key = p
            .idempotency_key
            .clone()
            .or_else(|| entries.iter().rfind(|e| matches(e)).and_then(|e| e.data.get("idempotency_key").and_then(Value::as_str).map(str::to_string)));
        if let Some(key) = &key {
            if let Some(done) = self.ledger.find_idempotency(key)? {
                return Err(EngineError::refused(
                    "attempt_completed",
                    format!(
                        "the attempt with the key '{key}' completed (candidate {}), so there is nothing to abandon. Calling generate with the same key and the same plan returns that candidate; use a new key to make another",
                        done.artifact_id.as_deref().unwrap_or("recorded in the ledger")
                    ),
                    RetryClass::Terminal,
                ));
            }
        }
        let recovered = |job_id: &str| {
            entries.iter().any(|e| {
                e.kind == kind::JOB_ENDED
                    && e.data.get("job_id").and_then(Value::as_str) == Some(job_id)
                    && matches!(e.data.get("outcome").and_then(Value::as_str), Some("completed" | "recovered"))
            })
        };
        let mut open: Vec<(String, Option<String>)> = Vec::new();
        for (i, e) in entries.iter().enumerate().filter(|(_, e)| matches(e)) {
            let id = e.data.get("job_id").and_then(Value::as_str).unwrap_or_default().to_string();
            if recovered(&id) {
                return Err(EngineError::refused(
                    "attempt_completed",
                    format!("job {id} is recorded as completed, so there is nothing to abandon"),
                    RetryClass::Terminal,
                ));
            }
            if !job_ended_after(&entries, i, &id) && !open.iter().any(|(o, _)| *o == id) {
                open.push((id, e.asset_id.clone()));
            }
        }
        if open.is_empty() {
            return Ok(json!({
                "status": "nothing_to_abandon", "idempotency_key": key,
                "note": "every recorded attempt for this key already ended, so the key is not blocked"
            }));
        }
        let when = self.now();
        let warning = "The first attempt was cut off before an outcome was recorded, so it may have been charged. Check the provider's usage page if the cost matters.";
        let mut ids = Vec::new();
        for (id, asset) in &open {
            self.ledger.append(
                kind::JOB_ENDED,
                asset.as_deref(),
                None,
                json!({"job_id": id, "outcome": "abandoned", "idempotency_key": key, "abandoned_at": when, "warning": warning}),
            )?;
            ids.push(id.clone());
        }
        Ok(json!({
            "status": "abandoned", "job_ids": ids, "idempotency_key": key, "abandoned_at": when, "warning": warning,
            "next": "The key can be used again, or use a new one. The ledger keeps the abandoned attempt."
        }))
    }

    /// Cancel a running job, or settle one a crashed run left behind.
    pub fn cancel(&self, p: CancelParams) -> EResult<Value> {
        if p.job_id.is_none() && p.idempotency_key.is_none() {
            return Err(EngineError::invalid("give a job_id or an idempotency_key"));
        }
        if p.abandon.unwrap_or(false) {
            if p.reconcile.unwrap_or(false) {
                return Err(EngineError::invalid(
                    "abandon and reconcile cannot be combined: reconcile asks the provider, abandon gives up on the attempt",
                ));
            }
            return self.abandon(&p);
        }
        if let Some((id, job)) = self.jobs.find(p.job_id.as_deref(), p.idempotency_key.as_deref()) {
            job.cancel.cancel();
            return Ok(json!({
                "job_id": id, "status": "cancel_requested", "provider": job.provider, "remote": job.remote,
                "note": "Providers with job ids are asked to cancel at their next poll. A synchronous API (OpenAI, Gemini) cannot be interrupted once its request is sent, and may still be charged."
            }));
        }
        // not running here: look for an unfinished job in the ledger
        let entries = self.ledger.entries()?;
        let (intent_idx, intent) = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == kind::JOB_INTENT)
            .rfind(|(_, e)| {
                let id = e.data.get("job_id").and_then(Value::as_str);
                let key = e.data.get("idempotency_key").and_then(Value::as_str);
                (p.job_id.as_deref().is_some() && id == p.job_id.as_deref())
                    || (p.idempotency_key.as_deref().is_some() && key == p.idempotency_key.as_deref())
            })
            .ok_or_else(|| EngineError::new("not_found", "no running or recorded job matches", RetryClass::Terminal))?;
        let job_id = intent.data.get("job_id").and_then(Value::as_str).unwrap_or_default().to_string();
        if job_ended_after(&entries, intent_idx, &job_id) {
            return Ok(json!({"job_id": job_id, "status": "already_ended", "note": "the ledger shows this job finished"}));
        }
        let remote: Option<JobRef> = entries
            .iter()
            .rfind(|e| e.kind == kind::JOB_SUBMITTED && e.data.get("job_id").and_then(Value::as_str) == Some(job_id.as_str()))
            .and_then(|e| e.data.get("remote").cloned())
            .and_then(|v| serde_json::from_value(v).ok());
        let Some(remote) = remote else {
            return Ok(json!({
                "job_id": job_id, "status": "unknown_outcome",
                "note": "This job used a synchronous provider, which has no job id to ask about. Whether the request was charged is unknown: check the provider's usage page, then retire the key with hoi4.image_cancel {idempotency_key, abandon: true} (or use a new idempotency key) if you want to try again."
            }));
        };
        let provider = self.registry.get(&remote.provider).ok_or_else(|| {
            EngineError::new("provider_not_found", format!("provider '{}' is not available", remote.provider), RetryClass::Terminal)
        })?;
        let ctx = CallContext::new();
        let asset_id = intent.data.get("asset_id").and_then(Value::as_str).map(str::to_string);
        if p.reconcile.unwrap_or(false) {
            return match provider.reconcile(&remote, &ctx)? {
                Reconciled::Completed(out) => {
                    let tpl: CandidateRecord = serde_json::from_value(intent.data.get("template").cloned().unwrap_or(Value::Null))
                        .map_err(|e| EngineError::new("ledger_error", format!("the job's intent record is unreadable: {e}"), RetryClass::Terminal))?;
                    let dir = intent.data.get("candidate_dir").and_then(Value::as_str).unwrap_or_default().to_string();
                    let pdir = intent.data.get("prompts_dir").and_then(Value::as_str).unwrap_or_default().to_string();
                    let prompt = intent.data.get("prompt").and_then(Value::as_str).unwrap_or_default().to_string();
                    let editing = intent.data.get("editing").and_then(Value::as_bool).unwrap_or(false);
                    let mut tpl = tpl;
                    tpl.note = Some("recovered by reconciling the provider job after an interrupted run".into());
                    let rec = self.finish_candidate(tpl, &out, &dir, &pdir, &prompt, editing)?;
                    let _ = self.ledger.append(
                        kind::JOB_ENDED,
                        asset_id.as_deref(),
                        Some(&rec.candidate_id),
                        json!({"job_id": job_id, "outcome": "recovered", "candidate_id": rec.candidate_id}),
                    );
                    Ok(json!({"job_id": job_id, "status": "recovered", "candidate": candidate_json(&rec, false, &self.fs)}))
                }
                Reconciled::Pending => {
                    Ok(json!({"job_id": job_id, "status": "pending", "note": "the provider is still working on it; reconcile again later"}))
                }
                Reconciled::Failed(why) => {
                    let _ = self.ledger.append(
                        kind::JOB_ENDED,
                        asset_id.as_deref(),
                        None,
                        json!({"job_id": job_id, "outcome": "failed", "message": why}),
                    );
                    Ok(json!({"job_id": job_id, "status": "failed", "message": why}))
                }
                Reconciled::Unknown => Ok(json!({"job_id": job_id, "status": "unknown_outcome", "note": "the provider cannot say"})),
            };
        }
        provider.cancel(&remote, &ctx)?;
        let _ = self.ledger.append(kind::JOB_ENDED, asset_id.as_deref(), None, json!({"job_id": job_id, "outcome": "cancelled"}));
        Ok(json!({"job_id": job_id, "status": "cancelled", "remote": remote}))
    }
}

/// True when the job has an end entry after the ledger position `idx`.
fn job_ended_after(entries: &[Entry], idx: usize, job_id: &str) -> bool {
    entries.iter().skip(idx + 1).any(|e| e.kind == kind::JOB_ENDED && e.data.get("job_id").and_then(Value::as_str) == Some(job_id))
}

fn validate_key(key: &str) -> EResult<()> {
    let ok = !key.is_empty() && key.len() <= 128 && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-' | '#'));
    if ok {
        Ok(())
    } else {
        Err(EngineError::invalid("idempotency_key must be 1 to 128 characters of letters, digits and . _ : - #"))
    }
}

/// Candidate as returned to callers: the record plus an alpha summary.
fn candidate_json(rec: &CandidateRecord, replayed: bool, fs: &imagegen_core::fsio::ProjectFs) -> Value {
    let transparent_pixels = fs
        .read(&rec.path, 64 * 1024 * 1024)
        .ok()
        .and_then(|b| decode(&b, Some(&rec.path), DecodeLimits::default()).ok())
        .map(|(img, _)| img.data.chunks_exact(4).filter(|p| p[3] == 0).count() as u64);
    let mut v = serde_json::to_value(rec).unwrap_or(Value::Null);
    v["replayed"] = json!(replayed);
    v["alpha"] = json!({"has_transparency": rec.has_transparency, "fully_transparent_pixels": transparent_pixels});
    v["needs_processing"] = json!(true);
    v
}

/// Public helper: the artifact id parser, re-exported for the CLI.
pub fn parse_artifact(id: &str) -> EResult<ArtifactId> {
    Ok(ArtifactId::parse(id)?)
}

#[allow(dead_code)]
fn _profile_ref_type_check(_: &ProfileRef) {}
