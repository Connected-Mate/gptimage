//! Leonardo.Ai (`leonardo`): create a v2 generation, poll it, download from the Leonardo CDN.
//!
//! NOT LIVE-VERIFIED. Built from Leonardo's API reference and guides read on 2026-11-02. No call was made to
//! Leonardo, and every test fixture below is a "documented shape, not recorded" response.
//!
//! Wire format:
//! - `POST {base}/api/rest/v2/generations` with `Authorization: Bearer $LEONARDO_API_KEY` and a body of
//!   `{"model": "<id>", "parameters": {"prompt", "width", "height", "quantity", ...}, "public": false}`.
//!   The answer is `{generationId, apiCreditCost}`. The older v1 answer shape `sdGenerationJob.generationId`
//!   is also accepted.
//! - `GET {base}/api/rest/v1/generations/{id}` returns `generations_by_pk` with `status` (`PENDING`, `COMPLETE`,
//!   `FAILED`) and `generated_images[].url`. An empty list means still pending.
//! - Images are downloaded from `cdn.leonardo.ai`. The API has no documented cancel call (DELETE removes the
//!   generation record), so a cancelled wait just stops polling.
//! - Reference images and edits need Leonardo's separate image upload flow, which is not implemented here.

use std::sync::Arc;
use std::time::Duration;

use imagegen_core::config::{Config, EnvSource, ProviderConfig};
use imagegen_core::error::RetryClass;
use serde_json::{json, Value};

use crate::common::{check_model, descriptor_from, sniff_mime, status_error, ApiKey};
use crate::registry::{policy_for, read_key, ProviderRegistry};
use crate::sizing::choose_size;
use crate::transport::{redact, send, HostPolicy, HttpRequest, HttpTransport};
use crate::types::{
    CallContext, CostEstimate, ErrorKind, Funding, ImageProvider, ImageRequest, JobRef, ProviderDescriptor, ProviderError, ProviderOutput, Reconciled,
};

/// Default API host.
pub const DEFAULT_API_BASE: &str = "https://cloud.leonardo.ai";

/// Adapter state.
pub struct LeonardoProvider {
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

impl LeonardoProvider {
    /// Build the adapter.
    pub fn new(cfg: ProviderConfig, key: Option<String>, transport: Arc<dyn HttpTransport>, policy: HostPolicy) -> Self {
        let api_base = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_API_BASE.to_string()).trim_end_matches('/').to_string();
        LeonardoProvider {
            desc: descriptor_from(include_str!("../data/leonardo.json")),
            cfg,
            key: key.map(ApiKey::new),
            api_base,
            transport,
            policy,
            poll_interval: Duration::from_millis(2000),
            max_polls: 300,
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
        let key = self.key.as_ref().ok_or_else(|| ProviderError::not_configured("LEONARDO_API_KEY is not set"))?;
        Ok(format!("Bearer {}", key.expose()))
    }

    fn error_for(&self, status: u16, body: &[u8]) -> ProviderError {
        let lower = String::from_utf8_lossy(body).to_ascii_lowercase();
        if (status == 400 || status == 422) && (lower.contains("moderat") || lower.contains("content filter") || lower.contains("nsfw")) {
            let msg = crate::transport::error_message(body, &self.secrets());
            return ProviderError::new(ErrorKind::ModerationBlocked, RetryClass::Terminal, format!("HTTP {status}: {msg}")).with_status(status);
        }
        status_error(status, body, &self.secrets())
    }

    /// Request body, public for wire-format tests. Returns the model id and the JSON body.
    pub fn build_body(&self, req: &ImageRequest) -> Result<(String, Value), ProviderError> {
        let model = check_model(&self.desc, req)?;
        let (w, h) = choose_size(&model.sizes, [req.width, req.height])?;
        let mut params = json!({"prompt": req.prompt, "width": w, "height": h, "quantity": 1});
        if let Some(seed) = req.seed {
            params["seed"] = json!(seed);
        }
        // `public: false` keeps the output out of Leonardo's community feed
        Ok((model.id.clone(), json!({"model": model.id, "parameters": params, "public": false})))
    }

    fn step(&self, v: &Value, job: &JobRef, credits: Option<i64>) -> Result<Step, ProviderError> {
        let rec = v.get("generations_by_pk").unwrap_or(&Value::Null);
        let status = rec.get("status").and_then(Value::as_str).unwrap_or("");
        let image_url = rec.pointer("/generated_images/0/url").and_then(Value::as_str);
        match status {
            "FAILED" => Ok(Step::Failed(
                ProviderError::new(ErrorKind::ServerError, RetryClass::ReconcileFirst, "Leonardo reported the generation as FAILED")
                    .with_job(job.clone()),
            )),
            "COMPLETE" => {
                let url = image_url.ok_or_else(|| {
                    ProviderError::new(
                        ErrorKind::MalformedResponse,
                        RetryClass::SafeSameInputs,
                        "the COMPLETE generation has no generated_images[0].url",
                    )
                    .with_job(job.clone())
                })?;
                // CDN URLs are public: the key is not sent to them
                let resp = send(&*self.transport, &self.policy, &HttpRequest::get(url)).map_err(|e| e.with_job(job.clone()))?;
                if resp.status != 200 {
                    return Err(self.error_for(resp.status, &resp.body).with_job(job.clone()));
                }
                let mime =
                    resp.header("content-type").map(|c| c.split(';').next().unwrap_or("").trim().to_string()).filter(|c| c.starts_with("image/"));
                let mime = mime.unwrap_or_else(|| sniff_mime(&resp.body).to_string());
                Ok(Step::Done(Box::new(ProviderOutput {
                    image: resp.body,
                    mime,
                    provider_request_id: Some(job.id.clone()),
                    usage: credits.map(|c| json!({"api_credit_cost": c})),
                    actual_usd: None,
                    usage_unknown: true,
                    model_echo: Some(job.model.clone()),
                    notes: credits.map(|c| format!("Leonardo reported an API credit cost of {c}")).into_iter().collect(),
                    job: Some(job.clone()),
                })))
            }
            // PENDING, or an empty image list, means still running
            "PENDING" | "" => Ok(Step::Pending),
            other => Ok(Step::Failed(
                ProviderError::new(
                    ErrorKind::MalformedResponse,
                    RetryClass::ReconcileFirst,
                    format!("the generation reported unknown status '{other}'"),
                )
                .with_job(job.clone()),
            )),
        }
    }

    fn status_url(&self, id: &str) -> String {
        format!("{}/api/rest/v1/generations/{id}", self.api_base)
    }

    fn json_get(&self, url: &str) -> Result<Value, ProviderError> {
        let resp = send(&*self.transport, &self.policy, &HttpRequest::get(url).header("Authorization", &self.auth()?))?;
        if resp.status >= 400 {
            return Err(self.error_for(resp.status, &resp.body));
        }
        serde_json::from_slice(&resp.body).map_err(|e| {
            ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, format!("{url} did not return JSON: {e}"))
                .with_status(resp.status)
        })
    }

    fn run(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        let auth = self.auth()?;
        let (model_id, body) = self.build_body(req)?;
        let submit = HttpRequest::post_json(format!("{}/api/rest/v2/generations", self.api_base), &body).header("Authorization", &auth);
        let resp = send(&*self.transport, &self.policy, &submit)?;
        if resp.status != 200 && resp.status != 201 {
            return Err(self.error_for(resp.status, &resp.body));
        }
        let v: Value = serde_json::from_slice(&resp.body).map_err(|e| {
            ProviderError::new(
                ErrorKind::MalformedResponse,
                RetryClass::ReconcileFirst,
                format!("the create response is not JSON ({e}); a generation may exist"),
            )
        })?;
        let id = v
            .get("generationId")
            .or_else(|| v.pointer("/sdGenerationJob/generationId"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                let detail = redact(&v.to_string(), &self.secrets());
                let detail: String = detail.chars().take(200).collect();
                ProviderError::new(
                    ErrorKind::MalformedResponse,
                    RetryClass::ReconcileFirst,
                    format!("the create response has no generationId ({detail}); a generation may exist"),
                )
            })?
            .to_string();
        let credits = v.get("apiCreditCost").and_then(Value::as_i64);
        let status_url = self.status_url(&id);
        let job =
            JobRef { provider: "leonardo".into(), id, status_url: Some(status_url.clone()), response_url: None, cancel_url: None, model: model_id };
        ctx.notify(&job);
        for _ in 0..self.max_polls {
            if ctx.cancel.is_cancelled() {
                return Err(ProviderError::new(
                    ErrorKind::Cancelled,
                    RetryClass::Terminal,
                    "cancelled (Leonardo has no cancel call, the generation may still finish and use credits)",
                )
                .with_job(job));
            }
            let st = self.json_get(&status_url).map_err(|e| e.with_job(job.clone()))?;
            match self.step(&st, &job, credits)? {
                Step::Done(out) => return Ok(*out),
                Step::Failed(e) => return Err(e),
                Step::Pending => ctx.sleeper.sleep(self.poll_interval),
            }
        }
        Err(ProviderError::new(
            ErrorKind::Transport,
            RetryClass::ReconcileFirst,
            "the generation is still running after the polling budget; reconcile it by id instead of resubmitting",
        )
        .with_job(job))
    }
}

impl ImageProvider for LeonardoProvider {
    fn id(&self) -> &str {
        "leonardo"
    }

    fn descriptor(&self) -> &ProviderDescriptor {
        &self.desc
    }

    fn is_configured(&self) -> bool {
        self.key.is_some()
    }

    fn default_model(&self, _editing: bool) -> String {
        self.cfg.model.clone().or_else(|| self.desc.recommended().map(|m| m.id.clone())).unwrap_or_else(|| "lucid-origin".into())
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        let model = check_model(&self.desc, req)?;
        choose_size(&model.sizes, [req.width, req.height])
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        let model = check_model(&self.desc, req)?;
        Ok(CostEstimate {
            usd: 0.0,
            basis: format!(
                "leonardo table retrieved {}: {} is billed in API credits and no USD price per image or credit was found in the documentation, so no figure is claimed (the create response's apiCreditCost is the real charge)",
                self.desc.retrieved, model.id
            ),
            usage_unknown: true,
            funding: Funding::UserKey,
        })
    }

    fn generate(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.edit.is_some() {
            return Err(ProviderError::unsupported("generate was called with an edit request"));
        }
        self.run(req, ctx)
    }

    fn edit(&self, _req: &ImageRequest, _ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        Err(ProviderError::unsupported(
            "Leonardo editing and reference images need its separate image upload flow, which this adapter does not implement",
        ))
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
}

/// Register this module's providers unless the project config disables them.
pub fn register(reg: &mut ProviderRegistry, config: &Config, env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) {
    let cfg = config.provider("leonardo");
    if cfg.enabled == Some(false) {
        return;
    }
    match policy_for(&["cloud.leonardo.ai", "cdn.leonardo.ai"], cfg.base_url.as_deref(), env) {
        Ok(policy) => {
            let key = read_key("leonardo", &cfg, env);
            reg.insert(Arc::new(LeonardoProvider::new(cfg, key, transport, policy)));
        }
        Err(why) => reg.problem("leonardo", why),
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

    // DOCUMENTED SHAPE, NOT RECORDED: every response below follows Leonardo's documented schemas (read
    // 2026-11-02). Nothing here came from a real Leonardo call.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [1, 2, 3, 255])).unwrap()
    }

    fn policy() -> HostPolicy {
        HostPolicy::api(&["cloud.leonardo.ai", "cdn.leonardo.ai"])
    }

    fn provider(fixture: Arc<FixtureTransport>, key: Option<&str>) -> LeonardoProvider {
        LeonardoProvider::new(ProviderConfig::default(), key.map(str::to_string), fixture, policy()).with_polling(Duration::from_millis(1), 5)
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

    fn generation(status: &str, urls: &[&str]) -> Vec<u8> {
        let imgs: Vec<Value> = urls.iter().map(|u| json!({"url": u, "id": "img-doc-1"})).collect();
        serde_json::to_vec(&json!({"generations_by_pk": {"id": "gen-doc-1", "status": status, "generated_images": imgs}})).unwrap()
    }

    #[test]
    fn body_follows_the_documented_v2_shape() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("leo-test-000000"));
        assert_eq!(p.default_model(false), "lucid-origin");
        let (model, body) = p.build_body(&request("lucid-origin")).unwrap();
        assert_eq!(model, "lucid-origin");
        assert_eq!(body["model"], "lucid-origin");
        assert_eq!(body["public"], false);
        let params = &body["parameters"];
        assert_eq!(params["prompt"], "an iron wall");
        assert_eq!(params["quantity"], 1);
        assert_eq!(params["seed"], 7);
        let (w, h) = (params["width"].as_u64().unwrap(), params["height"].as_u64().unwrap());
        assert!(w % 8 == 0 && h % 8 == 0 && w * h >= 262_144 && w <= 3840 && h <= 3616, "{w}x{h}");
        assert_eq!(p.prepare(&request("lucid-origin")).unwrap(), (w as u32, h as u32));
    }

    #[test]
    fn edits_and_references_are_refused_before_any_request() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider(fixture.clone(), Some("leo-test-000000"));
        let mut r = request("lucid-origin");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        assert_eq!(p.edit(&r, &CallContext::new()).unwrap_err().kind, ErrorKind::Unsupported);
        assert_eq!(p.generate(&r, &CallContext::new()).unwrap_err().kind, ErrorKind::Unsupported);
        let mut g = request("lucid-origin");
        g.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }];
        assert_eq!(p.generate(&g, &CallContext::new()).unwrap_err().kind, ErrorKind::Unsupported);
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn full_flow_creates_polls_and_downloads_from_the_cdn_without_the_key() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/api/rest/v2/generations", 200, &[], r#"{"generationId":"gen-doc-1","apiCreditCost":12}"#);
        fixture.respond(Method::Get, "/api/rest/v1/generations/gen-doc-1", 200, &[], generation("PENDING", &[]));
        fixture.respond(
            Method::Get,
            "/api/rest/v1/generations/gen-doc-1",
            200,
            &[],
            generation("COMPLETE", &["https://cdn.leonardo.ai/users/doc/generations/gen-doc-1/out.png"]),
        );
        fixture.respond(Method::Get, "cdn.leonardo.ai", 200, &[("content-type", "image/png")], png());
        let p = provider(fixture.clone(), Some("leo-test-000000"));
        let sleeper = Arc::new(CountingSleeper::default());
        let jobs = Arc::new(Jobs::default());
        let ctx = CallContext { cancel: CancelToken::new(), sleeper: sleeper.clone(), jobs: Some(jobs.clone()) };
        let out = p.generate(&request("lucid-origin"), &ctx).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!(*sleeper.0.lock().unwrap(), 1);
        assert_eq!(jobs.0.lock().unwrap().len(), 1);
        assert_eq!(out.job.as_ref().unwrap().id, "gen-doc-1");
        assert!(out.usage_unknown && out.actual_usd.is_none(), "no USD price is claimed");
        assert_eq!(out.usage.as_ref().unwrap()["api_credit_cost"], 12);
        let reqs = fixture.requests();
        assert_eq!(reqs[0].url, "https://cloud.leonardo.ai/api/rest/v2/generations");
        assert!(reqs[0].headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer leo-test-000000"));
        let dl = reqs.iter().find(|r| r.url.contains("cdn.leonardo.ai")).unwrap();
        assert!(dl.headers.is_empty(), "the key must not go to the CDN");
    }

    #[test]
    fn the_v1_create_answer_shape_is_accepted_and_failures_are_reported() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/api/rest/v2/generations", 200, &[], r#"{"sdGenerationJob":{"generationId":"gen-doc-1"}}"#);
        fixture.respond(Method::Get, "/generations/gen-doc-1", 200, &[], generation("FAILED", &[]));
        let p = provider(fixture, Some("leo-test-000000"));
        let err = p.generate(&request("lucid-origin"), &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.retry), (ErrorKind::ServerError, RetryClass::ReconcileFirst));
        assert_eq!(err.job.as_ref().unwrap().id, "gen-doc-1");
        // an answer with no id (for example an error list with status 200) is not mistaken for success
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/api/rest/v2/generations", 200, &[], r#"[{"message":"internal"}]"#);
        let p = provider(fixture, Some("leo-test-000000"));
        let err = p.generate(&request("lucid-origin"), &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.retry), (ErrorKind::MalformedResponse, RetryClass::ReconcileFirst));
    }

    #[test]
    fn cancel_stops_polling_and_keeps_the_job() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/api/rest/v2/generations", 200, &[], r#"{"generationId":"gen-doc-1"}"#);
        let p = provider(fixture.clone(), Some("leo-test-000000"));
        let ctx = CallContext::new();
        ctx.cancel.cancel();
        let err = p.generate(&request("lucid-origin"), &ctx).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Cancelled);
        assert_eq!(err.job.as_ref().unwrap().id, "gen-doc-1");
        assert_eq!(fixture.requests().len(), 1);
        assert_eq!(p.cancel(&err.job.unwrap(), &CallContext::new()).unwrap_err().kind, ErrorKind::Unsupported);
    }

    #[test]
    fn running_out_of_polls_demands_reconciliation() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/api/rest/v2/generations", 200, &[], r#"{"generationId":"gen-doc-1"}"#);
        fixture.respond(Method::Get, "/generations/gen-doc-1", 200, &[], generation("PENDING", &[]));
        let p = provider(fixture, Some("leo-test-000000"));
        let ctx = CallContext { cancel: CancelToken::new(), sleeper: Arc::new(CountingSleeper::default()), jobs: None };
        let err = p.generate(&request("lucid-origin"), &ctx).unwrap_err();
        assert_eq!(err.retry, RetryClass::ReconcileFirst);
        assert!(err.job.is_some());
    }

    #[test]
    fn a_lost_response_is_reconciled_by_generation_id() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Get, "/generations/gen-doc-1", 200, &[], generation("PENDING", &[]));
        fixture.respond(Method::Get, "/generations/gen-doc-1", 200, &[], generation("COMPLETE", &["https://cdn.leonardo.ai/doc/out.png"]));
        fixture.respond(Method::Get, "cdn.leonardo.ai", 200, &[], png());
        let p = provider(fixture, Some("leo-test-000000"));
        let job = JobRef {
            provider: "leonardo".into(),
            id: "gen-doc-1".into(),
            status_url: Some("https://cloud.leonardo.ai/api/rest/v1/generations/gen-doc-1".into()),
            response_url: None,
            cancel_url: None,
            model: "lucid-origin".into(),
        };
        assert!(matches!(p.reconcile(&job, &CallContext::new()).unwrap(), Reconciled::Pending));
        match p.reconcile(&job, &CallContext::new()).unwrap() {
            Reconciled::Completed(out) => assert_eq!((out.image, out.mime.as_str()), (png(), "image/png")),
            other => panic!("expected completion, got {other:?}"),
        }
        let none = JobRef { status_url: None, ..job };
        assert!(matches!(p.reconcile(&none, &CallContext::new()).unwrap(), Reconciled::Unknown));
    }

    #[test]
    fn image_urls_outside_the_leonardo_cdn_are_blocked() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/api/rest/v2/generations", 200, &[], r#"{"generationId":"gen-doc-1"}"#);
        fixture.respond(Method::Get, "/generations/gen-doc-1", 200, &[], generation("COMPLETE", &["https://evil.example/steal.png"]));
        let p = provider(fixture.clone(), Some("leo-test-000000"));
        assert_eq!(p.generate(&request("lucid-origin"), &CallContext::new()).unwrap_err().kind, ErrorKind::Blocked);
        assert!(!fixture.requests().iter().any(|r| r.url.contains("evil.example")));
    }

    #[test]
    fn estimate_claims_no_price() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("leo-test-000000"));
        let e = p.estimate(&request("lucid-origin")).unwrap();
        assert_eq!((e.usd, e.usage_unknown, e.funding), (0.0, true, Funding::UserKey));
        assert!(e.basis.contains("no figure is claimed") && e.basis.contains("2026-11-02"));
    }

    #[test]
    fn errors_redaction_moderation_and_missing_key() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/api/rest/v2/generations", 401, &[], r#"{"error":"Invalid key leo-test-secret-77"}"#);
        let p = provider(fixture, Some("leo-test-secret-77"));
        let err = p.generate(&request("lucid-origin"), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Auth);
        assert!(!err.message.contains("leo-test-secret-77"));
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/api/rest/v2/generations", 400, &[], r#"{"error":"Content moderation filter triggered"}"#);
        let p = provider(fixture, Some("leo-test-000000"));
        assert_eq!(p.generate(&request("lucid-origin"), &CallContext::new()).unwrap_err().kind, ErrorKind::ModerationBlocked);
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/api/rest/v2/generations", 429, &[], "{}");
        let p = provider(fixture, Some("leo-test-000000"));
        assert_eq!(p.generate(&request("lucid-origin"), &CallContext::new()).unwrap_err().kind, ErrorKind::RateLimited);
        let p = provider(Arc::new(FixtureTransport::new()), None);
        assert!(!p.is_configured());
        assert_eq!(p.generate(&request("lucid-origin"), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);
        let p = provider(Arc::new(FixtureTransport::new()), Some("leo-test-000000"));
        assert_eq!(p.generate(&request("phoenix-1.0"), &CallContext::new()).unwrap_err().kind, ErrorKind::Unsupported);
    }

    #[test]
    fn register_inserts_even_without_a_key_and_honours_enabled_false() {
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::default(), Arc::new(FixtureTransport::new()));
        assert!(!reg.get("leonardo").expect("registered without a key").is_configured());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::from_pairs(&[("LEONARDO_API_KEY", "leo-test-1")]), Arc::new(FixtureTransport::new()));
        assert!(reg.get("leonardo").unwrap().is_configured());
        let off = Config::parse("[providers.leonardo]\nenabled = false\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &off, &MapEnv::default(), Arc::new(FixtureTransport::new()));
        assert!(reg.get("leonardo").is_none());
    }

    #[test]
    fn the_table_has_one_recommended_model_and_is_marked_not_live_verified() {
        let d = descriptor_from(include_str!("../data/leonardo.json"));
        assert_eq!(d.models.iter().filter(|m| m.recommended).count(), 1);
        assert!(d.terms_note.contains("Not live-verified") && d.models.iter().all(|m| m.notes.contains("Not live-verified")));
        let raw: Value = serde_json::from_str(include_str!("../data/leonardo.json")).unwrap();
        assert_eq!(raw["live_verified"], false);
    }
}
