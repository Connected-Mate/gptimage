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
use crate::{BACKEND_URL, HOSTS, MODEL, ORIGINATOR, REASONING_EFFORT};

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
    /// Build from the environment: the operating system credential store (see [`CredentialStore::for_environment`]).
    pub fn from_environment(env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) -> Result<Self, String> {
        let mut p = Self::new(CredentialStore::for_environment(env)?, transport, Arc::new(SystemClock));
        p.record_dir = env.get(RECORD_ENV).map(PathBuf::from);
        Ok(p)
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
                notes: "Fixed to gpt-6-luna at high reasoning effort (maintainer decision); the backend must echo both or the image is refused. The backend's own image tool chooses its model, quality and size, so this route cannot force image quality; what it chose is recorded per image. No mask, no documented alpha (transparency is proven from pixels), no usage report.".into(),
            }],
            terms_note: "Presents itself as the Codex CLI to a backend OpenAI does not document for other tools; may break or be blocked without notice; account consequences are yours. Local only: never hosted, never shared.".into(),
        };
        let route = Route::new(store.state_dir());
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
    pub fn build_body(&self, req: &ImageRequest, size: Option<(u32, u32)>) -> Value {
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
        json!({
            "model": self.model,
            "reasoning": {"effort": REASONING_EFFORT},
            "instructions": INSTRUCTIONS,
            "input": [{"role": "user", "content": content}],
            "tools": [tool],
            "tool_choice": {"type": "image_generation"},
            "stream": true,
            "store": false,
        })
    }

    fn call(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.model != self.model {
            return Err(ProviderError::unsupported(format!(
                "the ChatGPT-login route runs only {} at {} reasoning effort (maintainer decision); '{}' was requested",
                self.model, REASONING_EFFORT, req.model
            )));
        }
        let size = self.prepare(req).ok();
        // A recorded revocation ends the call before anything is sent or waited for.
        self.refuse_if_revoked(&self.route.state())?;
        // One request at a time on this machine, across processes. Held until this function returns.
        let guard = self.route.acquire(&ctx.cancel, &*ctx.sleeper)?;
        let mut retries = 0u32;
        let (resp, creds) = loop {
            // the state may have changed while this call waited for the lock
            self.refuse_if_revoked(&guard.state())?;
            let wait_until = guard.next_allowed(self.now_secs());
            guard.wait_until(self.now_secs(), wait_until, &ctx.cancel, &*ctx.sleeper)?;
            let creds = self.credentials(&guard, "token refresh")?;
            let body = self.build_body(req, size);
            let mut http = HttpRequest::post_json(BACKEND_URL, &body)
                .header("Authorization", &format!("Bearer {}", creds.access))
                .header("originator", ORIGINATOR)
                .header("Accept", "text/event-stream");
            if let Some(id) = &creds.account_id {
                http = http.header("chatgpt-account-id", id);
            }
            // Stamp before sending, so a request that dies mid-flight still spaces out the next one.
            let now = self.now_secs();
            guard.update(|s| s.last_request_epoch = Some(now))?;
            let resp = send(&*self.transport, &self.policy, &http)?;
            if resp.status == 429 {
                let failures = guard.state().rate_limit.map(|r| r.failures).unwrap_or(0) + 1;
                let retry_after_epoch = self.now_secs() + route::backoff_secs(failures);
                guard.update(|s| s.rate_limit = Some(route::RateLimit { failures, retry_after_epoch }))?;
                if retries < RATE_LIMIT_MAX_RETRIES {
                    // Back off (the next loop waits for retry_after_epoch while still holding the lock), then try again.
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
            s => return Err(status_error(s, &resp.body, &secrets)),
        }
        let text = String::from_utf8_lossy(&resp.body);
        self.record(&text);
        let outcome = parse_sse(&text).map_err(|e| {
            if let Some(d) = e.strip_prefix(MODERATION_PREFIX) {
                ProviderError::new(
                    ErrorKind::ModerationBlocked,
                    RetryClass::Terminal,
                    format!("the provider's safety system refused this image; change the brief or the references and plan again ({d})"),
                )
            } else if let Some(d) = e.strip_prefix(FAILED_PREFIX) {
                ProviderError::new(ErrorKind::BadRequest, RetryClass::Terminal, format!("the image call failed: {d}"))
            } else {
                ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, e)
            }
        })?;
        let (model, effort) = verify_model(&outcome)?;
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
        Ok(ProviderOutput {
            image: b64_decode(&outcome.image_b64)?,
            mime: "image/png".into(),
            provider_request_id: outcome.response_id.clone(),
            usage: outcome.usage.clone(),
            actual_usd: None,
            usage_unknown: true,
            model_echo: Some(model),
            notes,
            job: None,
        })
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
                return Err(format!("backend error: {msg}"));
            }
            _ => {}
        }
    }
    out.image_b64 = image.ok_or("the stream ended without an image_generation result")?;
    Ok(out)
}

/// Refuse anything that did not run on gpt-6-luna at high effort, or that does not say what it ran on.
pub fn verify_model(o: &SseOutcome) -> Result<(String, String), ProviderError> {
    let refuse = |msg: String| ProviderError::new(ErrorKind::Unsupported, RetryClass::Terminal, msg);
    let model = o.model.clone().ok_or_else(|| {
        refuse(format!("the backend did not report which model ran, so the image cannot be shown to come from {MODEL}; it was not used"))
    })?;
    if model != MODEL && !model.starts_with(&format!("{MODEL}-")) {
        return Err(refuse(format!("the backend ran '{model}' instead of {MODEL}; the image was not used (no fallback is allowed)")));
    }
    let effort = o.effort.clone().ok_or_else(|| {
        refuse(format!("the backend did not report the reasoning effort, so {REASONING_EFFORT} effort cannot be shown; the image was not used"))
    })?;
    if effort != REASONING_EFFORT {
        return Err(refuse(format!("the backend ran at '{effort}' reasoning effort instead of {REASONING_EFFORT}; the image was not used")));
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
    fn only_gpt_6_luna_at_high_effort_is_accepted_and_silence_is_refused() {
        let ok = parse_sse(&sse("QUJD")).unwrap();
        assert_eq!(verify_model(&ok).unwrap(), ("gpt-6-luna".to_string(), "high".to_string()));
        let snapshot = SseOutcome { model: Some("gpt-6-luna-2026-10-01".into()), ..ok.clone() };
        assert!(verify_model(&snapshot).is_ok(), "a dated snapshot of the same model is the same model");
        for (model, effort, needle) in [
            (Some("gpt-6-astra"), Some("high"), "instead of gpt-6-luna"),
            (Some("gpt-6-lunar"), Some("high"), "instead of gpt-6-luna"),
            (None, Some("high"), "did not report which model"),
            (Some("gpt-6-luna"), Some("medium"), "instead of high"),
            (Some("gpt-6-luna"), None, "did not report the reasoning effort"),
        ] {
            let o = SseOutcome { model: model.map(String::from), effort: effort.map(String::from), ..ok.clone() };
            let e = verify_model(&o).unwrap_err();
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
        assert!(e.message.contains("instead of high"), "{}", e.message);
    }

    #[test]
    fn the_request_matches_what_upstream_sent() {
        let d = dir();
        let p = provider(&d, Arc::new(FixtureTransport::new()), None, 0);
        let mut r = request();
        r.edit = Some(EditSpec { base_png: vec![1], mask_png: None });
        let body = p.build_body(&r, Some((928, 816)));
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
