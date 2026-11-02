//! Replicate predictions API (`replicate`): create a prediction, poll it, download the output.
//!
//! NOT LIVE-VERIFIED. Built from Replicate's HTTP reference and each model's published input schema (read
//! 2026-11-02). No call was made to Replicate, and every test fixture below is a "documented shape, not
//! recorded" response.
//!
//! Wire format:
//! - `POST {base}/v1/models/{owner}/{name}/predictions` with `Authorization: Bearer $REPLICATE_API_TOKEN`,
//!   a body of `{"input": {...}}` and `Prefer: wait=N` so a fast model answers in the same request.
//! - The response carries `id`, `status` (`starting`, `processing`, `succeeded`, `failed`, `canceled`), `output`,
//!   `error` and `urls.get` / `urls.cancel`. The URLs are used as returned.
//! - Output files are HTTPS URLs on `replicate.delivery`. The reference says they need the `Authorization`
//!   header, so the token is sent to that Replicate-owned host and to no other.
//! - Cancel is `POST urls.cancel`.

use std::sync::Arc;
use std::time::Duration;

use imagegen_core::config::{Config, EnvSource, ProviderConfig};
use imagegen_core::error::RetryClass;
use serde_json::{json, Value};

use crate::common::{check_model, data_url, descriptor_from, parse_data_url, per_image_price, sniff_mime, status_error, ApiKey};
use crate::registry::{policy_for, read_key, ProviderRegistry};
use crate::sizing::{nearest_ratio, tier_size};
use crate::transport::{redact, send, HostPolicy, HttpRequest, HttpTransport, Method};
use crate::types::{
    CallContext, CostEstimate, CostModel, ErrorKind, Funding, ImageProvider, ImageRequest, JobRef, ModelInfo, ProviderDescriptor, ProviderError,
    ProviderOutput, Reconciled, SizeRule,
};

/// Default API host.
pub const DEFAULT_API_BASE: &str = "https://api.replicate.com";

/// Seconds a create call may hold the connection (`Prefer: wait=N`, documented range 1 to 60).
const PREFER_WAIT_SECS: u32 = 20;

/// Largest input image sent as a data URI. Replicate's input-files page recommends data URIs only below 1 MB
/// (the HTTP reference says 256 KB), so anything larger is refused rather than guessed.
const MAX_INLINE_IMAGE_BYTES: usize = 1_000_000;

/// Adapter state.
pub struct ReplicateProvider {
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    api_base: String,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
    poll_interval: Duration,
    max_polls: u32,
}

/// What one look at a prediction found.
enum Step {
    Done(Box<ProviderOutput>),
    Pending,
    Failed(ProviderError),
}

impl ReplicateProvider {
    /// Build the adapter.
    pub fn new(cfg: ProviderConfig, key: Option<String>, transport: Arc<dyn HttpTransport>, policy: HostPolicy) -> Self {
        let api_base = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_API_BASE.to_string()).trim_end_matches('/').to_string();
        ReplicateProvider {
            desc: descriptor_from(include_str!("../data/replicate.json")),
            cfg,
            key: key.map(ApiKey::new),
            api_base,
            transport,
            policy,
            poll_interval: Duration::from_millis(1500),
            max_polls: 400,
        }
    }

    /// Override polling (tests use a tiny budget).
    pub fn with_polling(mut self, interval: Duration, max_polls: u32) -> Self {
        self.poll_interval = interval;
        self.max_polls = max_polls;
        self
    }

    fn secrets(&self) -> Vec<String> {
        self.key.iter().map(|k| k.expose().to_string()).collect()
    }

    fn auth(&self) -> Result<String, ProviderError> {
        let key = self.key.as_ref().ok_or_else(|| ProviderError::not_configured("REPLICATE_API_TOKEN is not set"))?;
        Ok(format!("Bearer {}", key.expose()))
    }

    fn input_images(req: &ImageRequest) -> Result<Vec<String>, ProviderError> {
        let mut out = Vec::new();
        let mut push = |png: &[u8]| -> Result<(), ProviderError> {
            if png.len() > MAX_INLINE_IMAGE_BYTES {
                return Err(ProviderError::unsupported(format!(
                    "an input image of {} bytes is too large to send inline to Replicate (limit {MAX_INLINE_IMAGE_BYTES}); shrink the reference",
                    png.len()
                )));
            }
            out.push(data_url("image/png", png));
            Ok(())
        };
        if let Some(edit) = &req.edit {
            push(&edit.base_png)?;
        }
        for r in &req.refs {
            push(&r.png)?;
        }
        Ok(out)
    }

    /// Request body for a model, public for wire-format tests. Returns the model id and the `input` object.
    pub fn build_body(&self, req: &ImageRequest) -> Result<(String, Value), ProviderError> {
        let model = check_model(&self.desc, req)?;
        let SizeRule::AspectTiers { ratios, .. } = &model.sizes else {
            return Err(ProviderError::unsupported("the Replicate table lists only models that choose size by aspect ratio"));
        };
        let ratio = nearest_ratio(ratios, [req.width, req.height]).unwrap_or_else(|| "1:1".into());
        let mut input = json!({"prompt": req.prompt, "aspect_ratio": ratio, "output_format": "png"});
        if let Some(seed) = req.seed {
            input["seed"] = json!(seed);
        }
        let images = Self::input_images(req)?;
        match model.id.as_str() {
            "black-forest-labs/flux-schnell" | "black-forest-labs/flux-dev" => {
                input["megapixels"] = json!("1");
                input["num_outputs"] = json!(1);
            }
            "black-forest-labs/flux-2-pro" => {
                input["resolution"] = json!("1 MP");
                if !images.is_empty() {
                    input["input_images"] = json!(images);
                }
            }
            "black-forest-labs/flux-kontext-pro" => {
                if let Some(first) = images.first() {
                    input["input_image"] = json!(first);
                }
            }
            _ => {}
        }
        Ok((model.id.clone(), json!({"input": input})))
    }

    fn json_get(&self, url: &str) -> Result<Value, ProviderError> {
        let resp = send(&*self.transport, &self.policy, &HttpRequest::get(url).header("Authorization", &self.auth()?))?;
        if resp.status >= 400 {
            return Err(status_error(resp.status, &resp.body, &self.secrets()));
        }
        serde_json::from_slice(&resp.body).map_err(|e| {
            ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, format!("{url} did not return JSON: {e}"))
                .with_status(resp.status)
        })
    }

    /// The first output URL of a prediction (a string, or the first of an array).
    fn output_url(v: &Value) -> Option<String> {
        match v.get("output")? {
            Value::String(s) => Some(s.clone()),
            Value::Array(items) => items.iter().find_map(|i| i.as_str().map(str::to_string)),
            _ => None,
        }
    }

    fn download(&self, url: &str, job: &JobRef) -> Result<(String, Vec<u8>), ProviderError> {
        if let Some((m, b)) = parse_data_url(url) {
            return Ok((m, b));
        }
        // the reference says output files need the Authorization header; the policy only lets it reach replicate.delivery
        let req = HttpRequest::get(url).header("Authorization", &self.auth()?);
        let resp = send(&*self.transport, &self.policy, &req).map_err(|e| e.with_job(job.clone()))?;
        if resp.status != 200 {
            return Err(status_error(resp.status, &resp.body, &self.secrets()).with_job(job.clone()));
        }
        let mime = resp.header("content-type").map(|c| c.split(';').next().unwrap_or("").trim().to_string()).filter(|c| c.starts_with("image/"));
        let mime = mime.unwrap_or_else(|| sniff_mime(&resp.body).to_string());
        Ok((mime, resp.body))
    }

    /// Interpret a prediction object.
    fn step(&self, v: &Value, job: &JobRef, est_usd: Option<f64>) -> Result<Step, ProviderError> {
        match v.get("status").and_then(Value::as_str).unwrap_or("") {
            "starting" | "processing" => Ok(Step::Pending),
            "succeeded" => {
                let Some(url) = Self::output_url(v) else {
                    let removed = v.get("data_removed").and_then(Value::as_bool).unwrap_or(false);
                    let why = if removed {
                        "the prediction's data was removed (Replicate keeps API outputs for one hour)"
                    } else {
                        "the prediction succeeded without an output URL"
                    };
                    return Ok(Step::Failed(ProviderError::new(ErrorKind::MalformedResponse, RetryClass::Terminal, why).with_job(job.clone())));
                };
                let (mime, bytes) = self.download(&url, job)?;
                Ok(Step::Done(Box::new(ProviderOutput {
                    image: bytes,
                    mime,
                    provider_request_id: Some(job.id.clone()),
                    usage: v.get("metrics").cloned(),
                    actual_usd: est_usd,
                    usage_unknown: est_usd.is_none(),
                    model_echo: Some(job.model.clone()),
                    notes: vec![],
                    job: Some(job.clone()),
                })))
            }
            "failed" => {
                let text = v.get("error").and_then(Value::as_str).unwrap_or("the prediction failed");
                let text = redact(text, &self.secrets());
                let lower = text.to_ascii_lowercase();
                let moderated = ["nsfw", "flagged", "sensitive", "safety", "e005"].iter().any(|w| lower.contains(w));
                let (kind, retry) = if moderated {
                    (ErrorKind::ModerationBlocked, RetryClass::Terminal)
                } else {
                    (ErrorKind::ServerError, RetryClass::ReconcileFirst)
                };
                Ok(Step::Failed(ProviderError::new(kind, retry, format!("Replicate reported a failed prediction: {text}")).with_job(job.clone())))
            }
            "canceled" => {
                Ok(Step::Failed(ProviderError::new(ErrorKind::Cancelled, RetryClass::Terminal, "the prediction was canceled").with_job(job.clone())))
            }
            other => Ok(Step::Failed(
                ProviderError::new(
                    ErrorKind::MalformedResponse,
                    RetryClass::ReconcileFirst,
                    format!("the prediction reported unknown status '{other}'"),
                )
                .with_job(job.clone()),
            )),
        }
    }

    fn run(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        let auth = self.auth()?;
        let (model_id, body) = self.build_body(req)?;
        let est = self.estimate(req).ok().map(|e| e.usd);
        let submit = HttpRequest::post_json(format!("{}/v1/models/{}/predictions", self.api_base, model_id), &body)
            .header("Authorization", &auth)
            .header("Prefer", &format!("wait={PREFER_WAIT_SECS}"));
        let resp = send(&*self.transport, &self.policy, &submit)?;
        if resp.status != 200 && resp.status != 201 {
            return Err(status_error(resp.status, &resp.body, &self.secrets()));
        }
        let v: Value = serde_json::from_slice(&resp.body).map_err(|e| {
            ProviderError::new(
                ErrorKind::MalformedResponse,
                RetryClass::ReconcileFirst,
                format!("the create response is not JSON ({e}); a prediction may exist"),
            )
        })?;
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, "the create response has no id; a prediction may exist")
            })?
            .to_string();
        let url_of = |k: &str| v.pointer(&format!("/urls/{k}")).and_then(Value::as_str).map(str::to_string);
        let job =
            JobRef { provider: "replicate".into(), id, status_url: url_of("get"), response_url: None, cancel_url: url_of("cancel"), model: model_id };
        ctx.notify(&job);
        let mut current = v;
        for _ in 0..=self.max_polls {
            match self.step(&current, &job, est)? {
                Step::Done(out) => return Ok(*out),
                Step::Failed(e) => return Err(e),
                Step::Pending => {}
            }
            if ctx.cancel.is_cancelled() {
                let _ = self.cancel(&job, ctx);
                return Err(ProviderError::new(ErrorKind::Cancelled, RetryClass::Terminal, "cancelled").with_job(job));
            }
            let status_url = job.status_url.clone().ok_or_else(|| {
                ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, "the create response has no urls.get")
                    .with_job(job.clone())
            })?;
            ctx.sleeper.sleep(self.poll_interval);
            current = self.json_get(&status_url).map_err(|e| e.with_job(job.clone()))?;
        }
        Err(ProviderError::new(
            ErrorKind::Transport,
            RetryClass::ReconcileFirst,
            "the prediction is still running after the polling budget; reconcile it by id instead of resubmitting",
        )
        .with_job(job))
    }

    fn basis_text(&self, what: &str) -> String {
        format!("replicate price table retrieved {}: {what}", self.desc.retrieved)
    }
}

impl ImageProvider for ReplicateProvider {
    fn id(&self) -> &str {
        "replicate"
    }

    fn descriptor(&self) -> &ProviderDescriptor {
        &self.desc
    }

    fn is_configured(&self) -> bool {
        self.key.is_some()
    }

    fn default_model(&self, editing: bool) -> String {
        if let Some(m) = &self.cfg.model {
            return m.clone();
        }
        if editing {
            if let Some(m) = self.desc.models.iter().find(|m| m.supports_edit && m.max_refs == 1) {
                return m.id.clone();
            }
        }
        self.desc.recommended().map(|m| m.id.clone()).unwrap_or_else(|| "black-forest-labs/flux-schnell".into())
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        let model = check_model(&self.desc, req)?;
        match &model.sizes {
            SizeRule::AspectTiers { ratios, .. } => {
                let ratio = nearest_ratio(ratios, [req.width, req.height]).unwrap_or_else(|| "1:1".into());
                Ok(tier_size(&ratio))
            }
            _ => Err(ProviderError::unsupported("the model has no aspect ratio list")),
        }
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        let model: &ModelInfo = check_model(&self.desc, req)?;
        let (w, h) = self.prepare(req)?;
        match &model.cost {
            CostModel::PerMegapixel { first, additional } => {
                // the nominal size is rounded to whole pixels and can land a hair above 1 MP, so allow a small tolerance
                let out_mp = (f64::from(w) * f64::from(h) / 1_048_576.0 - 0.05).ceil().max(1.0);
                let inputs = req.refs.len() + usize::from(req.edit.is_some());
                let usd = first + additional * ((out_mp - 1.0) + inputs as f64);
                Ok(CostEstimate {
                    usd: (usd * 1e6).round() / 1e6,
                    basis: self.basis_text(&format!(
                        "{}: ${first} for a run with the first output megapixel, plus ${additional} per extra output or input megapixel; {out_mp} output MP and {inputs} input image(s) assumed 1 MP each",
                        model.id
                    )),
                    usage_unknown: false,
                    funding: Funding::UserKey,
                })
            }
            other => {
                let (usd, _) = per_image_price(other, None).ok_or_else(|| ProviderError::unsupported("no price in the table"))?;
                Ok(CostEstimate {
                    usd,
                    basis: self.basis_text(&format!("{} per output image", model.id)),
                    usage_unknown: false,
                    funding: Funding::UserKey,
                })
            }
        }
    }

    fn generate(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.edit.is_some() {
            return Err(ProviderError::unsupported("generate was called with an edit request"));
        }
        self.run(req, ctx)
    }

    fn edit(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.edit.is_none() {
            return Err(ProviderError::unsupported("edit was called without a base image"));
        }
        self.run(req, ctx)
    }

    fn reconcile(&self, job: &JobRef, _ctx: &CallContext) -> Result<Reconciled, ProviderError> {
        let Some(status_url) = &job.status_url else { return Ok(Reconciled::Unknown) };
        let v = self.json_get(status_url)?;
        match self.step(&v, job, None)? {
            Step::Done(out) => Ok(Reconciled::Completed(out)),
            Step::Pending => Ok(Reconciled::Pending),
            Step::Failed(e) => Ok(Reconciled::Failed(e.message)),
        }
    }

    fn cancel(&self, job: &JobRef, _ctx: &CallContext) -> Result<(), ProviderError> {
        let url = job.cancel_url.clone().ok_or_else(|| ProviderError::unsupported("the prediction has no cancel URL"))?;
        let mut req = HttpRequest::get(url).header("Authorization", &self.auth()?);
        req.method = Method::Post;
        let resp = send(&*self.transport, &self.policy, &req)?;
        match resp.status {
            200..=202 => Ok(()),
            s => Err(status_error(s, &resp.body, &self.secrets())),
        }
    }
}

/// Register this module's providers unless the project config disables them.
pub fn register(reg: &mut ProviderRegistry, config: &Config, env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) {
    let cfg = config.provider("replicate");
    if cfg.enabled == Some(false) {
        return;
    }
    match policy_for(&["api.replicate.com", ".replicate.delivery"], cfg.base_url.as_deref(), env) {
        Ok(policy) => {
            let key = read_key("replicate", &cfg, env);
            reg.insert(Arc::new(ReplicateProvider::new(cfg, key, transport, policy)));
        }
        Err(why) => reg.problem("replicate", why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FixtureTransport;
    use crate::types::{CancelToken, EditSpec, JobSink, ProviderRef, Sleeper};
    use imagegen_core::config::MapEnv;
    use imagegen_core::pixels::{encode_png, Rgba};
    use imagegen_core::refs::RefRole;
    use std::sync::Mutex;

    // DOCUMENTED SHAPE, NOT RECORDED: every response below follows Replicate's HTTP reference and model schemas
    // (read 2026-11-02). Nothing here came from a real Replicate call.

    const SCHNELL: &str = "black-forest-labs/flux-schnell";
    const KONTEXT: &str = "black-forest-labs/flux-kontext-pro";
    const FLUX2: &str = "black-forest-labs/flux-2-pro";

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [1, 2, 3, 255])).unwrap()
    }

    fn policy() -> HostPolicy {
        HostPolicy::api(&["api.replicate.com", ".replicate.delivery"])
    }

    fn provider(fixture: Arc<FixtureTransport>, key: Option<&str>) -> ReplicateProvider {
        ReplicateProvider::new(ProviderConfig::default(), key.map(str::to_string), fixture, policy()).with_polling(Duration::from_millis(1), 5)
    }

    fn request(model: &str) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "an iron wall".into(),
            negative: vec![],
            width: 100,
            height: 88,
            transparent: false,
            quality: None,
            refs: vec![],
            edit: None,
            idempotency_key: None,
            seed: Some(7),
        }
    }

    #[derive(Default)]
    struct CountingSleeper(Mutex<u32>);
    impl Sleeper for CountingSleeper {
        fn sleep(&self, _d: Duration) {
            *self.0.lock().unwrap() += 1;
        }
    }

    #[derive(Default)]
    struct Jobs(Mutex<Vec<JobRef>>);
    impl JobSink for Jobs {
        fn submitted(&self, job: &JobRef) {
            self.0.lock().unwrap().push(job.clone());
        }
    }

    fn prediction(status: &str, output: Value) -> Vec<u8> {
        let v = json!({
            "id": "pred-doc-1",
            "model": "black-forest-labs/flux-schnell",
            "status": status,
            "input": {"prompt": "an iron wall"},
            "output": output,
            "error": null,
            "urls": {
                "get": "https://api.replicate.com/v1/predictions/pred-doc-1",
                "cancel": "https://api.replicate.com/v1/predictions/pred-doc-1/cancel"
            }
        });
        serde_json::to_vec(&v).unwrap()
    }

    fn png_ref() -> ProviderRef {
        ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }
    }

    #[test]
    fn schnell_body_uses_the_nearest_documented_aspect_ratio() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("r8_test_000000"));
        assert_eq!(p.default_model(false), SCHNELL);
        assert_eq!(p.default_model(true), KONTEXT);
        let (model, body) = p.build_body(&request(SCHNELL)).unwrap();
        assert_eq!(model, SCHNELL);
        let input = &body["input"];
        assert_eq!(input["aspect_ratio"], "5:4");
        assert_eq!(input["output_format"], "png");
        assert_eq!(input["megapixels"], "1");
        assert_eq!(input["num_outputs"], 1);
        assert_eq!(input["seed"], 7);
        assert!(input.get("input_image").is_none());
        // flux-1.1-pro has no 21:9 ratio in its enum, kontext does
        let mut wide = request("black-forest-labs/flux-1.1-pro");
        wide.width = 397;
        wide.height = 153;
        assert_eq!(p.build_body(&wide).unwrap().1["input"]["aspect_ratio"], "16:9");
        let mut wide = request(SCHNELL);
        wide.width = 397;
        wide.height = 153;
        assert_eq!(p.build_body(&wide).unwrap().1["input"]["aspect_ratio"], "21:9");
    }

    #[test]
    fn kontext_and_flux2_carry_images_as_data_uris() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("r8_test_000000"));
        let mut k = request(KONTEXT);
        k.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let (_, body) = p.build_body(&k).unwrap();
        assert!(body["input"]["input_image"].as_str().unwrap().starts_with("data:image/png;base64,"));
        // kontext accepts exactly one image, so edit plus a reference is refused before sending
        k.refs = vec![png_ref()];
        assert_eq!(p.build_body(&k).unwrap_err().kind, ErrorKind::Unsupported);

        let mut f = request(FLUX2);
        f.edit = Some(EditSpec { base_png: png(), mask_png: None });
        f.refs = vec![png_ref(), png_ref()];
        let (_, body) = p.build_body(&f).unwrap();
        assert_eq!(body["input"]["input_images"].as_array().unwrap().len(), 3);
        assert_eq!(body["input"]["resolution"], "1 MP");

        // text-only models refuse references
        let mut s = request(SCHNELL);
        s.refs = vec![png_ref()];
        assert_eq!(p.build_body(&s).unwrap_err().kind, ErrorKind::Unsupported);
        // an oversize inline image is refused instead of guessed
        let mut big = request(KONTEXT);
        big.edit = Some(EditSpec { base_png: vec![0; MAX_INLINE_IMAGE_BYTES + 1], mask_png: None });
        assert!(p.build_body(&big).unwrap_err().message.contains("too large"));
    }

    #[test]
    fn prefer_wait_answer_is_used_without_polling() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(
            Method::Post,
            "/v1/models/black-forest-labs/flux-schnell/predictions",
            201,
            &[],
            prediction("succeeded", json!(["https://replicate.delivery/xezq/doc/out-0.png"])),
        );
        fixture.respond(Method::Get, "replicate.delivery", 200, &[("content-type", "image/png")], png());
        let p = provider(fixture.clone(), Some("r8_test_000000"));
        let sleeper = Arc::new(CountingSleeper::default());
        let jobs = Arc::new(Jobs::default());
        let ctx = CallContext { cancel: CancelToken::new(), sleeper: sleeper.clone(), jobs: Some(jobs.clone()) };
        let out = p.generate(&request(SCHNELL), &ctx).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!(*sleeper.0.lock().unwrap(), 0);
        assert_eq!(jobs.0.lock().unwrap().len(), 1, "the job is announced even when the first answer is final");
        assert!((out.actual_usd.unwrap() - 0.003).abs() < 1e-9);
        let reqs = fixture.requests();
        let create = &reqs[0];
        assert!(create.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer r8_test_000000"));
        assert!(create.headers.iter().any(|(k, v)| k == "Prefer" && v == "wait=20"));
        let body: Value = serde_json::from_slice(&create.body).unwrap();
        assert_eq!(body["input"]["prompt"], "an iron wall");
    }

    #[test]
    fn polling_flow_runs_until_succeeded_and_downloads_from_replicate_delivery() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/predictions", 201, &[], prediction("starting", Value::Null));
        fixture.respond(Method::Get, "/v1/predictions/pred-doc-1", 200, &[], prediction("processing", Value::Null));
        fixture.respond(
            Method::Get,
            "/v1/predictions/pred-doc-1",
            200,
            &[],
            prediction("succeeded", json!("https://pbxt.replicate.delivery/doc/out.png")),
        );
        fixture.respond(Method::Get, "pbxt.replicate.delivery", 200, &[], png());
        let p = provider(fixture.clone(), Some("r8_test_000000"));
        let sleeper = Arc::new(CountingSleeper::default());
        let ctx = CallContext { cancel: CancelToken::new(), sleeper: sleeper.clone(), jobs: None };
        let out = p.generate(&request("black-forest-labs/flux-1.1-pro"), &ctx).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png", "the type is sniffed when the host sends none");
        assert_eq!(*sleeper.0.lock().unwrap(), 2);
        assert_eq!(out.job.as_ref().unwrap().id, "pred-doc-1");
        assert!((out.actual_usd.unwrap() - 0.04).abs() < 1e-9);
    }

    #[test]
    fn cancellation_posts_to_the_cancel_url_and_keeps_the_job() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/predictions", 201, &[], prediction("starting", Value::Null));
        fixture.respond(Method::Post, "/cancel", 200, &[], prediction("canceled", Value::Null));
        let p = provider(fixture.clone(), Some("r8_test_000000"));
        let ctx = CallContext::new();
        ctx.cancel.cancel();
        let err = p.generate(&request(SCHNELL), &ctx).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Cancelled);
        assert_eq!(err.job.as_ref().unwrap().id, "pred-doc-1");
        assert!(fixture.requests().iter().any(|r| r.method == Method::Post && r.url.ends_with("/pred-doc-1/cancel")));
    }

    #[test]
    fn running_out_of_polls_demands_reconciliation() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/predictions", 201, &[], prediction("starting", Value::Null));
        fixture.respond(Method::Get, "/v1/predictions/pred-doc-1", 200, &[], prediction("processing", Value::Null));
        let p = provider(fixture, Some("r8_test_000000"));
        let ctx = CallContext { cancel: CancelToken::new(), sleeper: Arc::new(CountingSleeper::default()), jobs: None };
        let err = p.generate(&request(SCHNELL), &ctx).unwrap_err();
        assert_eq!(err.retry, RetryClass::ReconcileFirst);
        assert!(err.job.is_some());
    }

    #[test]
    fn a_lost_response_is_reconciled_by_prediction_id() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Get, "/v1/predictions/pred-doc-1", 200, &[], prediction("processing", Value::Null));
        fixture.respond(
            Method::Get,
            "/v1/predictions/pred-doc-1",
            200,
            &[],
            prediction("succeeded", json!(["https://replicate.delivery/doc/out.png"])),
        );
        fixture.respond(Method::Get, "replicate.delivery", 200, &[("content-type", "image/png")], png());
        let p = provider(fixture, Some("r8_test_000000"));
        let job = JobRef {
            provider: "replicate".into(),
            id: "pred-doc-1".into(),
            status_url: Some("https://api.replicate.com/v1/predictions/pred-doc-1".into()),
            response_url: None,
            cancel_url: None,
            model: SCHNELL.into(),
        };
        assert!(matches!(p.reconcile(&job, &CallContext::new()).unwrap(), Reconciled::Pending));
        match p.reconcile(&job, &CallContext::new()).unwrap() {
            Reconciled::Completed(out) => assert_eq!(out.image, png()),
            other => panic!("expected completion, got {other:?}"),
        }
        let none = JobRef { status_url: None, ..job };
        assert!(matches!(p.reconcile(&none, &CallContext::new()).unwrap(), Reconciled::Unknown));
    }

    #[test]
    fn failed_canceled_and_removed_predictions_are_reported_not_retried() {
        let fixture = Arc::new(FixtureTransport::new());
        let failed = json!({"id": "pred-doc-1", "status": "failed", "error": "NSFW content detected in the output", "output": null});
        fixture.respond(Method::Get, "/v1/predictions/failed", 200, &[], serde_json::to_vec(&failed).unwrap());
        let crashed = json!({"id": "pred-doc-1", "status": "failed", "error": "CUDA out of memory", "output": null});
        fixture.respond(Method::Get, "/v1/predictions/crashed", 200, &[], serde_json::to_vec(&crashed).unwrap());
        let gone = json!({"id": "pred-doc-1", "status": "succeeded", "output": null, "data_removed": true});
        fixture.respond(Method::Get, "/v1/predictions/gone", 200, &[], serde_json::to_vec(&gone).unwrap());
        let p = provider(fixture, Some("r8_test_000000"));
        let job = |name: &str| JobRef {
            provider: "replicate".into(),
            id: name.into(),
            status_url: Some(format!("https://api.replicate.com/v1/predictions/{name}")),
            response_url: None,
            cancel_url: None,
            model: SCHNELL.into(),
        };
        for name in ["failed", "crashed", "gone"] {
            assert!(matches!(p.reconcile(&job(name), &CallContext::new()).unwrap(), Reconciled::Failed(_)), "{name}");
        }
        // through generate the moderation message maps to its own kind
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/predictions", 201, &[], serde_json::to_vec(&json!({"id": "p", "status": "failed", "error": "NSFW content detected", "urls": {"get": "https://api.replicate.com/v1/predictions/p"}})).unwrap());
        let p = provider(fixture, Some("r8_test_000000"));
        let err = p.generate(&request(SCHNELL), &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.retry), (ErrorKind::ModerationBlocked, RetryClass::Terminal));
    }

    #[test]
    fn output_urls_outside_replicate_delivery_are_blocked_and_never_get_the_token() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/predictions", 201, &[], prediction("succeeded", json!(["https://evil.example/steal.png"])));
        let p = provider(fixture.clone(), Some("r8_test_000000"));
        let err = p.generate(&request(SCHNELL), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Blocked);
        assert!(!fixture.requests().iter().any(|r| r.url.contains("evil.example")));
    }

    #[test]
    fn status_urls_pointing_elsewhere_are_blocked_too() {
        let fixture = Arc::new(FixtureTransport::new());
        let body = json!({"id": "p", "status": "starting", "urls": {"get": "https://evil.example/status", "cancel": "https://evil.example/c"}});
        fixture.respond(Method::Post, "/predictions", 201, &[], serde_json::to_vec(&body).unwrap());
        let p = provider(fixture.clone(), Some("r8_test_000000"));
        let err = p.generate(&request(SCHNELL), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Blocked);
        assert!(!fixture.requests().iter().any(|r| r.url.contains("evil.example")));
    }

    #[test]
    fn estimates_follow_the_published_prices() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("r8_test_000000"));
        assert!((p.estimate(&request(SCHNELL)).unwrap().usd - 0.003).abs() < 1e-9);
        assert!((p.estimate(&request("black-forest-labs/flux-dev")).unwrap().usd - 0.025).abs() < 1e-9);
        assert!((p.estimate(&request(KONTEXT)).unwrap().usd - 0.04).abs() < 1e-9);
        let gen = p.estimate(&request(FLUX2)).unwrap();
        assert!((gen.usd - 0.03).abs() < 1e-9, "{gen:?}");
        let mut edit = request(FLUX2);
        edit.edit = Some(EditSpec { base_png: png(), mask_png: None });
        assert!((p.estimate(&edit).unwrap().usd - 0.045).abs() < 1e-9);
        assert!(gen.basis.contains("2026-11-02"));
    }

    #[test]
    fn errors_redaction_and_missing_key() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/predictions", 401, &[], r#"{"detail":"Invalid token r8_test_secret_77"}"#);
        let p = provider(fixture, Some("r8_test_secret_77"));
        let err = p.generate(&request(SCHNELL), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Auth);
        assert!(!err.message.contains("r8_test_secret_77"));
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/predictions", 429, &[], r#"{"detail":"slow down"}"#);
        fixture.respond(Method::Post, "/predictions", 429, &[], r#"{"detail":"slow down"}"#);
        let p = provider(fixture, Some("r8_test_000000"));
        assert_eq!(p.generate(&request(SCHNELL), &CallContext::new()).unwrap_err().kind, ErrorKind::RateLimited);
        let p = provider(Arc::new(FixtureTransport::new()), None);
        assert!(!p.is_configured());
        assert_eq!(p.generate(&request(SCHNELL), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);
        let p = provider(Arc::new(FixtureTransport::new()), Some("r8_test_000000"));
        assert_eq!(p.generate(&request("stability-ai/sdxl"), &CallContext::new()).unwrap_err().kind, ErrorKind::Unsupported);
    }

    #[test]
    fn register_inserts_even_without_a_key_and_honours_enabled_false() {
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::default(), Arc::new(FixtureTransport::new()));
        let p = reg.get("replicate").expect("registered without a key so doctor can say what to set");
        assert!(!p.is_configured());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::from_pairs(&[("REPLICATE_API_TOKEN", "r8_test_1")]), Arc::new(FixtureTransport::new()));
        assert!(reg.get("replicate").unwrap().is_configured());
        let off = Config::parse("[providers.replicate]\nenabled = false\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &off, &MapEnv::default(), Arc::new(FixtureTransport::new()));
        assert!(reg.get("replicate").is_none());
        let bad = Config::parse("[providers.replicate]\nbase_url = \"https://evil.example\"\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &bad, &MapEnv::default(), Arc::new(FixtureTransport::new()));
        assert!(reg.get("replicate").is_none() && reg.problems().contains_key("replicate"));
    }

    #[test]
    fn the_table_has_one_recommended_model_and_is_marked_not_live_verified() {
        let d = descriptor_from(include_str!("../data/replicate.json"));
        assert_eq!(d.models.iter().filter(|m| m.recommended).count(), 1);
        assert!(d.terms_note.contains("Not live-verified") && d.models.iter().all(|m| m.notes.contains("Not live-verified")));
        let raw: Value = serde_json::from_str(include_str!("../data/replicate.json")).unwrap();
        assert_eq!(raw["live_verified"], false);
        assert!(raw["doc_urls"].as_array().unwrap().len() >= 5);
    }
}
