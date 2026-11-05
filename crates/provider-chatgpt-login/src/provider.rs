//! The provider: refresh the token, call the undocumented responses endpoint, read the image out of the stream.
//!
//! Rust port of Connected-Mate/gptimage `src/codex.js` (MIT, commit dfb4948), plus the fixed model and reasoning
//! effort (maintainer decision, 2026-11-01): every request asks for gpt-6-luna at high effort, and a response that
//! does not report both is refused, so a silent downgrade can never produce an asset.

use std::path::PathBuf;
use std::sync::Arc;

use imagegen_core::config::EnvSource;
use imagegen_core::error::RetryClass;
use imagegen_core::hashing::{Clock, SystemClock};
use imagegen_providers::common::{b64, b64_decode, data_url, status_error};
use imagegen_providers::sizing::choose_size;
use imagegen_providers::transport::{send, HostPolicy, HttpRequest, HttpTransport};
use imagegen_providers::types::{
    AlphaSupport, CallContext, CostEstimate, CostModel, ErrorKind, Funding, ImageProvider, ImageRequest, ModelInfo, ModelStatus, ProviderDescriptor,
    ProviderError, ProviderOutput, SizeRule, TABLES_RETRIEVED,
};
use serde_json::{json, Value};

use crate::oauth::{self, account_id, expiry_ms, parse_tokens, refresh_request};
use crate::route::{self, Route, RouteGuard, RouteState, RATE_LIMIT_MAX_RETRIES};
use crate::store::{CredentialStore, Credentials};
use crate::{BACKEND_URL, EFFORT_BACKEND_DEFAULT, EFFORT_LADDER, EFFORT_POLICY, EFFORT_REFUSAL_MEMORY_SECS, HOSTS, MODEL, ORIGINATOR};

/// Environment variable naming a folder where raw backend responses are saved, to build recorded-real fixtures.
pub const RECORD_ENV: &str = "HOI4_IMAGEGEN_RECORD_RESPONSES";

/// Start of a parse error for an image the provider's safety system refused.
pub const MODERATION_PREFIX: &str = "moderation_blocked: ";
/// Start of a parse error for an image call that failed for another reason.
pub const FAILED_PREFIX: &str = "image_generation_failed: ";

/// Refresh this long before the token expires.
const EXPIRY_MARGIN_MS: u64 = 60_000;

const INSTRUCTIONS: &str = "You are an image generation assistant running inside the Codex backend. Always satisfy the request by invoking the image_generation tool exactly once. Do not respond with text only.";

/// The provider.
pub struct ChatGptLoginProvider {
    desc: ProviderDescriptor,
    store: CredentialStore,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
    clock: Arc<dyn Clock>,
    model: String,
    record_dir: Option<PathBuf>,
    route: Route,
}

/// What `login-status --check` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The token endpoint accepted a refresh: the sign-in is alive and the new tokens are saved.
    Accepted,
    /// The token is not near expiry, so no request was made. There is no cheap, safe backend call to test it with.
    NotChecked(String),
}

impl ChatGptLoginProvider {
    /// Build from the environment: the operating system credential store (see [`CredentialStore::for_environment`]),
    /// and the number of requests at once from [`route::CONCURRENCY_ENV`] or the default.
    pub fn from_environment(env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) -> Result<Self, String> {
        Self::from_environment_with(env, transport, None)
    }

    /// [`ChatGptLoginProvider::from_environment`] with `[providers.chatgpt_login] concurrency` from the project config.
    /// The environment variable wins over it.
    pub fn from_environment_with(env: &dyn EnvSource, transport: Arc<dyn HttpTransport>, concurrency: Option<i64>) -> Result<Self, String> {
        let mut p = Self::new(CredentialStore::for_environment(env)?, transport, Arc::new(SystemClock))
            .with_concurrency(route::configured_concurrency(env, concurrency));
        p.record_dir = env.get(RECORD_ENV).map(PathBuf::from);
        Ok(p)
    }

    /// The same provider with `n` requests at once on the machine (clamped to 1..=6).
    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.route = self.route.with_concurrency(n);
        self
    }

    /// The configured number of requests at once.
    pub fn concurrency(&self) -> usize {
        self.route.concurrency()
    }

    /// Build with explicit parts (tests).
    pub fn new(store: CredentialStore, transport: Arc<dyn HttpTransport>, clock: Arc<dyn Clock>) -> Self {
        let model = MODEL.to_string();
        let desc = ProviderDescriptor {
            id: "chatgpt_login".into(),
            display_name: "ChatGPT login (your own plan; local only)".into(),
            doc_url: "docs/chatgpt-login.md".into(),
            retrieved: TABLES_RETRIEVED.into(),
            env_vars: vec![],
            hosts: HOSTS.iter().map(|h| h.to_string()).collect(),
            models: vec![ModelInfo {
                id: model.clone(),
                status: ModelStatus::Current,
                recommended: true,
                max_refs: 16,
                mask: false,
                alpha: AlphaSupport::None,
                sizes: SizeRule::Arbitrary { multiple: 16, min_pixels: 65_536, max_pixels: 8_294_400, max_edge: 3840, max_ratio: 3.0 },
                cost: CostModel::PlanQuota,
                async_job: false,
                supports_edit: true,
                edit_model: None,
                notes: "Fixed to gpt-6-luna (maintainer decision) at the highest reasoning effort the backend accepts, tried high, then medium, then low (W5-6); the backend must echo the model and the effort asked for or the image is refused. The backend's own image tool chooses its model, quality and size, so this route cannot force image quality; what it chose is recorded per image. No mask, no documented alpha (transparency is proven from pixels), no usage report.".into(),
            }],
            terms_note: "Presents itself as the Codex CLI to a backend OpenAI does not document for other tools; may break or be blocked without notice; account consequences are yours. Local only: never hosted, never shared.".into(),
            // recorded real responses: docs/evidence/2026-11-01-luna-probe.json and the 40-family run of 2026-11-02
            live_verified: true,
        };
        let route = Route::new(store.state_dir()).with_clock(clock.clone());
        ChatGptLoginProvider { desc, store, transport, policy: HostPolicy::api(&HOSTS), clock, model, record_dir: None, route }
    }

    /// The recorded server revocation, as the sentence every report uses. `None` while the sign-in is not known to be revoked.
    pub fn revocation_notice(&self) -> Option<String> {
        self.route.state().revoked_message()
    }

    /// The route's recorded state (revocation, last request, backoff). Holds no secret.
    pub fn route_state(&self) -> RouteState {
        self.route.state()
    }

    /// The warning that the route dropped to one request at a time, while that holds.
    pub fn fallback_notice(&self) -> Option<String> {
        self.route.state().active_fallback(self.now_secs()).map(route::Fallback::message)
    }

    /// Add the fall-back warning to a generation's notes or to its error.
    fn with_fallback_notice(&self, result: Result<ProviderOutput, ProviderError>) -> Result<ProviderOutput, ProviderError> {
        let Some(warning) = self.fallback_notice() else { return result };
        match result {
            Ok(mut out) => {
                out.notes.push(format!("warning: {warning}"));
                Ok(out)
            }
            Err(mut e) if e.kind != ErrorKind::Cancelled => {
                e.message = format!("{}; warning: {warning}", e.message);
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    fn refuse_if_revoked(&self, state: &RouteState) -> Result<(), ProviderError> {
        match &state.revoked {
            Some(r) => Err(route::revoked_error(r)),
            None => Ok(()),
        }
    }

    fn now_secs(&self) -> i64 {
        self.clock.now_secs()
    }

    /// `login-status --check`: confirm with the server that the sign-in is still accepted, using only a call this
    /// provider already makes. The only such call that is cheap and sends no image is the token refresh, so the
    /// check refreshes when the access token is within a minute of expiry and otherwise makes no request: the
    /// backend's responses endpoint accepts only generations, and a generation costs plan quota.
    pub fn check(&self, ctx: &CallContext) -> Result<CheckOutcome, ProviderError> {
        self.refuse_if_revoked(&self.route.state())?;
        let guard = self.route.acquire(&ctx.cancel, &*ctx.sleeper)?;
        self.refuse_if_revoked(&guard.state())?;
        let creds = self
            .store
            .load()
            .map_err(|e| ProviderError::new(ErrorKind::NotConfigured, RetryClass::RequiresAuthorization, e))?
            .ok_or_else(|| ProviderError::not_configured("not signed in: run `hoi4-imagegen login`"))?;
        if !self.near_expiry(&creds) {
            return Ok(CheckOutcome::NotChecked(
                "the access token is not near expiry, so no request was made; the only other endpoint this route calls is the image backend, and every call to it is a generation that costs plan quota"
                    .into(),
            ));
        }
        self.credentials(&guard, "token refresh")?;
        Ok(CheckOutcome::Accepted)
    }

    fn near_expiry(&self, creds: &Credentials) -> bool {
        let now_ms = (self.now_secs().max(0) as u64) * 1000;
        creds.expires_ms.map(|e| now_ms + EXPIRY_MARGIN_MS >= e).unwrap_or(false)
    }

    /// A valid access token and account id, refreshing and saving when needed. The caller holds the route lock.
    fn credentials(&self, guard: &RouteGuard, during: &str) -> Result<Credentials, ProviderError> {
        let creds = self
            .store
            .load()
            .map_err(|e| ProviderError::new(ErrorKind::NotConfigured, RetryClass::RequiresAuthorization, e))?
            .ok_or_else(|| ProviderError::not_configured("not signed in: run `hoi4-imagegen login`"))?;
        let now_ms = (self.now_secs().max(0) as u64) * 1000;
        if !self.near_expiry(&creds) {
            return Ok(creds);
        }
        let refresh = creds.refresh.clone().ok_or_else(|| {
            ProviderError::new(
                ErrorKind::Auth,
                RetryClass::RequiresAuthorization,
                "the access token expired and there is no refresh token: sign in again",
            )
        })?;
        let resp = send(&*self.transport, &self.policy, &refresh_request(&refresh))?;
        if resp.status != 200 {
            if route::is_revocation(resp.status, &resp.body) {
                return Err(route::revoked_error(&guard.record_revoked(self.now_secs(), resp.status, &resp.body, during)));
            }
            return Err(status_error(resp.status, &resp.body, &[refresh, creds.access.clone()]));
        }
        let tokens = parse_tokens(&resp.body).map_err(|e| ProviderError::new(ErrorKind::MalformedResponse, RetryClass::Terminal, e))?;
        let updated = Credentials {
            account_id: creds.account_id.clone().or_else(|| account_id(&tokens.access)),
            expires_ms: tokens.expires_in.map(|s| now_ms + s * 1000).or_else(|| expiry_ms(&tokens.access)),
            refresh: tokens.refresh.or(creds.refresh),
            access: tokens.access,
            last_refresh: self.clock.now_rfc3339(),
        };
        self.store.save(&updated).map_err(|e| ProviderError::new(ErrorKind::Transport, RetryClass::Terminal, e))?;
        Ok(updated)
    }

    /// Request body, public for tests.
    pub fn build_body(&self, req: &ImageRequest, size: Option<(u32, u32)>, effort: &str) -> Value {
        let mut content = vec![json!({"type": "input_text", "text": req.prompt})];
        if let Some(edit) = &req.edit {
            content.push(json!({"type": "input_image", "image_url": data_url("image/png", &edit.base_png)}));
        }
        for r in &req.refs {
            content.push(json!({"type": "input_image", "image_url": data_url("image/png", &r.png)}));
        }
        let mut tool = json!({"type": "image_generation", "output_format": "png", "quality": req.quality.clone().unwrap_or_else(|| "high".into())});
        if let Some((w, h)) = size {
            tool["size"] = json!(format!("{w}x{h}"));
        }
        let mut body = json!({
            "model": self.model,
            "reasoning": {"effort": effort},
            "instructions": INSTRUCTIONS,
            "input": [{"role": "user", "content": content}],
            "tools": [tool],
            "tool_choice": {"type": "image_generation"},
            "stream": true,
            "store": false,
        });
        if effort == EFFORT_BACKEND_DEFAULT {
            // no reasoning field: the backend chooses the effort, and the response reports which one
            body.as_object_mut().expect("an object").remove("reasoning");
        }
        body
    }

    fn call(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        let result = self.call_inner(req, ctx);
        self.with_fallback_notice(result)
    }

    fn call_inner(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.model != self.model {
            return Err(ProviderError::unsupported(format!(
                "the ChatGPT-login route runs only {} at {} (maintainer decision); '{}' was requested",
                self.model, EFFORT_POLICY, req.model
            )));
        }
        let size = self.prepare(req).ok();
        // A recorded revocation ends the call before anything is sent or waited for.
        self.refuse_if_revoked(&self.route.state())?;
        // One slot of the route's N on this machine, across processes (W5-8). Held until this function returns, so the
        // effort ladder below runs its requests one after another in this slot and keeps the slot's pacing.
        let mut guard = self.route.acquire(&ctx.cancel, &*ctx.sleeper)?;
        // W5-6: the highest effort the backend accepts. Each effort is tried once; a refusal of the effort itself
        // moves on to the next one, and every other error ends the call as before.
        // An effort refused within the last day is skipped, so each image is one request; the last rung is always
        // tried, so a backend that starts accepting it again is noticed.
        let now = self.now_secs();
        let remembered: Vec<String> = guard.state().refused_efforts.iter().filter(|r| r.until_epoch > now).map(|r| r.effort.clone()).collect();
        let mut refused: Vec<String> = Vec::new();
        let mut skipped: Vec<&str> = Vec::new();
        for (i, effort) in EFFORT_LADDER.iter().copied().enumerate() {
            let last = i + 1 == EFFORT_LADDER.len();
            if !last && remembered.iter().any(|e| e == effort) {
                skipped.push(effort);
                continue;
            }
            match self.attempt(req, size, effort, &mut guard, ctx)? {
                Attempt::Done(reported_effort, out) => {
                    let mut out = *out;
                    if !skipped.is_empty() {
                        out.notes.push(format!("skipped {} reasoning effort, which the backend refused within the last day", skipped.join(" and ")));
                    }
                    if !refused.is_empty() {
                        out.notes.push(format!("the backend refused the higher efforts first ({})", refused.join("; ")));
                    }
                    // the effort as data, on the candidate record in the ledger
                    let backend_usage = out.usage.take();
                    out.usage = Some(json!({
                        "reasoning_effort": effort,
                        "reported_effort": reported_effort,
                        "refused_efforts": refused.iter().map(|r| r.split(':').next().unwrap_or("")).chain(skipped.iter().copied()).collect::<Vec<_>>(),
                        "backend_usage": backend_usage,
                    }));
                    return Ok(out);
                }
                Attempt::EffortRefused(msg) => {
                    let until = self.now_secs() + EFFORT_REFUSAL_MEMORY_SECS;
                    guard.update(|s| {
                        s.refused_efforts.retain(|r| r.effort != effort);
                        s.refused_efforts.push(route::RefusedEffort { effort: effort.to_string(), until_epoch: until });
                    })?;
                    refused.push(format!("{effort}: {msg}"));
                }
            }
        }
        Err(ProviderError::new(
            ErrorKind::BadRequest,
            RetryClass::Terminal,
            format!(
                "the backend refused {} at every reasoning effort it was offered ({}), so no image was made: {}",
                self.model,
                EFFORT_LADDER.join(", "),
                refused.join("; ")
            ),
        ))
    }

    /// One request at one reasoning effort, in the caller's slot.
    fn attempt(
        &self,
        req: &ImageRequest,
        size: Option<(u32, u32)>,
        effort: &str,
        guard: &mut RouteGuard,
        ctx: &CallContext,
    ) -> Result<Attempt, ProviderError> {
        let mut retries = 0u32;
        let (resp, creds) = loop {
            // the state may have changed while this call waited for a slot
            self.refuse_if_revoked(&guard.state())?;
            // after a sign-out or a rate limit anywhere on the machine, only slot 0 may send
            guard.narrow(&ctx.cancel, &*ctx.sleeper)?;
            let wait_until = guard.next_allowed(self.now_secs());
            guard.wait_until(self.now_secs(), wait_until, &ctx.cancel, &*ctx.sleeper)?;
            let creds = self.credentials(guard, "token refresh")?;
            let body = self.build_body(req, size, effort);
            let mut http = HttpRequest::post_json(BACKEND_URL, &body)
                .header("Authorization", &format!("Bearer {}", creds.access))
                .header("originator", ORIGINATOR)
                .header("Accept", "text/event-stream");
            if let Some(id) = &creds.account_id {
                http = http.header("chatgpt-account-id", id);
            }
            // Stamp before sending, so a request that dies mid-flight still spaces out the next one.
            guard.stamp_request(self.now_secs())?;
            let resp = send(&*self.transport, &self.policy, &http)?;
            if resp.status == 429 {
                // the backoff is shared by every slot, and the route drops to one request at a time
                let failures = guard.record_rate_limited(self.now_secs())?.failures;
                if retries < RATE_LIMIT_MAX_RETRIES {
                    // Back off (the next loop waits for retry_after_epoch while still holding a slot), then try again.
                    retries += 1;
                    continue;
                }
                return Err(ProviderError::new(
                    ErrorKind::RateLimited,
                    RetryClass::SafeSameInputs,
                    format!(
                        "rate limited by the ChatGPT plan (429) after {} tries with growing waits; the next try is held back for {} seconds",
                        retries + 1,
                        route::backoff_secs(failures)
                    ),
                )
                .with_status(429));
            }
            break (resp, creds);
        };
        let secrets = vec![creds.access.clone()];
        match resp.status {
            200 => {
                if guard.state().rate_limit.is_some() {
                    guard.update(|s| s.rate_limit = None)?;
                }
            }
            s if route::is_revocation(s, &resp.body) => {
                return Err(route::revoked_error(&guard.record_revoked(self.now_secs(), s, &resp.body, "generation")));
            }
            s if is_effort_refusal(&String::from_utf8_lossy(&resp.body)) => {
                return Ok(Attempt::EffortRefused(format!("HTTP {s}: {}", short(&String::from_utf8_lossy(&resp.body)))));
            }
            s if route::is_usage_limit(&String::from_utf8_lossy(&resp.body)) => {
                guard.record_usage_limit(self.now_secs());
                return Err(usage_limit_error(&format!("HTTP {s}")));
            }
            s => return Err(status_error(s, &resp.body, &secrets)),
        }
        let text = String::from_utf8_lossy(&resp.body);
        self.record(&text);
        if let Err(e) = parse_sse(&text) {
            if let Some(msg) = e.strip_prefix(BACKEND_ERROR_PREFIX).filter(|m| is_effort_refusal(m)) {
                return Ok(Attempt::EffortRefused(msg.to_string()));
            }
            if let Some(msg) = e.strip_prefix(BACKEND_ERROR_PREFIX).filter(|m| route::is_usage_limit(m)) {
                guard.record_usage_limit(self.now_secs());
                return Err(usage_limit_error(&short(msg)));
            }
        }
        let outcome = parse_sse(&text).map_err(|e| {
            if let Some(d) = e.strip_prefix(MODERATION_PREFIX) {
                ProviderError::new(
                    ErrorKind::ModerationBlocked,
                    RetryClass::Terminal,
                    format!("the provider's safety system refused this image; change the brief or the references and plan again ({d})"),
                )
            } else if let Some(d) = e.strip_prefix(FAILED_PREFIX) {
                ProviderError::new(ErrorKind::BadRequest, RetryClass::Terminal, format!("the image call failed: {d}"))
            } else if let Some(d) = e.strip_prefix(BACKEND_ERROR_PREFIX) {
                // the backend refused the request itself: nothing was made, so there is nothing to reconcile
                ProviderError::new(
                    ErrorKind::BadRequest,
                    RetryClass::Terminal,
                    format!("the ChatGPT backend rejected the request at {effort} reasoning effort: {d}"),
                )
            } else {
                ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, e)
            }
        })?;
        let (model, effort) = verify_model(&outcome, effort)?;
        let reported_effort = outcome.effort.clone();
        let mut notes = vec![format!("ran on {model} at {effort} reasoning effort, as reported by the backend; usage is not reported")];
        if let Some(meta) = outcome.image_meta.as_object() {
            if let Some(m) = &outcome.image_tool_model {
                notes.push(format!("image tool model (chosen by the backend): {m}"));
            }
            for k in ["size", "quality", "background", "output_format"] {
                if let Some(v) = meta.get(k).and_then(Value::as_str) {
                    notes.push(format!("image {k}: {v}"));
                }
            }
            if let Some(v) = meta.get("revised_prompt").and_then(Value::as_str) {
                notes.push(format!("revised prompt: {v}"));
            }
        }
        Ok(Attempt::Done(
            reported_effort,
            Box::new(ProviderOutput {
                image: b64_decode(&outcome.image_b64)?,
                mime: "image/png".into(),
                provider_request_id: outcome.response_id.clone(),
                usage: outcome.usage.clone(),
                actual_usd: None,
                usage_unknown: true,
                model_echo: Some(model),
                notes,
                job: None,
            }),
        ))
    }

    /// Save the raw response for fixtures when [`RECORD_ENV`] names a folder. Never fails the call.
    fn record(&self, text: &str) {
        let Some(dir) = &self.record_dir else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let name = format!("chatgpt-login-{}.sse", self.clock.now_rfc3339().replace([':', '-'], ""));
        let _ = std::fs::write(dir.join(name), text.as_bytes());
    }
}

/// What a response stream said.
#[derive(Debug, Clone, Default)]
pub struct SseOutcome {
    /// The base64 PNG.
    pub image_b64: String,
    /// `response.model`, as reported.
    pub model: Option<String>,
    /// `response.reasoning.effort`, as reported.
    pub effort: Option<String>,
    /// `response.id`.
    pub response_id: Option<String>,
    /// The image call item without its result (size, quality, revised prompt and so on).
    pub image_meta: Value,
    /// `response.usage`, when present.
    pub usage: Option<Value>,
    /// The model of the backend's hosted image tool, as listed in `response.tools` (it renders the pixels).
    pub image_tool_model: Option<String>,
}

/// Read a response stream: the first image result, plus the model and effort the backend reports in its response
/// events. Surfaces backend error events instead of waiting for the end.
pub fn parse_sse(stream: &str) -> Result<SseOutcome, String> {
    let normalized = stream.replace("\r\n", "\n");
    let mut out = SseOutcome::default();
    let mut image: Option<String> = None;
    for raw in normalized.split("\n\n") {
        let data: String = raw.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim).collect::<Vec<_>>().join("");
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        if let Some(r) = v.get("response") {
            if let Some(m) = r.get("model").and_then(Value::as_str) {
                out.model = Some(m.to_string());
            }
            if let Some(e) = r.pointer("/reasoning/effort").and_then(Value::as_str) {
                out.effort = Some(e.to_string());
            }
            if let Some(id) = r.get("id").and_then(Value::as_str) {
                out.response_id = Some(id.to_string());
            }
            if let Some(u) = r.get("usage").filter(|u| !u.is_null()) {
                out.usage = Some(u.clone());
            }
            if let Some(m) = r
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|t| t.iter().find(|x| x["type"] == "image_generation"))
                .and_then(|x| x.get("model"))
                .and_then(Value::as_str)
            {
                out.image_tool_model = Some(m.to_string());
            }
        }
        match v.get("type").and_then(Value::as_str) {
            Some("response.output_item.done") if image.is_none() => {
                // a refused or failed image call: the backend then answers in text, and the stream has no image
                if v.pointer("/item/type").and_then(Value::as_str) == Some("image_generation_call")
                    && v.pointer("/item/status").and_then(Value::as_str) == Some("failed")
                {
                    let detail = match v.pointer("/item/error") {
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => other.to_string(),
                        None => "no detail".into(),
                    };
                    let kind = if detail.contains("moderation_blocked") { MODERATION_PREFIX } else { FAILED_PREFIX };
                    return Err(format!("{kind}{detail}"));
                }
                if v.pointer("/item/type").and_then(Value::as_str) == Some("image_generation_call") {
                    if let Some(result) = v.pointer("/item/result").and_then(Value::as_str) {
                        image = Some(result.to_string());
                        let mut meta = v["item"].clone();
                        if let Some(o) = meta.as_object_mut() {
                            o.remove("result");
                        }
                        out.image_meta = meta;
                    }
                }
            }
            Some("response.failed") | Some("error") => {
                let msg = v
                    .pointer("/response/error/message")
                    .or_else(|| v.pointer("/error/message"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown backend error");
                return Err(format!("{BACKEND_ERROR_PREFIX}{msg}"));
            }
            _ => {}
        }
    }
    out.image_b64 = image.ok_or("the stream ended without an image_generation result")?;
    Ok(out)
}

/// What one effort attempt came to.
enum Attempt {
    /// An image, verified.
    Done(Option<String>, Box<ProviderOutput>),
    /// The backend refused this reasoning effort; the next one may be tried.
    EffortRefused(String),
}

/// The backend said the plan's usage limit is reached. Nothing was made; retrying soon would only hit it again.
fn usage_limit_error(detail: &str) -> ProviderError {
    ProviderError::new(
        ErrorKind::RateLimited,
        RetryClass::RequiresAuthorization,
        format!("the ChatGPT plan's usage limit is reached, so no image was made; wait for the plan to reset or use an API provider ({detail})"),
    )
}

/// Prefix of a backend error event, as [`parse_sse`] reports it.
const BACKEND_ERROR_PREFIX: &str = "backend error: ";

/// True when the backend refused the reasoning effort itself, for example "Complexity 'high' is not supported for
/// this image model" (2026-11-05) or a message naming the reasoning effort as unsupported.
pub fn is_effort_refusal(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    let unsupported = m.contains("not supported") || m.contains("unsupported") || m.contains("not allowed") || m.contains("invalid value");
    unsupported && (m.contains("complexity") || m.contains("reasoning effort") || m.contains("reasoning.effort"))
}

fn short(text: &str) -> String {
    text.chars().take(300).collect()
}

/// Refuse anything that did not run on gpt-6-luna at the effort this attempt asked for, or that does not say what it
/// ran on.
pub fn verify_model(o: &SseOutcome, asked: &str) -> Result<(String, String), ProviderError> {
    let refuse = |msg: String| ProviderError::new(ErrorKind::Unsupported, RetryClass::Terminal, msg);
    let model = o.model.clone().ok_or_else(|| {
        refuse(format!("the backend did not report which model ran, so the image cannot be shown to come from {MODEL}; it was not used"))
    })?;
    if model != MODEL && !model.starts_with(&format!("{MODEL}-")) {
        return Err(refuse(format!("the backend ran '{model}' instead of {MODEL}; the image was not used (no fallback is allowed)")));
    }
    if asked == EFFORT_BACKEND_DEFAULT {
        // no effort was asked for: whatever effort the backend reports is recorded with the image
        let effort =
            o.effort.clone().map(|e| format!("{EFFORT_BACKEND_DEFAULT} (reported {e})")).unwrap_or_else(|| EFFORT_BACKEND_DEFAULT.to_string());
        return Ok((model, effort));
    }
    let effort = o.effort.clone().ok_or_else(|| {
        refuse(format!("the backend did not report the reasoning effort, so the asked {asked} effort cannot be shown; the image was not used"))
    })?;
    if effort != asked {
        return Err(refuse(format!("the backend ran at '{effort}' reasoning effort instead of the asked {asked}; the image was not used")));
    }
    Ok((model, effort))
}

impl ImageProvider for ChatGptLoginProvider {
    fn id(&self) -> &str {
        "chatgpt_login"
    }

    fn descriptor(&self) -> &ProviderDescriptor {
        &self.desc
    }

    fn is_configured(&self) -> bool {
        self.store.load().map(|c| c.is_some()).unwrap_or(false)
    }

    fn default_model(&self, _editing: bool) -> String {
        self.model.clone()
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        choose_size(&self.desc.models[0].sizes, [req.width, req.height])
    }

    fn estimate(&self, _req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        Ok(CostEstimate {
            usd: 0.0,
            basis:
                "ChatGPT plan quota: there is no per-image price and no usage report, so spend is recorded as unknown and no operator allowance is ever charged"
                    .into(),
            usage_unknown: true,
            funding: Funding::PlanQuota,
        })
    }

    fn generate(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.edit.is_some() {
            return Err(ProviderError::unsupported("generate was called with an edit request"));
        }
        self.call(req, ctx)
    }

    fn edit(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.edit.is_none() {
            return Err(ProviderError::unsupported("edit was called without a base image"));
        }
        if req.edit.as_ref().map(|e| e.mask_png.is_some()).unwrap_or(false) {
            return Err(ProviderError::unsupported("this provider has no mask support"));
        }
        self.call(req, ctx)
    }
}

/// Used by the command-line helper to show who is signed in.
pub fn describe(creds: &Credentials) -> String {
    format!(
        "signed in; account {}; plan {}; token {}",
        creds.account_id.as_deref().unwrap_or("unknown"),
        oauth::plan_type(&creds.access).unwrap_or_else(|| "unknown".into()),
        match creds.expires_ms {
            Some(ms) => format!("expires at {ms} ms since the epoch"),
            None => "has no recorded expiry".to_string(),
        }
    )
}

#[allow(dead_code)]
fn _b64(bytes: &[u8]) -> String {
    b64(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imagegen_core::hashing::FixedClock;
    use imagegen_providers::transport::{FixtureTransport, Method};
    use imagegen_providers::types::EditSpec;
    use std::path::Path;

    // SYNTHETIC: every reply below was written for these tests. The backend is never contacted.

    fn dir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new().prefix("g").tempdir_in(base).unwrap()
    }

    fn request() -> ImageRequest {
        ImageRequest {
            model: MODEL.into(),
            prompt: "an iron wall".into(),
            negative: vec![],
            width: 100,
            height: 88,
            transparent: true,
            quality: Some("low".into()),
            refs: vec![],
            edit: None,
            idempotency_key: None,
            seed: None,
        }
    }

    fn provider(d: &tempfile::TempDir, fixture: Arc<FixtureTransport>, creds: Option<Credentials>, now: i64) -> ChatGptLoginProvider {
        let store = CredentialStore::at(d.path().join("auth.json"));
        if let Some(c) = creds {
            store.save(&c).unwrap();
        }
        ChatGptLoginProvider::new(store, fixture, Arc::new(FixedClock(now)))
    }

    fn creds(expires_ms: Option<u64>) -> Credentials {
        Credentials {
            access: "access-synthetic-1".into(),
            refresh: Some("refresh-synthetic-1".into()),
            account_id: Some("acct-synthetic".into()),
            expires_ms,
            last_refresh: "t".into(),
        }
    }

    fn sse(png_b64: &str) -> String {
        format!(
            "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp_synthetic\",\"model\":\"gpt-6-luna\",\"reasoning\":{{\"effort\":\"high\"}}}}}}\n\n: keepalive\n\ndata: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"image_generation_call\",\"size\":\"1024x1024\",\"result\":\"{png_b64}\"}}}}\n\ndata: [DONE]\n\n"
        )
    }

    #[test]
    fn sse_parsing_finds_the_image_and_surfaces_errors() {
        let o = parse_sse(&sse("QUJD")).unwrap();
        assert_eq!(
            (o.image_b64.as_str(), o.model.as_deref(), o.effort.as_deref(), o.response_id.as_deref()),
            ("QUJD", Some("gpt-6-luna"), Some("high"), Some("resp_synthetic"))
        );
        assert_eq!(o.image_meta["size"], "1024x1024");
        assert!(o.image_meta.get("result").is_none(), "the image itself is not kept in the metadata");
        assert_eq!(parse_sse(&sse("QUJD").replace('\n', "\r\n")).unwrap().image_b64, "QUJD", "CRLF streams parse too");
        assert!(parse_sse("data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"quota\"}}}\n\n")
            .unwrap_err()
            .contains("quota"));
        assert!(parse_sse("data: {\"type\":\"error\",\"error\":{\"message\":\"boom\"}}\n\n").unwrap_err().contains("boom"));
        assert!(parse_sse("data: [DONE]\n\n").is_err());
        assert!(parse_sse("data: not json\n\n").is_err());
    }

    #[test]
    fn only_gpt_6_luna_at_the_asked_effort_is_accepted_and_silence_is_refused() {
        let ok = parse_sse(&sse("QUJD")).unwrap();
        assert_eq!(verify_model(&ok, "high").unwrap(), ("gpt-6-luna".to_string(), "high".to_string()));
        let snapshot = SseOutcome { model: Some("gpt-6-luna-2026-10-01".into()), ..ok.clone() };
        assert!(verify_model(&snapshot, "high").is_ok(), "a dated snapshot of the same model is the same model");
        let medium = SseOutcome { effort: Some("medium".into()), ..ok.clone() };
        assert!(verify_model(&medium, "medium").is_ok(), "the effort an attempt asked for is accepted");
        for (model, effort, needle) in [
            (Some("gpt-6-astra"), Some("high"), "instead of gpt-6-luna"),
            (Some("gpt-6-lunar"), Some("high"), "instead of gpt-6-luna"),
            (None, Some("high"), "did not report which model"),
            (Some("gpt-6-luna"), Some("medium"), "instead of the asked high"),
            (Some("gpt-6-luna"), None, "did not report the reasoning effort"),
        ] {
            let o = SseOutcome { model: model.map(String::from), effort: effort.map(String::from), ..ok.clone() };
            let e = verify_model(&o, "high").unwrap_err();
            assert!(e.message.contains(needle), "{model:?}/{effort:?}: {}", e.message);
            assert_eq!(e.retry, RetryClass::Terminal, "no retry and no fallback");
        }
    }

    #[test]
    fn a_downgraded_response_never_becomes_an_image() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], sse("QUJD").replace("\"effort\":\"high\"", "\"effort\":\"low\""));
        let p = provider(&d, fixture, Some(creds(None)), 0);
        let e = p.generate(&request(), &CallContext::new()).unwrap_err();
        assert!(e.message.contains("instead of the asked high"), "{}", e.message);
    }

    fn no_sleep() -> CallContext {
        CallContext { sleeper: Arc::new(crate::route::CountingSleeper::default()), ..CallContext::new() }
    }

    /// The rung that sends no reasoning field: the body has none, the model is still checked, and the effort the
    /// backend reports is recorded.
    #[test]
    fn the_backend_default_rung_omits_the_effort_and_still_checks_the_model() {
        let d = dir();
        let p = provider(&d, Arc::new(FixtureTransport::new()), None, 0);
        let body = p.build_body(&request(), None, crate::EFFORT_BACKEND_DEFAULT);
        assert!(body.get("reasoning").is_none(), "{body}");
        let ok = parse_sse(&sse("QUJD")).unwrap();
        assert_eq!(verify_model(&ok, crate::EFFORT_BACKEND_DEFAULT).unwrap().1, "backend default (reported high)");
        let silent = SseOutcome { effort: None, ..ok.clone() };
        assert_eq!(verify_model(&silent, crate::EFFORT_BACKEND_DEFAULT).unwrap().1, "backend default");
        let other = SseOutcome { model: Some("gpt-6-astra".into()), ..ok };
        assert!(verify_model(&other, crate::EFFORT_BACKEND_DEFAULT).is_err(), "the model check never relaxes");
    }

    fn refusal(effort: &str) -> String {
        format!(
            "event: error
data: {{\"type\":\"error\",\"error\":{{\"message\":\"Complexity '{effort}' is not supported for this image model.\"}}}}

"
        )
    }

    /// W5-6: high is refused (the 2026-11-05 backend message), medium is accepted and recorded.
    #[test]
    fn a_refused_effort_falls_to_the_next_one_and_the_effort_used_is_recorded() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], refusal("high"));
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], sse("QUJD").replace("\"effort\":\"high\"", "\"effort\":\"medium\""));
        let p = provider(&d, fixture.clone(), Some(creds(None)), 0);
        let out = p.generate(&request(), &no_sleep()).unwrap();
        assert!(out.notes[0].contains("gpt-6-luna at medium reasoning effort"), "{:?}", out.notes);
        assert!(out.notes.iter().any(|n| n.contains("refused the higher efforts first") && n.contains("high: Complexity 'high'")), "{:?}", out.notes);
        let sent: Vec<Value> = fixture.requests().iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect();
        assert_eq!(sent.iter().map(|b| b["reasoning"]["effort"].as_str().unwrap()).collect::<Vec<_>>(), ["high", "medium"]);
        assert!(sent.iter().all(|b| b["model"] == "gpt-6-luna"), "the model never changes");
        // the effort is data on the candidate, not only a note
        let usage = out.usage.clone().unwrap();
        assert_eq!(usage["reasoning_effort"], "medium");
        assert_eq!(usage["refused_efforts"], json!(["high"]));
    }

    /// W5-6 (2026-11-05): high, medium and low refused, then the request without an effort is accepted. The effort
    /// is recorded as the backend default, beside what the backend reports.
    #[test]
    fn when_every_named_effort_is_refused_the_backend_default_is_used_and_recorded() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        for e in ["high", "medium", "low"] {
            fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], refusal(e));
        }
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], sse("QUJD").replace("\"effort\":\"high\"", "\"effort\":\"none\""));
        let p = provider(&d, fixture.clone(), Some(creds(None)), 0);
        let out = p.generate(&request(), &no_sleep()).unwrap();
        let usage = out.usage.clone().unwrap();
        assert_eq!(usage["reasoning_effort"], "backend default");
        assert_eq!(usage["reported_effort"], "none");
        assert_eq!(usage["refused_efforts"], json!(["high", "medium", "low"]));
        assert!(out.notes[0].contains("gpt-6-luna at backend default (reported none) reasoning effort"), "{:?}", out.notes);
        // the next image goes straight to the backend default
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], sse("QUJD"));
        let next = p.generate(&request(), &no_sleep()).unwrap();
        assert_eq!(fixture.requests().len(), 5, "one request for the second image");
        assert_eq!(next.usage.unwrap()["reasoning_effort"], "backend default");
    }

    /// A refused effort is remembered on the machine, so the next image is one request at the effort that works.
    #[test]
    fn a_refused_effort_is_skipped_on_the_next_image() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        let medium = sse("QUJD").replace("\"effort\":\"high\"", "\"effort\":\"medium\"");
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], refusal("high"));
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], medium.clone());
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], medium);
        let p = provider(&d, fixture.clone(), Some(creds(None)), 0);
        p.generate(&request(), &no_sleep()).unwrap();
        let second = p.generate(&request(), &no_sleep()).unwrap();
        assert_eq!(fixture.requests().len(), 3, "the second image sends one request");
        assert!(second.notes.iter().any(|n| n.contains("skipped high reasoning effort")), "{:?}", second.notes);
        assert_eq!(second.usage.unwrap()["reasoning_effort"], "medium");
        // after a day, high is tried again
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], sse("QUJD"));
        let later = provider(&d, fixture.clone(), Some(creds(None)), crate::EFFORT_REFUSAL_MEMORY_SECS + 60);
        assert_eq!(later.generate(&request(), &no_sleep()).unwrap().usage.unwrap()["reasoning_effort"], "high");
    }

    #[test]
    fn every_effort_refused_is_one_clear_terminal_error() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        for e in ["high", "medium", "low", "low"] {
            fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], refusal(e));
        }
        let p = provider(&d, fixture.clone(), Some(creds(None)), 0);
        let e = p.generate(&request(), &no_sleep()).unwrap_err();
        assert_eq!((e.kind, e.retry), (ErrorKind::BadRequest, RetryClass::Terminal));
        assert!(e.message.contains("every reasoning effort") && e.message.contains("low: Complexity 'low'"), "{}", e.message);
        assert_eq!(fixture.requests().len(), 4, "each rung is tried once, the last without an effort");
        let last: Value = serde_json::from_slice(&fixture.requests()[3].body).unwrap();
        assert!(last.get("reasoning").is_none(), "the last rung sends no reasoning field");
    }

    #[test]
    fn an_http_level_effort_refusal_also_moves_on() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(
            Method::Post,
            "/backend-api/codex/responses",
            400,
            &[],
            br#"{"error":{"message":"Unsupported value: 'high' is not supported with this model for reasoning.effort"}}"#.to_vec(),
        );
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], sse("QUJD").replace("\"effort\":\"high\"", "\"effort\":\"medium\""));
        let p = provider(&d, fixture, Some(creds(None)), 0);
        assert!(p.generate(&request(), &no_sleep()).is_ok());
    }

    /// Any other backend error event is the backend refusing the request: terminal, and named, never "malformed".
    #[test]
    fn another_backend_error_is_terminal_and_not_retried_at_a_lower_effort() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(
            Method::Post,
            "/backend-api/codex/responses",
            200,
            &[],
            "event: error
data: {\"type\":\"error\",\"error\":{\"message\":\"Invalid prompt.\"}}

",
        );
        let p = provider(&d, fixture.clone(), Some(creds(None)), 0);
        let e = p.generate(&request(), &no_sleep()).unwrap_err();
        assert_eq!((e.kind, e.retry), (ErrorKind::BadRequest, RetryClass::Terminal));
        assert!(e.message.contains("rejected the request at high reasoning effort") && e.message.contains("Invalid prompt"), "{}", e.message);
        assert_eq!(fixture.requests().len(), 1);
    }

    #[test]
    fn only_effort_messages_count_as_effort_refusals() {
        assert!(is_effort_refusal("Complexity 'high' is not supported for this image model."));
        assert!(is_effort_refusal("Unsupported value: 'high' is not supported with this model for reasoning.effort"));
        assert!(!is_effort_refusal("Invalid prompt."));
        assert!(!is_effort_refusal("Your request was rejected by the safety system"));
    }

    #[test]
    fn the_request_matches_what_upstream_sent() {
        let d = dir();
        let p = provider(&d, Arc::new(FixtureTransport::new()), None, 0);
        let mut r = request();
        r.edit = Some(EditSpec { base_png: vec![1], mask_png: None });
        let body = p.build_body(&r, Some((928, 816)), "high");
        assert_eq!(body["model"], "gpt-6-luna");
        assert_eq!(body["reasoning"], json!({"effort": "high"}));
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert_eq!(body["tool_choice"], json!({"type": "image_generation"}));
        assert_eq!(body["tools"][0], json!({"type": "image_generation", "output_format": "png", "quality": "low", "size": "928x816"}));
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][0]["content"][1]["type"], "input_image");
        assert!(body["instructions"].as_str().unwrap().contains("exactly once"));
    }

    #[test]
    fn a_fresh_token_generates_without_refreshing() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[("content-type", "text/event-stream")], sse("QUJD"));
        let p = provider(&d, fixture.clone(), Some(creds(Some(4_000_000_000_000))), 1_793_404_800);
        assert!(p.is_configured());
        let out = p.generate(&request(), &CallContext::new()).unwrap();
        assert_eq!(out.image, b"ABC");
        assert!(out.usage_unknown && out.actual_usd.is_none(), "plan quota never reports usage");
        let reqs = fixture.requests();
        assert_eq!(reqs.len(), 1, "no refresh request for a fresh token");
        let h = |n: &str| reqs[0].headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
        assert_eq!(h("Authorization").as_deref(), Some("Bearer access-synthetic-1"));
        assert_eq!(h("chatgpt-account-id").as_deref(), Some("acct-synthetic"));
        assert_eq!(h("originator").as_deref(), Some("codex_cli_rs"));
    }

    #[test]
    fn an_expired_token_is_refreshed_and_saved_in_our_own_file() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(
            Method::Post,
            "/oauth/token",
            200,
            &[],
            br#"{"access_token":"access-synthetic-2","refresh_token":"refresh-synthetic-2","expires_in":3600}"#.to_vec(),
        );
        fixture.respond(Method::Post, "/backend-api/codex/responses", 200, &[], sse("QUJD"));
        let p = provider(&d, fixture.clone(), Some(creds(Some(1_000))), 1_793_404_800);
        p.generate(&request(), &CallContext::new()).unwrap();
        let reqs = fixture.requests();
        assert!(reqs[0].url.ends_with("/oauth/token"), "refresh first: {:?}", reqs.iter().map(|r| &r.url).collect::<Vec<_>>());
        let sent = reqs[1].headers.iter().find(|(k, _)| k == "Authorization").unwrap();
        assert_eq!(sent.1, "Bearer access-synthetic-2");
        let saved = CredentialStore::at(d.path().join("auth.json")).load().unwrap().unwrap();
        assert_eq!((saved.access.as_str(), saved.refresh.as_deref()), ("access-synthetic-2", Some("refresh-synthetic-2")));
        assert_eq!(saved.expires_ms, Some(1_793_404_800_000 + 3_600_000));
    }

    #[test]
    fn missing_expired_and_rejected_credentials_are_reported_plainly() {
        let d = dir();
        let p = provider(&d, Arc::new(FixtureTransport::new()), None, 0);
        assert!(!p.is_configured());
        assert_eq!(p.generate(&request(), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);

        let d2 = dir();
        let mut c = creds(Some(1));
        c.refresh = None;
        let p = provider(&d2, Arc::new(FixtureTransport::new()), Some(c), 1_793_404_800);
        assert_eq!(p.generate(&request(), &CallContext::new()).unwrap_err().kind, ErrorKind::Auth);

        let d3 = dir();
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/backend-api/codex/responses", 401, &[], "denied access-synthetic-1");
        let p = provider(&d3, fixture, Some(creds(None)), 0);
        let e = p.generate(&request(), &CallContext::new()).unwrap_err();
        assert_eq!((e.kind, e.retry), (ErrorKind::Auth, RetryClass::RequiresAuthorization));
        assert!(!e.message.contains("access-synthetic-1"));

        let d4 = dir();
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/backend-api/codex/responses", 429, &[], "");
        let p = provider(&d4, fixture, Some(creds(None)), 0);
        // the route backs off between tries; the sleeper only records, so this test does not really wait
        let ctx = CallContext { sleeper: Arc::new(crate::route::CountingSleeper::default()), ..CallContext::new() };
        assert_eq!(p.generate(&request(), &ctx).unwrap_err().kind, ErrorKind::RateLimited);
    }

    #[test]
    fn it_is_a_plan_quota_provider_that_never_claims_alpha_or_masks() {
        let d = dir();
        let p = provider(&d, Arc::new(FixtureTransport::new()), None, 0);
        let e = p.estimate(&request()).unwrap();
        assert_eq!((e.usd, e.funding, e.usage_unknown), (0.0, Funding::PlanQuota, true));
        assert_eq!(p.descriptor().models[0].alpha, AlphaSupport::None);
        let mut r = request();
        r.edit = Some(EditSpec { base_png: vec![1], mask_png: Some(vec![2]) });
        assert!(p.edit(&r, &CallContext::new()).unwrap_err().message.contains("mask"));
        assert!(p.generate(&ImageRequest { model: "other".into(), ..request() }, &CallContext::new()).is_err());
    }

    #[test]
    fn requests_only_ever_go_to_the_two_documented_hosts() {
        let d = dir();
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider(&d, fixture.clone(), Some(creds(None)), 0);
        let policy = &p.policy;
        assert!(policy.check(BACKEND_URL).is_ok());
        assert!(policy.check(crate::TOKEN_URL).is_ok());
        assert!(policy.check("https://api.openai.com/v1/images").is_err());
        assert!(policy.check("https://chatgpt.com.evil.example/x").is_err());
    }
}
