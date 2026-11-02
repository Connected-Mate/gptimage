//! Black Forest Labs FLUX API (`bfl`): submit, poll the returned `polling_url`, download `result.sample`.
//!
//! NOT LIVE-VERIFIED. Built from BFL's API documentation read on 2026-11-02. No call was made to BFL, and every
//! test fixture below is a "documented shape, not recorded" response.
//!
//! Wire format:
//! - `POST {base}/v1/{model}` with the key in an `x-key` header. The answer is `{id, polling_url, cost, input_mp,
//!   output_mp}`, where `cost` is in credits and 1 credit is $0.01.
//! - The docs say to use `polling_url` as returned (it may name a regional host). `GET` it until `status` is
//!   `Ready`, then download `result.sample`. Other statuses: `Pending`, `Reasoning`, `Generating`, `Task not found`,
//!   `Request Moderated`, `Content Moderated`, `No Card Generated`, `Error`, `Failed`.
//! - Delivery URLs live under `delivery.*.bfl.ai`, expire after 10 minutes and need no key.
//! - The API has no documented cancel call, so a cancelled wait just stops polling.

use std::sync::Arc;
use std::time::Duration;

use imagegen_core::config::{Config, EnvSource, ProviderConfig};
use imagegen_core::error::RetryClass;
use serde_json::{json, Value};

use crate::common::{b64, check_model, descriptor_from, per_image_price, sniff_mime, status_error, ApiKey};
use crate::registry::{policy_for, read_key, ProviderRegistry};
use crate::sizing::{choose_size, nearest_ratio, tier_size};
use crate::transport::{redact, send, HostPolicy, HttpRequest, HttpTransport};
use crate::types::{
    CallContext, CostEstimate, CostModel, ErrorKind, Funding, ImageProvider, ImageRequest, JobRef, ProviderDescriptor, ProviderError, ProviderOutput,
    Reconciled, SizeRule,
};

/// Default global API host.
pub const DEFAULT_API_BASE: &str = "https://api.bfl.ai";

/// USD per BFL credit (pricing page: "1 credit = $0.01 USD").
const USD_PER_CREDIT: f64 = 0.01;

/// Adapter state.
pub struct BflProvider {
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    api_base: String,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
    poll_interval: Duration,
    max_polls: u32,
}

enum Step {
    Done(Box<ProviderOutput>),
    Pending,
    Failed(ProviderError),
}

impl BflProvider {
    /// Build the adapter.
    pub fn new(cfg: ProviderConfig, key: Option<String>, transport: Arc<dyn HttpTransport>, policy: HostPolicy) -> Self {
        let api_base = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_API_BASE.to_string()).trim_end_matches('/').to_string();
        BflProvider {
            desc: descriptor_from(include_str!("../data/bfl.json")),
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

    fn key_value(&self) -> Result<String, ProviderError> {
        self.key.as_ref().map(|k| k.expose().to_string()).ok_or_else(|| ProviderError::not_configured("BFL_API_KEY is not set"))
    }

    fn error_for(&self, status: u16, body: &[u8]) -> ProviderError {
        if status == 402 {
            return ProviderError::new(
                ErrorKind::RateLimited,
                RetryClass::RequiresAuthorization,
                "HTTP 402: the BFL account has insufficient credits",
            )
            .with_status(402);
        }
        status_error(status, body, &self.secrets())
    }

    /// Request body for a model, public for wire-format tests. Returns the endpoint model id and the JSON body.
    pub fn build_body(&self, req: &ImageRequest) -> Result<(String, Value), ProviderError> {
        let model = check_model(&self.desc, req)?;
        let mut body = json!({"prompt": req.prompt, "output_format": "png"});
        match &model.sizes {
            SizeRule::AspectTiers { ratios, .. } => {
                body["aspect_ratio"] = json!(nearest_ratio(ratios, [req.width, req.height]).unwrap_or_else(|| "1:1".into()));
            }
            rule => {
                let (w, h) = choose_size(rule, [req.width, req.height])?;
                body["width"] = json!(w);
                body["height"] = json!(h);
            }
        }
        if let Some(seed) = req.seed {
            body["seed"] = json!(seed);
        }
        if model.id == "flux-2-pro" {
            body["disable_pup"] = json!(true);
        }
        let mut images: Vec<String> = Vec::new();
        if let Some(edit) = &req.edit {
            images.push(b64(&edit.base_png));
        }
        for r in &req.refs {
            images.push(b64(&r.png));
        }
        for (i, img) in images.iter().enumerate() {
            let name = if i == 0 { "input_image".to_string() } else { format!("input_image_{}", i + 1) };
            body[name] = json!(img);
        }
        Ok((model.id.clone(), body))
    }

    fn poll_status(&self, url: &str) -> Result<Value, ProviderError> {
        let req = HttpRequest::get(url).header("x-key", &self.key_value()?);
        let resp = send(&*self.transport, &self.policy, &req)?;
        if resp.status >= 400 {
            return Err(self.error_for(resp.status, &resp.body));
        }
        serde_json::from_slice(&resp.body).map_err(|e| {
            ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, format!("the polling URL did not return JSON: {e}"))
                .with_status(resp.status)
        })
    }

    fn step(&self, v: &Value, job: &JobRef, est_usd: Option<f64>, reconciling: bool) -> Result<Step, ProviderError> {
        let status = v.get("status").and_then(Value::as_str).unwrap_or("");
        match status {
            "Pending" | "Reasoning" | "Generating" => Ok(Step::Pending),
            // right after a submit the id may not be visible yet, but a restarted reconcile finding nothing means it expired
            "Task not found" if !reconciling => Ok(Step::Pending),
            "Task not found" => Ok(Step::Failed(
                ProviderError::new(ErrorKind::ServerError, RetryClass::Terminal, "BFL no longer knows this task (results expire quickly)")
                    .with_job(job.clone()),
            )),
            "Ready" => {
                let url = v.pointer("/result/sample").and_then(Value::as_str).ok_or_else(|| {
                    ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, "the Ready result has no result.sample URL")
                        .with_job(job.clone())
                })?;
                // delivery URLs are signed and public: the key is not sent to them
                let resp = send(&*self.transport, &self.policy, &HttpRequest::get(url)).map_err(|e| e.with_job(job.clone()))?;
                if resp.status != 200 {
                    return Err(self.error_for(resp.status, &resp.body).with_job(job.clone()));
                }
                let mime =
                    resp.header("content-type").map(|c| c.split(';').next().unwrap_or("").trim().to_string()).filter(|c| c.starts_with("image/"));
                let mime = mime.unwrap_or_else(|| sniff_mime(&resp.body).to_string());
                let settled = v.get("cost").and_then(Value::as_f64);
                let actual = settled.map(|c| c * USD_PER_CREDIT).or(est_usd);
                Ok(Step::Done(Box::new(ProviderOutput {
                    image: resp.body,
                    mime,
                    provider_request_id: Some(job.id.clone()),
                    usage: settled.map(|c| json!({"cost_credits": c})),
                    actual_usd: actual,
                    usage_unknown: actual.is_none(),
                    model_echo: Some(job.model.clone()),
                    notes: vec![],
                    job: Some(job.clone()),
                })))
            }
            "Request Moderated" | "Content Moderated" => Ok(Step::Failed(
                ProviderError::new(ErrorKind::ModerationBlocked, RetryClass::Terminal, format!("BFL moderation refused the request ({status})"))
                    .with_job(job.clone()),
            )),
            "Error" | "Failed" | "No Card Generated" => {
                let detail = v.get("details").map(|d| redact(&d.to_string(), &self.secrets())).unwrap_or_default();
                Ok(Step::Failed(
                    ProviderError::new(
                        ErrorKind::ServerError,
                        RetryClass::ReconcileFirst,
                        format!("the job reported status '{status}' {detail}").trim().to_string(),
                    )
                    .with_job(job.clone()),
                ))
            }
            other => Ok(Step::Failed(
                ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, format!("the job reported unknown status '{other}'"))
                    .with_job(job.clone()),
            )),
        }
    }

    fn run(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        let key = self.key_value()?;
        let (model_id, body) = self.build_body(req)?;
        let est = self.estimate(req).ok().map(|e| e.usd);
        let submit = HttpRequest::post_json(format!("{}/v1/{}", self.api_base, model_id), &body).header("x-key", &key);
        let resp = send(&*self.transport, &self.policy, &submit)?;
        if resp.status != 200 {
            return Err(self.error_for(resp.status, &resp.body));
        }
        let v: Value = serde_json::from_slice(&resp.body).map_err(|e| {
            ProviderError::new(
                ErrorKind::MalformedResponse,
                RetryClass::ReconcileFirst,
                format!("the submit response is not JSON ({e}); a job may exist"),
            )
        })?;
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, "the submit response has no id; a job may exist")
            })?
            .to_string();
        let polling = v.get("polling_url").and_then(Value::as_str).map(str::to_string);
        let job = JobRef { provider: "bfl".into(), id, status_url: polling, response_url: None, cancel_url: None, model: model_id };
        ctx.notify(&job);
        let status_url = job.status_url.clone().ok_or_else(|| {
            ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, "the submit response has no polling_url")
                .with_job(job.clone())
        })?;
        // the submit response reports the charge in credits
        let est = v.get("cost").and_then(Value::as_f64).map(|c| c * USD_PER_CREDIT).or(est);
        for _ in 0..self.max_polls {
            if ctx.cancel.is_cancelled() {
                // BFL documents no cancel call: stop waiting and keep the job so it can be reconciled
                return Err(ProviderError::new(
                    ErrorKind::Cancelled,
                    RetryClass::Terminal,
                    "cancelled (BFL has no cancel call, the job may still finish and be billed)",
                )
                .with_job(job));
            }
            let st = self.poll_status(&status_url).map_err(|e| e.with_job(job.clone()))?;
            match self.step(&st, &job, est, false)? {
                Step::Done(out) => return Ok(*out),
                Step::Failed(e) => return Err(e),
                Step::Pending => ctx.sleeper.sleep(self.poll_interval),
            }
        }
        Err(ProviderError::new(
            ErrorKind::Transport,
            RetryClass::ReconcileFirst,
            "the job is still running after the polling budget; reconcile it by id instead of resubmitting",
        )
        .with_job(job))
    }

    fn basis_text(&self, what: &str) -> String {
        format!("bfl price table retrieved {}: {what}", self.desc.retrieved)
    }
}

impl ImageProvider for BflProvider {
    fn id(&self) -> &str {
        "bfl"
    }

    fn descriptor(&self) -> &ProviderDescriptor {
        &self.desc
    }

    fn is_configured(&self) -> bool {
        self.key.is_some()
    }

    fn default_model(&self, _editing: bool) -> String {
        self.cfg.model.clone().or_else(|| self.desc.recommended().map(|m| m.id.clone())).unwrap_or_else(|| "flux-2-pro".into())
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        let model = check_model(&self.desc, req)?;
        match &model.sizes {
            SizeRule::AspectTiers { ratios, .. } => {
                let ratio = nearest_ratio(ratios, [req.width, req.height]).unwrap_or_else(|| "1:1".into());
                Ok(tier_size(&ratio))
            }
            rule => choose_size(rule, [req.width, req.height]),
        }
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        let model = check_model(&self.desc, req)?;
        let (w, h) = self.prepare(req)?;
        match &model.cost {
            CostModel::PerMegapixel { first, additional } => {
                let out_mp = (f64::from(w) * f64::from(h) / 1_048_576.0 - 0.05).ceil().max(1.0);
                let inputs = req.refs.len() + usize::from(req.edit.is_some());
                let usd = first + additional * ((out_mp - 1.0) + inputs as f64);
                Ok(CostEstimate {
                    usd: (usd * 1e6).round() / 1e6,
                    basis: self.basis_text(&format!(
                        "{}: ${first} for the first output megapixel, ${additional} per further input or output megapixel; {out_mp} output MP and {inputs} input image(s) assumed 1 MP each",
                        model.id
                    )),
                    usage_unknown: false,
                    funding: Funding::UserKey,
                })
            }
            other => {
                let (usd, _) = per_image_price(other, None).ok_or_else(|| ProviderError::unsupported("no price in the table"))?;
                let proxy = if let CostModel::UnpublishedProxy { proxy_of, .. } = other {
                    format!(" (planning proxy, not a BFL figure: {proxy_of})")
                } else {
                    String::new()
                };
                Ok(CostEstimate {
                    usd,
                    basis: self.basis_text(&format!("{} per image{proxy}", model.id)),
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
        let v = self.poll_status(status_url)?;
        match self.step(&v, job, None, true)? {
            Step::Done(out) => Ok(Reconciled::Completed(out)),
            Step::Pending => Ok(Reconciled::Pending),
            Step::Failed(e) => Ok(Reconciled::Failed(e.message)),
        }
    }
}

/// Register this module's providers unless the project config disables them.
pub fn register(reg: &mut ProviderRegistry, config: &Config, env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) {
    let cfg = config.provider("bfl");
    if cfg.enabled == Some(false) {
        return;
    }
    match policy_for(&["api.bfl.ai", "api.eu.bfl.ai", "api.us.bfl.ai", ".bfl.ai"], cfg.base_url.as_deref(), env) {
        Ok(policy) => {
            let key = read_key("bfl", &cfg, env);
            reg.insert(Arc::new(BflProvider::new(cfg, key, transport, policy)));
        }
        Err(why) => reg.problem("bfl", why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{FixtureTransport, Method};
    use crate::types::{CancelToken, EditSpec, JobSink, ProviderRef, Sleeper};
    use imagegen_core::config::MapEnv;
    use imagegen_core::pixels::{encode_png, Rgba};
    use imagegen_core::refs::RefRole;
    use std::sync::Mutex;

    // DOCUMENTED SHAPE, NOT RECORDED: every response below follows BFL's documented schemas (read 2026-11-02).
    // Nothing here came from a real BFL call.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [1, 2, 3, 255])).unwrap()
    }

    fn policy() -> HostPolicy {
        HostPolicy::api(&["api.bfl.ai", "api.eu.bfl.ai", "api.us.bfl.ai", ".bfl.ai"])
    }

    fn provider(fixture: Arc<FixtureTransport>, key: Option<&str>) -> BflProvider {
        BflProvider::new(ProviderConfig::default(), key.map(str::to_string), fixture, policy()).with_polling(Duration::from_millis(1), 6)
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

    fn png_ref() -> ProviderRef {
        ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }
    }

    const POLL: &str = "https://api.eu1.bfl.ai/v1/get_result?id=task-doc-1";

    fn submit_ok(fixture: &FixtureTransport, model: &str, cost: Option<f64>) {
        let body = json!({"id": "task-doc-1", "polling_url": POLL, "cost": cost, "input_mp": 0.0, "output_mp": 1.05});
        fixture.respond(Method::Post, &format!("/v1/{model}"), 200, &[], serde_json::to_vec(&body).unwrap());
    }

    fn status(fixture: &FixtureTransport, st: &str, sample: Option<&str>) {
        let v = json!({"id": "task-doc-1", "status": st, "result": sample.map(|s| json!({"sample": s})), "progress": null, "details": null});
        fixture.respond(Method::Get, "get_result", 200, &[], serde_json::to_vec(&v).unwrap());
    }

    #[test]
    fn flux2_body_uses_pixel_sizes_numbered_input_images_and_no_prompt_upsampling() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("bfl-test-000000"));
        assert_eq!(p.default_model(false), "flux-2-pro");
        let mut r = request("flux-2-pro");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        r.refs = vec![png_ref(), png_ref()];
        let (model, body) = p.build_body(&r).unwrap();
        assert_eq!(model, "flux-2-pro");
        assert_eq!(body["output_format"], "png");
        assert_eq!(body["disable_pup"], true);
        assert_eq!(body["seed"], 7);
        let (w, h) = (body["width"].as_u64().unwrap(), body["height"].as_u64().unwrap());
        assert!(w % 16 == 0 && h % 16 == 0 && w * h >= 262_144);
        assert_eq!(body["input_image"], b64(&png()));
        assert!(body.get("input_image_2").is_some() && body.get("input_image_3").is_some() && body.get("input_image_4").is_none());
        assert!(body.get("aspect_ratio").is_none());
        // more than eight images are refused before sending
        r.refs = (0..8).map(|_| png_ref()).collect();
        assert_eq!(p.build_body(&r).unwrap_err().kind, ErrorKind::Unsupported);
    }

    #[test]
    fn legacy_models_use_their_documented_fields() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("bfl-test-000000"));
        let (_, pro) = p.build_body(&request("flux-pro-1.1")).unwrap();
        let (w, h) = (pro["width"].as_u64().unwrap(), pro["height"].as_u64().unwrap());
        assert!(w % 32 == 0 && h % 32 == 0 && (256..=1440).contains(&w) && (256..=1440).contains(&h), "{w}x{h}");
        assert!(pro.get("aspect_ratio").is_none() && pro.get("disable_pup").is_none());
        let (_, ultra) = p.build_body(&request("flux-pro-1.1-ultra")).unwrap();
        assert_eq!(ultra["aspect_ratio"], "5:4");
        assert!(ultra.get("width").is_none());
        let mut k = request("flux-kontext-pro");
        k.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let (_, kb) = p.build_body(&k).unwrap();
        assert_eq!(kb["aspect_ratio"], "5:4");
        assert_eq!(kb["input_image"], b64(&png()));
        // text-only legacy models refuse references and edits
        let mut u = request("flux-pro-1.1-ultra");
        u.refs = vec![png_ref()];
        assert_eq!(p.build_body(&u).unwrap_err().kind, ErrorKind::Unsupported);
        // every HOI4 canvas maps onto an allowed fixed size for flux-pro-1.1
        for c in [[100, 88], [397, 153], [156, 210], [457, 328], [10, 7], [45, 45], [146, 54]] {
            let mut r = request("flux-pro-1.1");
            r.width = c[0];
            r.height = c[1];
            let (w, h) = p.prepare(&r).unwrap();
            assert!(w % 32 == 0 && h % 32 == 0 && (256..=1440).contains(&w) && (256..=1440).contains(&h), "{c:?} -> {w}x{h}");
        }
    }

    #[test]
    fn full_flow_polls_downloads_without_the_key_and_uses_the_settled_cost() {
        let fixture = Arc::new(FixtureTransport::new());
        submit_ok(&fixture, "flux-2-pro", Some(3.0));
        status(&fixture, "Pending", None);
        status(&fixture, "Generating", None);
        status(&fixture, "Ready", Some("https://delivery-eu1.bfl.ai/results/doc/sample.png?s=sig"));
        fixture.respond(Method::Get, "delivery-eu1.bfl.ai", 200, &[("content-type", "image/png")], png());
        let p = provider(fixture.clone(), Some("bfl-test-000000"));
        let sleeper = Arc::new(CountingSleeper::default());
        let jobs = Arc::new(Jobs::default());
        let ctx = CallContext { cancel: CancelToken::new(), sleeper: sleeper.clone(), jobs: Some(jobs.clone()) };
        let out = p.generate(&request("flux-2-pro"), &ctx).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!(*sleeper.0.lock().unwrap(), 2);
        assert_eq!(jobs.0.lock().unwrap().len(), 1);
        let job = out.job.as_ref().unwrap();
        assert_eq!((job.id.as_str(), job.status_url.as_deref()), ("task-doc-1", Some(POLL)));
        assert!((out.actual_usd.unwrap() - 0.03).abs() < 1e-9, "3 credits are $0.03: {:?}", out.actual_usd);
        let reqs = fixture.requests();
        assert!(reqs[0].headers.iter().any(|(k, v)| k == "x-key" && v == "bfl-test-000000"));
        assert_eq!(reqs[0].url, "https://api.bfl.ai/v1/flux-2-pro");
        assert!(reqs.iter().filter(|r| r.url.contains("get_result")).all(|r| r.headers.iter().any(|(k, _)| k == "x-key")), "polling sends the key");
        let dl = reqs.iter().find(|r| r.url.contains("delivery-eu1")).unwrap();
        assert!(dl.headers.is_empty(), "the key must not go to the delivery host");
    }

    #[test]
    fn moderation_and_failure_statuses_map_to_distinct_errors() {
        for (st, kind, retry) in [
            ("Request Moderated", ErrorKind::ModerationBlocked, RetryClass::Terminal),
            ("Content Moderated", ErrorKind::ModerationBlocked, RetryClass::Terminal),
            ("Error", ErrorKind::ServerError, RetryClass::ReconcileFirst),
            ("Failed", ErrorKind::ServerError, RetryClass::ReconcileFirst),
        ] {
            let fixture = Arc::new(FixtureTransport::new());
            submit_ok(&fixture, "flux-2-pro", None);
            status(&fixture, st, None);
            let p = provider(fixture, Some("bfl-test-000000"));
            let err = p.generate(&request("flux-2-pro"), &CallContext::new()).unwrap_err();
            assert_eq!((err.kind, err.retry), (kind, retry), "{st}");
            assert_eq!(err.job.as_ref().unwrap().id, "task-doc-1");
        }
    }

    #[test]
    fn cancel_stops_polling_and_keeps_the_job_for_reconciliation() {
        let fixture = Arc::new(FixtureTransport::new());
        submit_ok(&fixture, "flux-2-pro", None);
        status(&fixture, "Pending", None);
        let p = provider(fixture.clone(), Some("bfl-test-000000"));
        let ctx = CallContext::new();
        ctx.cancel.cancel();
        let err = p.generate(&request("flux-2-pro"), &ctx).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Cancelled);
        assert!(err.job.is_some());
        assert!(!fixture.requests().iter().any(|r| r.url.contains("get_result")), "no further polling once cancelled");
        // BFL has no cancel call
        let job = err.job.unwrap();
        assert_eq!(p.cancel(&job, &CallContext::new()).unwrap_err().kind, ErrorKind::Unsupported);
    }

    #[test]
    fn running_out_of_polls_demands_reconciliation() {
        let fixture = Arc::new(FixtureTransport::new());
        submit_ok(&fixture, "flux-2-pro", None);
        status(&fixture, "Generating", None);
        let p = provider(fixture, Some("bfl-test-000000"));
        let ctx = CallContext { cancel: CancelToken::new(), sleeper: Arc::new(CountingSleeper::default()), jobs: None };
        let err = p.generate(&request("flux-2-pro"), &ctx).unwrap_err();
        assert_eq!(err.retry, RetryClass::ReconcileFirst);
        assert!(err.job.is_some());
    }

    #[test]
    fn a_lost_response_is_reconciled_from_the_polling_url() {
        let fixture = Arc::new(FixtureTransport::new());
        status(&fixture, "Pending", None);
        status(&fixture, "Ready", Some("https://delivery-us1.bfl.ai/doc/sample.png"));
        fixture.respond(Method::Get, "delivery-us1.bfl.ai", 200, &[], png());
        let p = provider(fixture, Some("bfl-test-000000"));
        let job = JobRef {
            provider: "bfl".into(),
            id: "task-doc-1".into(),
            status_url: Some(POLL.into()),
            response_url: None,
            cancel_url: None,
            model: "flux-2-pro".into(),
        };
        assert!(matches!(p.reconcile(&job, &CallContext::new()).unwrap(), Reconciled::Pending));
        match p.reconcile(&job, &CallContext::new()).unwrap() {
            Reconciled::Completed(out) => {
                assert_eq!(out.image, png());
                assert_eq!(out.mime, "image/png", "sniffed when the host sends no type");
            }
            other => panic!("expected completion, got {other:?}"),
        }
        let none = JobRef { status_url: None, ..job.clone() };
        assert!(matches!(p.reconcile(&none, &CallContext::new()).unwrap(), Reconciled::Unknown));
        // an unknown task after a restart is reported as failed, not pending forever
        let fixture = Arc::new(FixtureTransport::new());
        status(&fixture, "Task not found", None);
        let p = provider(fixture, Some("bfl-test-000000"));
        assert!(matches!(p.reconcile(&job, &CallContext::new()).unwrap(), Reconciled::Failed(_)));
    }

    #[test]
    fn hosts_outside_bfl_are_blocked_for_polling_and_download() {
        let fixture = Arc::new(FixtureTransport::new());
        let body = json!({"id": "t", "polling_url": "https://evil.example/poll"});
        fixture.respond(Method::Post, "/v1/flux-2-pro", 200, &[], serde_json::to_vec(&body).unwrap());
        let p = provider(fixture.clone(), Some("bfl-test-000000"));
        assert_eq!(p.generate(&request("flux-2-pro"), &CallContext::new()).unwrap_err().kind, ErrorKind::Blocked);
        assert!(!fixture.requests().iter().any(|r| r.url.contains("evil.example")));

        let fixture = Arc::new(FixtureTransport::new());
        submit_ok(&fixture, "flux-2-pro", None);
        status(&fixture, "Ready", Some("https://evil.example/steal.png"));
        let p = provider(fixture.clone(), Some("bfl-test-000000"));
        assert_eq!(p.generate(&request("flux-2-pro"), &CallContext::new()).unwrap_err().kind, ErrorKind::Blocked);
        assert!(!fixture.requests().iter().any(|r| r.url.contains("evil.example")));
    }

    #[test]
    fn estimates_follow_the_table_and_label_proxies() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("bfl-test-000000"));
        let gen = p.estimate(&request("flux-2-pro")).unwrap();
        assert!((gen.usd - 0.03).abs() < 1e-9, "{gen:?}");
        let mut edit = request("flux-2-pro");
        edit.edit = Some(EditSpec { base_png: png(), mask_png: None });
        edit.refs = vec![png_ref()];
        assert!((p.estimate(&edit).unwrap().usd - 0.06).abs() < 1e-9);
        for (m, usd) in [("flux-pro-1.1", 0.04), ("flux-pro-1.1-ultra", 0.06), ("flux-kontext-pro", 0.04), ("flux-kontext-max", 0.08)] {
            let e = p.estimate(&request(m)).unwrap();
            assert!((e.usd - usd).abs() < 1e-9, "{m}");
            assert!(e.basis.contains("planning proxy, not a BFL figure") && e.basis.contains("2026-11-02"), "{}", e.basis);
        }
    }

    #[test]
    fn errors_redaction_credits_and_missing_key() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/v1/flux-2-pro", 403, &[], r#"{"detail":"Not authenticated: bfl-test-secret-77"}"#);
        let p = provider(fixture, Some("bfl-test-secret-77"));
        let err = p.generate(&request("flux-2-pro"), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Auth);
        assert!(!err.message.contains("bfl-test-secret-77"));
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/v1/flux-2-pro", 402, &[], r#"{"detail":"insufficient credits"}"#);
        let p = provider(fixture.clone(), Some("bfl-test-000000"));
        let err = p.generate(&request("flux-2-pro"), &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.status), (ErrorKind::RateLimited, Some(402)));
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/v1/flux-2-pro", 429, &[], "{}");
        let p = provider(fixture, Some("bfl-test-000000"));
        assert_eq!(p.generate(&request("flux-2-pro"), &CallContext::new()).unwrap_err().kind, ErrorKind::RateLimited);
        let p = provider(Arc::new(FixtureTransport::new()), None);
        assert!(!p.is_configured());
        assert_eq!(p.generate(&request("flux-2-pro"), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);
        let p = provider(Arc::new(FixtureTransport::new()), Some("bfl-test-000000"));
        assert_eq!(p.generate(&request("flux-dev"), &CallContext::new()).unwrap_err().kind, ErrorKind::Unsupported);
    }

    #[test]
    fn register_inserts_even_without_a_key_and_honours_enabled_false_and_region_urls() {
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::default(), Arc::new(FixtureTransport::new()));
        assert!(!reg.get("bfl").expect("registered without a key").is_configured());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::from_pairs(&[("BFL_API_KEY", "bfl-test-1")]), Arc::new(FixtureTransport::new()));
        assert!(reg.get("bfl").unwrap().is_configured());
        let eu = Config::parse("[providers.bfl]\nbase_url = \"https://api.eu.bfl.ai\"\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &eu, &MapEnv::default(), Arc::new(FixtureTransport::new()));
        assert!(reg.get("bfl").is_some());
        let off = Config::parse("[providers.bfl]\nenabled = false\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &off, &MapEnv::default(), Arc::new(FixtureTransport::new()));
        assert!(reg.get("bfl").is_none());
    }

    #[test]
    fn the_table_has_one_recommended_model_and_is_marked_not_live_verified() {
        let d = descriptor_from(include_str!("../data/bfl.json"));
        assert_eq!(d.models.iter().filter(|m| m.recommended).count(), 1);
        assert!(d.terms_note.contains("Not live-verified") && d.models.iter().all(|m| m.notes.contains("Not live-verified")));
        let raw: Value = serde_json::from_str(include_str!("../data/bfl.json")).unwrap();
        assert_eq!(raw["live_verified"], false);
    }
}
