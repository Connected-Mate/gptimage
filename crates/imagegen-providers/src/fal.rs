//! fal.ai adapter (queue API).
//!
//! Submit with `POST https://queue.fal.run/{endpoint}` and `Authorization: Key $FAL_KEY`; the response carries
//! `request_id`, `status_url`, `response_url` and `cancel_url`, which are used as returned (fal documents that
//! rebuilding them from the endpoint path fails). Poll the status URL until `COMPLETED`, fetch the result, then
//! download the image from a fal media host. Jobs are reconcilable by id, which is why fal is the first hosted
//! provider where a lost response can be recovered without paying twice.

use std::sync::Arc;
use std::time::Duration;

use imagegen_core::config::ProviderConfig;
use imagegen_core::error::RetryClass;
use serde_json::{json, Value};

use crate::common::{basis, check_model, data_url, descriptor_from, parse_data_url, per_image_price, sniff_mime, status_error, ApiKey};
use crate::sizing::{choose_size, nearest_ratio, tier_size};
use crate::transport::{send, HostPolicy, HttpRequest, HttpTransport, Method};
use crate::types::{
    CallContext, CostEstimate, CostModel, ErrorKind, Funding, ImageProvider, ImageRequest, JobRef, ModelInfo, ProviderDescriptor, ProviderError,
    ProviderOutput, Reconciled, SizeRule,
};

/// Default queue host.
pub const DEFAULT_QUEUE_BASE: &str = "https://queue.fal.run";

/// Adapter state.
pub struct FalProvider {
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    queue_base: String,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
    poll_interval: Duration,
    max_polls: u32,
}

impl FalProvider {
    /// Build the adapter.
    pub fn new(cfg: ProviderConfig, key: Option<String>, transport: Arc<dyn HttpTransport>, policy: HostPolicy) -> Self {
        let queue_base = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_QUEUE_BASE.to_string()).trim_end_matches('/').to_string();
        FalProvider {
            desc: descriptor_from(include_str!("../data/fal.json")),
            cfg,
            key: key.map(ApiKey::new),
            queue_base,
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
        let key = self.key.as_ref().ok_or_else(|| ProviderError::not_configured("FAL_KEY is not set"))?;
        Ok(format!("Key {}", key.expose()))
    }

    fn endpoint_for(&self, model: &ModelInfo, req: &ImageRequest) -> String {
        if req.edit.is_some() || !req.refs.is_empty() {
            model.edit_model.clone().unwrap_or_else(|| model.id.clone())
        } else {
            model.id.clone()
        }
    }

    /// Submit body, public for wire-format tests.
    pub fn build_body(&self, req: &ImageRequest) -> Result<(String, Value), ProviderError> {
        let model = check_model(&self.desc, req)?;
        let endpoint = self.endpoint_for(model, req);
        let mut body = json!({"prompt": req.prompt, "output_format": "png"});
        match &model.sizes {
            SizeRule::AspectTiers { ratios, .. } => {
                let ratio = nearest_ratio(ratios, [req.width, req.height]).unwrap_or_else(|| "1:1".into());
                body["aspect_ratio"] = json!(ratio);
                body["num_images"] = json!(1);
            }
            rule => {
                let (w, h) = choose_size(rule, [req.width, req.height])?;
                body["image_size"] = json!({"width": w, "height": h});
            }
        }
        if let Some(seed) = req.seed {
            body["seed"] = json!(seed);
        }
        if req.edit.is_some() || !req.refs.is_empty() {
            let mut urls = Vec::new();
            if let Some(edit) = &req.edit {
                urls.push(json!(data_url("image/png", &edit.base_png)));
            }
            for r in &req.refs {
                urls.push(json!(data_url("image/png", &r.png)));
            }
            body["image_urls"] = Value::Array(urls);
        }
        Ok((endpoint, body))
    }

    fn json_get(&self, url: &str) -> Result<(u16, Value), ProviderError> {
        let resp = send(&*self.transport, &self.policy, &HttpRequest::get(url).header("Authorization", &self.auth()?))?;
        if resp.status >= 400 {
            return Err(status_error(resp.status, &resp.body, &self.secrets()));
        }
        let v = serde_json::from_slice(&resp.body).map_err(|e| {
            ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, format!("{url} did not return JSON: {e}"))
                .with_status(resp.status)
        })?;
        Ok((resp.status, v))
    }

    /// Fetch a finished result and download its image.
    fn fetch_result(&self, job: &JobRef, model: &ModelInfo, req_cost: Option<f64>) -> Result<ProviderOutput, ProviderError> {
        let url = job.response_url.clone().ok_or_else(|| {
            ProviderError::new(ErrorKind::MalformedResponse, RetryClass::Terminal, "the job has no response URL").with_job(job.clone())
        })?;
        let (_, v) = self.json_get(&url).map_err(|e| e.with_job(job.clone()))?;
        let image_url = v
            .pointer("/images/0/url")
            .or_else(|| v.pointer("/image/url"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, "the result has no images[0].url").with_job(job.clone())
            })?
            .to_string();
        let (mime, bytes) = if let Some((m, b)) = parse_data_url(&image_url) {
            (m, b)
        } else {
            // media downloads are public URLs; the key is not sent to them
            let resp = send(&*self.transport, &self.policy, &HttpRequest::get(&image_url)).map_err(|e| e.with_job(job.clone()))?;
            if resp.status != 200 {
                return Err(status_error(resp.status, &resp.body, &self.secrets()).with_job(job.clone()));
            }
            let mime = resp.header("content-type").map(|c| c.split(';').next().unwrap_or("").trim().to_string()).filter(|c| c.starts_with("image/"));
            let mime = mime.unwrap_or_else(|| sniff_mime(&resp.body).to_string());
            (mime, resp.body)
        };
        let _ = model;
        Ok(ProviderOutput {
            image: bytes,
            mime,
            provider_request_id: Some(job.id.clone()),
            usage: None,
            actual_usd: req_cost,
            usage_unknown: req_cost.is_none(),
            model_echo: Some(job.model.clone()),
            notes: vec![],
            job: Some(job.clone()),
        })
    }

    fn run(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        let auth = self.auth()?;
        let model = check_model(&self.desc, req)?.clone();
        let (endpoint, body) = self.build_body(req)?;
        let est = self.estimate(req).ok();
        let submit = HttpRequest::post_json(format!("{}/{}", self.queue_base, endpoint), &body).header("Authorization", &auth);
        let resp = send(&*self.transport, &self.policy, &submit)?;
        if resp.status != 200 && resp.status != 201 {
            return Err(status_error(resp.status, &resp.body, &self.secrets()));
        }
        let v: Value = serde_json::from_slice(&resp.body).map_err(|e| {
            ProviderError::new(
                ErrorKind::MalformedResponse,
                RetryClass::ReconcileFirst,
                format!("the submit response is not JSON ({e}); a job may exist"),
            )
        })?;
        let id = v
            .get("request_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, "the submit response has no request_id; a job may exist")
            })?
            .to_string();
        let url_of = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let job = JobRef {
            provider: "fal".into(),
            id,
            status_url: url_of("status_url"),
            response_url: url_of("response_url"),
            cancel_url: url_of("cancel_url"),
            model: endpoint,
        };
        ctx.notify(&job);
        let status_url = job.status_url.clone().ok_or_else(|| {
            ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, "the submit response has no status_url")
                .with_job(job.clone())
        })?;
        for _ in 0..self.max_polls {
            if ctx.cancel.is_cancelled() {
                let _ = self.cancel(&job, ctx);
                return Err(ProviderError::new(ErrorKind::Cancelled, RetryClass::Terminal, "cancelled").with_job(job));
            }
            let (_, st) = self.json_get(&status_url).map_err(|e| e.with_job(job.clone()))?;
            match st.get("status").and_then(Value::as_str).unwrap_or("") {
                "COMPLETED" => {
                    return self.fetch_result(&job, &model, est.as_ref().map(|e| e.usd));
                }
                "IN_QUEUE" | "IN_PROGRESS" => ctx.sleeper.sleep(self.poll_interval),
                other => {
                    return Err(ProviderError::new(ErrorKind::ServerError, RetryClass::ReconcileFirst, format!("the job reported status '{other}'"))
                        .with_job(job))
                }
            }
        }
        Err(ProviderError::new(
            ErrorKind::Transport,
            RetryClass::ReconcileFirst,
            "the job is still running after the polling budget; reconcile it by id instead of resubmitting",
        )
        .with_job(job))
    }
}

impl ImageProvider for FalProvider {
    fn id(&self) -> &str {
        "fal"
    }

    fn descriptor(&self) -> &ProviderDescriptor {
        &self.desc
    }

    fn is_configured(&self) -> bool {
        self.key.is_some()
    }

    fn default_model(&self, _editing: bool) -> String {
        self.cfg.model.clone().or_else(|| self.desc.recommended().map(|m| m.id.clone())).unwrap_or_else(|| "fal-ai/flux-2".into())
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
                let out_mp = (f64::from(w) * f64::from(h) / 1_048_576.0).ceil().max(1.0);
                let inputs = req.refs.len() + usize::from(req.edit.is_some());
                let usd = first + additional * ((out_mp - 1.0) + inputs as f64);
                Ok(CostEstimate {
                    usd: (usd * 1e6).round() / 1e6,
                    basis: basis("fal", &format!("{}: ${first} first megapixel + ${additional} per additional input or output megapixel; {out_mp} output MP and {inputs} input image(s) assumed 1 MP each", model.id)),
                    usage_unknown: false,
                    funding: Funding::UserKey,
                })
            }
            other => {
                let (usd, _) = per_image_price(other, None).ok_or_else(|| ProviderError::unsupported("no price in the table"))?;
                Ok(CostEstimate { usd, basis: basis("fal", &format!("{} per image", model.id)), usage_unknown: false, funding: Funding::UserKey })
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
        let (_, st) = self.json_get(status_url)?;
        match st.get("status").and_then(Value::as_str).unwrap_or("") {
            "COMPLETED" => {
                let model = self.desc.models.iter().find(|m| m.id == job.model || m.edit_model.as_deref() == Some(job.model.as_str()));
                let model = model.cloned().unwrap_or_else(|| self.desc.models[0].clone());
                Ok(Reconciled::Completed(Box::new(self.fetch_result(job, &model, None)?)))
            }
            "IN_QUEUE" | "IN_PROGRESS" => Ok(Reconciled::Pending),
            other => Ok(Reconciled::Failed(format!("the job reported status '{other}'"))),
        }
    }

    fn cancel(&self, job: &JobRef, _ctx: &CallContext) -> Result<(), ProviderError> {
        let url = job.cancel_url.clone().ok_or_else(|| ProviderError::unsupported("the job has no cancel URL"))?;
        let mut req = HttpRequest::get(url).header("Authorization", &self.auth()?);
        req.method = Method::Put;
        let resp = send(&*self.transport, &self.policy, &req)?;
        // 200 removed from the queue, 202 cancellation requested, 400 already completed (nothing left to cancel)
        match resp.status {
            200 | 202 | 400 => Ok(()),
            s => Err(status_error(s, &resp.body, &self.secrets())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FixtureTransport;
    use crate::types::{CancelToken, EditSpec, JobSink, ProviderRef, Sleeper};
    use imagegen_core::pixels::{encode_png, Rgba};
    use imagegen_core::refs::RefRole;
    use std::sync::Mutex;

    // SYNTHETIC fixtures: shapes follow fal's queue documentation (retrieved 2026-10-31), not recorded calls.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [1, 2, 3, 255])).unwrap()
    }

    fn policy() -> HostPolicy {
        HostPolicy::api(&["queue.fal.run", ".fal.media", ".fal.run"])
    }

    fn provider(fixture: Arc<FixtureTransport>, key: Option<&str>) -> FalProvider {
        FalProvider::new(ProviderConfig::default(), key.map(str::to_string), fixture, policy()).with_polling(Duration::from_millis(1), 5)
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

    fn submit_ok(fixture: &FixtureTransport, endpoint_path: &str) {
        let body = json!({
            "request_id": "req-synthetic-1",
            "status_url": format!("https://queue.fal.run/{endpoint_path}/requests/req-synthetic-1/status"),
            "response_url": format!("https://queue.fal.run/{endpoint_path}/requests/req-synthetic-1"),
            "cancel_url": format!("https://queue.fal.run/{endpoint_path}/requests/req-synthetic-1/cancel"),
            "queue_position": 0
        });
        fixture.respond(Method::Post, "queue.fal.run/fal-ai/", 200, &[], serde_json::to_vec(&body).unwrap());
    }

    fn finish(fixture: &FixtureTransport, endpoint_path: &str) {
        fixture.respond(Method::Get, "/status", 200, &[], r#"{"status":"IN_QUEUE"}"#);
        fixture.respond(Method::Get, "/status", 200, &[], r#"{"status":"IN_PROGRESS"}"#);
        fixture.respond(Method::Get, "/status", 200, &[], r#"{"status":"COMPLETED"}"#);
        let result = json!({"images": [{"url": "https://v3.fal.media/files/synthetic.png", "content_type": "image/png", "width": 928, "height": 816}], "seed": 7});
        fixture.respond(Method::Get, &format!("${endpoint_path}/requests/req-synthetic-1"), 200, &[], serde_json::to_vec(&result).unwrap());
        fixture.respond(Method::Get, "v3.fal.media", 200, &[("content-type", "image/png")], png());
    }

    #[test]
    fn generate_body_uses_image_size_and_the_cheap_default_model() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("fal-test-000000"));
        assert_eq!(p.default_model(false), "fal-ai/flux-2");
        let (endpoint, body) = p.build_body(&request("fal-ai/flux-2")).unwrap();
        assert_eq!(endpoint, "fal-ai/flux-2");
        assert_eq!(body["output_format"], "png");
        assert_eq!(body["seed"], 7);
        let w = body["image_size"]["width"].as_u64().unwrap();
        let h = body["image_size"]["height"].as_u64().unwrap();
        assert!(w % 16 == 0 && h % 16 == 0 && w * h >= 262_144);
        assert!(body.get("image_urls").is_none());
    }

    #[test]
    fn references_switch_to_the_edit_endpoint_and_travel_as_data_uris() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("fal-test-000000"));
        let mut r = request("fal-ai/flux-2-pro");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        r.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }];
        let (endpoint, body) = p.build_body(&r).unwrap();
        assert_eq!(endpoint, "fal-ai/flux-2-pro/edit");
        let urls = body["image_urls"].as_array().unwrap();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].as_str().unwrap().starts_with("data:image/png;base64,"));
        // a reference-guided generation also uses the edit endpoint
        let mut g = request("fal-ai/flux-2");
        g.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }];
        assert_eq!(p.build_body(&g).unwrap().0, "fal-ai/flux-2/edit");
    }

    #[test]
    fn full_queue_flow_polls_downloads_and_reports_the_job() {
        let fixture = Arc::new(FixtureTransport::new());
        submit_ok(&fixture, "fal-ai/flux-2");
        finish(&fixture, "fal-ai/flux-2");
        let p = provider(fixture.clone(), Some("fal-test-000000"));
        let sleeper = Arc::new(CountingSleeper::default());
        let jobs = Arc::new(Jobs::default());
        let ctx = CallContext { cancel: CancelToken::new(), sleeper: sleeper.clone(), jobs: Some(jobs.clone()) };
        let out = p.generate(&request("fal-ai/flux-2"), &ctx).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!(out.provider_request_id.as_deref(), Some("req-synthetic-1"));
        assert_eq!(*sleeper.0.lock().unwrap(), 2, "two non-final polls");
        assert_eq!(jobs.0.lock().unwrap().len(), 1, "the job is announced before the wait so a ledger can record it");
        assert_eq!(out.job.as_ref().unwrap().id, "req-synthetic-1");
        assert!(out.actual_usd.unwrap() > 0.0);
        let reqs = fixture.requests();
        let submit = &reqs[0];
        assert!(submit.headers.iter().any(|(k, v)| k == "Authorization" && v == "Key fal-test-000000"));
        let download = reqs.iter().find(|r| r.url.contains("v3.fal.media")).unwrap();
        assert!(!download.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization")), "the key must not go to the media host");
    }

    #[test]
    fn cancellation_calls_the_cancel_url_and_keeps_the_job() {
        let fixture = Arc::new(FixtureTransport::new());
        submit_ok(&fixture, "fal-ai/flux-2");
        fixture.respond(Method::Get, "/status", 200, &[], r#"{"status":"IN_PROGRESS"}"#);
        fixture.respond(Method::Put, "/cancel", 202, &[], r#"{"status":"CANCELLATION_REQUESTED"}"#);
        let p = provider(fixture.clone(), Some("fal-test-000000"));
        let ctx = CallContext::new();
        ctx.cancel.cancel();
        let err = p.generate(&request("fal-ai/flux-2"), &ctx).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Cancelled);
        assert_eq!(err.job.as_ref().unwrap().id, "req-synthetic-1");
        assert!(fixture.requests().iter().any(|r| r.method == Method::Put && r.url.ends_with("/cancel")));
    }

    #[test]
    fn running_out_of_polls_demands_reconciliation_not_resubmission() {
        let fixture = Arc::new(FixtureTransport::new());
        submit_ok(&fixture, "fal-ai/flux-2");
        fixture.respond(Method::Get, "/status", 200, &[], r#"{"status":"IN_PROGRESS"}"#);
        let p = provider(fixture, Some("fal-test-000000"));
        let sleeper = Arc::new(CountingSleeper::default());
        let ctx = CallContext { cancel: CancelToken::new(), sleeper, jobs: None };
        let err = p.generate(&request("fal-ai/flux-2"), &ctx).unwrap_err();
        assert_eq!(err.retry, RetryClass::ReconcileFirst);
        assert!(err.job.is_some());
    }

    #[test]
    fn a_lost_response_is_reconciled_by_job_id() {
        let fixture = Arc::new(FixtureTransport::new());
        finish(&fixture, "fal-ai/flux-2");
        let p = provider(fixture.clone(), Some("fal-test-000000"));
        let job = JobRef {
            provider: "fal".into(),
            id: "req-synthetic-1".into(),
            status_url: Some("https://queue.fal.run/fal-ai/flux-2/requests/req-synthetic-1/status".into()),
            response_url: Some("https://queue.fal.run/fal-ai/flux-2/requests/req-synthetic-1".into()),
            cancel_url: None,
            model: "fal-ai/flux-2".into(),
        };
        // the fixture's status route steps IN_QUEUE, IN_PROGRESS, COMPLETED
        assert!(matches!(p.reconcile(&job, &CallContext::new()).unwrap(), Reconciled::Pending));
        assert!(matches!(p.reconcile(&job, &CallContext::new()).unwrap(), Reconciled::Pending));
        match p.reconcile(&job, &CallContext::new()).unwrap() {
            Reconciled::Completed(out) => assert_eq!(out.image, png()),
            other => panic!("expected completion, got {other:?}"),
        }
        let none = JobRef { status_url: None, ..job };
        assert!(matches!(p.reconcile(&none, &CallContext::new()).unwrap(), Reconciled::Unknown));
    }

    #[test]
    fn media_urls_outside_fal_hosts_are_blocked() {
        let fixture = Arc::new(FixtureTransport::new());
        submit_ok(&fixture, "fal-ai/flux-2");
        fixture.respond(Method::Get, "/status", 200, &[], r#"{"status":"COMPLETED"}"#);
        let result = json!({"images": [{"url": "https://evil.example/steal.png"}]});
        fixture.respond(Method::Get, "$requests/req-synthetic-1", 200, &[], serde_json::to_vec(&result).unwrap());
        let p = provider(fixture.clone(), Some("fal-test-000000"));
        let err = p.generate(&request("fal-ai/flux-2"), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Blocked);
        assert!(!fixture.requests().iter().any(|r| r.url.contains("evil.example")));
    }

    #[test]
    fn status_urls_pointing_elsewhere_are_blocked_too() {
        let fixture = Arc::new(FixtureTransport::new());
        let body = json!({"request_id": "r", "status_url": "https://evil.example/status", "response_url": "https://evil.example/r", "cancel_url": "https://evil.example/c"});
        fixture.respond(Method::Post, "queue.fal.run", 200, &[], serde_json::to_vec(&body).unwrap());
        let p = provider(fixture.clone(), Some("fal-test-000000"));
        let err = p.generate(&request("fal-ai/flux-2"), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Blocked, "the key must not follow a URL returned by the server to another host");
        assert!(!fixture.requests().iter().any(|r| r.url.contains("evil.example")));
    }

    #[test]
    fn estimates_follow_the_documented_megapixel_rules() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("fal-test-000000"));
        let gen_pro = p.estimate(&request("fal-ai/flux-2-pro")).unwrap();
        assert!((gen_pro.usd - 0.03).abs() < 1e-9, "{gen_pro:?}");
        let mut edit = request("fal-ai/flux-2-pro");
        edit.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let e = p.estimate(&edit).unwrap();
        assert!((e.usd - 0.045).abs() < 1e-9, "one 1 MP input adds $0.015: {e:?}");
        let flux2 = p.estimate(&request("fal-ai/flux-2")).unwrap();
        assert!((flux2.usd - 0.012).abs() < 1e-9);
        let nb = p.estimate(&request("fal-ai/nano-banana")).unwrap();
        assert!((nb.usd - 0.0398).abs() < 1e-9);
    }

    #[test]
    fn errors_and_missing_key() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "queue.fal.run", 401, &[], r#"{"detail":"Invalid key fal-test-secret-77"}"#);
        let p = provider(fixture, Some("fal-test-secret-77"));
        let err = p.generate(&request("fal-ai/flux-2"), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Auth);
        assert!(!err.message.contains("fal-test-secret-77"));
        let p = provider(Arc::new(FixtureTransport::new()), None);
        assert_eq!(p.generate(&request("fal-ai/flux-2"), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);
        assert_eq!(p.generate(&request("fal-ai/flux/dev"), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);
    }
}
