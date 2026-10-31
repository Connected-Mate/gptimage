//! The provider: refresh the token, call the undocumented responses endpoint, read the image out of the stream.
//!
//! Rust port of Connected-Mate/gptimage `src/codex.js` (MIT, commit dfb4948). The request is reconstructed from
//! that file; it has never been sent from this repository (no test or tool here contacts the backend).

use std::sync::Arc;

use imagegen_core::config::EnvSource;
use imagegen_core::error::RetryClass;
use imagegen_core::hashing::{Clock, SystemClock};
use imagegen_providers::common::{b64, b64_decode, data_url, status_error};
use imagegen_providers::sizing::choose_size;
use imagegen_providers::transport::{send, HostPolicy, HttpRequest, HttpTransport};
use imagegen_providers::types::{
    AlphaSupport, CallContext, CostEstimate, CostModel, ErrorKind, Funding, ImageProvider, ImageRequest, ModelInfo, ModelStatus,
    ProviderDescriptor, ProviderError, ProviderOutput, SizeRule, TABLES_RETRIEVED,
};
use serde_json::{json, Value};

use crate::oauth::{self, account_id, expiry_ms, parse_tokens, refresh_request};
use crate::store::{default_path, CredentialStore, Credentials};
use crate::{BACKEND_URL, DEFAULT_MODEL, HOSTS, ORIGINATOR};

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
}

impl ChatGptLoginProvider {
    /// Build from the environment: the credential file location comes from [`default_path`].
    pub fn from_environment(env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) -> Self {
        Self::new(CredentialStore::at(default_path(env)), transport, Arc::new(SystemClock), DEFAULT_MODEL.to_string())
    }

    /// Build with explicit parts (tests).
    pub fn new(store: CredentialStore, transport: Arc<dyn HttpTransport>, clock: Arc<dyn Clock>, model: String) -> Self {
        let desc = ProviderDescriptor {
            id: "chatgpt_login".into(),
            display_name: "ChatGPT login (UNOFFICIAL, personal use only)".into(),
            doc_url: "docs/unofficial-chatgpt-login.md".into(),
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
                notes: "Not a stable image model: the slug carries the hosted image tool of an undocumented backend and can rotate. No mask, no documented alpha (transparency is proven from pixels), no usage report.".into(),
            }],
            terms_note: "Unofficial. Impersonates the Codex CLI against an undocumented endpoint; may break or be blocked without notice; account consequences are yours. Not for servers, not for sharing tokens.".into(),
        };
        ChatGptLoginProvider { desc, store, transport, policy: HostPolicy::api(&HOSTS), clock, model }
    }

    /// A valid access token and account id, refreshing and saving when needed.
    fn credentials(&self) -> Result<Credentials, ProviderError> {
        let creds = self
            .store
            .load()
            .map_err(|e| ProviderError::new(ErrorKind::NotConfigured, RetryClass::RequiresAuthorization, e))?
            .ok_or_else(|| ProviderError::not_configured("not signed in: run `hoi4-imagegen chatgpt-login login`"))?;
        let now_ms = (self.clock.now_secs().max(0) as u64) * 1000;
        let expired = creds.expires_ms.map(|e| now_ms + EXPIRY_MARGIN_MS >= e).unwrap_or(false);
        if !expired {
            return Ok(creds);
        }
        let refresh = creds
            .refresh
            .clone()
            .ok_or_else(|| ProviderError::new(ErrorKind::Auth, RetryClass::RequiresAuthorization, "the access token expired and there is no refresh token: sign in again"))?;
        let resp = send(&*self.transport, &self.policy, &refresh_request(&refresh))?;
        if resp.status != 200 {
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
            "model": req.model,
            "instructions": INSTRUCTIONS,
            "input": [{"role": "user", "content": content}],
            "tools": [tool],
            "tool_choice": {"type": "image_generation"},
            "stream": true,
            "store": false,
        })
    }

    fn call(&self, req: &ImageRequest) -> Result<ProviderOutput, ProviderError> {
        if req.model != self.model {
            return Err(ProviderError::unsupported(format!("this provider serves '{}', not '{}'", self.model, req.model)));
        }
        let size = self.prepare(req).ok();
        let creds = self.credentials()?;
        let body = self.build_body(req, size);
        let mut http = HttpRequest::post_json(BACKEND_URL, &body)
            .header("Authorization", &format!("Bearer {}", creds.access))
            .header("originator", ORIGINATOR)
            .header("Accept", "text/event-stream");
        if let Some(id) = &creds.account_id {
            http = http.header("chatgpt-account-id", id);
        }
        let secrets = vec![creds.access.clone()];
        let resp = send(&*self.transport, &self.policy, &http)?;
        match resp.status {
            200 => {}
            401 => {
                return Err(ProviderError::new(ErrorKind::Auth, RetryClass::RequiresAuthorization, "the backend rejected the token (401): sign in again").with_status(401));
            }
            429 => {
                return Err(ProviderError::new(ErrorKind::RateLimited, RetryClass::SafeSameInputs, "rate limited by the ChatGPT plan (429): wait and retry").with_status(429));
            }
            s => return Err(status_error(s, &resp.body, &secrets)),
        }
        let text = String::from_utf8_lossy(&resp.body);
        let b64_png = parse_sse(&text).map_err(|e| ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, e))?;
        Ok(ProviderOutput {
            image: b64_decode(&b64_png)?,
            mime: "image/png".into(),
            provider_request_id: None,
            usage: None,
            actual_usd: None,
            usage_unknown: true,
            model_echo: Some(self.model.clone()),
            notes: vec!["generated through an unofficial backend; usage is not reported".into()],
            job: None,
        })
    }
}

/// Find the first image result in an SSE stream. Surfaces backend error events instead of waiting for the end.
pub fn parse_sse(stream: &str) -> Result<String, String> {
    let normalized = stream.replace("\r\n", "\n");
    for raw in normalized.split("\n\n") {
        let data: String = raw.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim).collect::<Vec<_>>().join("");
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&data) else { continue };
        match v.get("type").and_then(Value::as_str) {
            Some("response.output_item.done") => {
                if v.pointer("/item/type").and_then(Value::as_str) == Some("image_generation_call") {
                    if let Some(result) = v.pointer("/item/result").and_then(Value::as_str) {
                        return Ok(result.to_string());
                    }
                }
            }
            Some("response.failed") | Some("error") => {
                let msg = v.pointer("/response/error/message").or_else(|| v.pointer("/error/message")).and_then(Value::as_str).unwrap_or("unknown backend error");
                return Err(format!("backend error: {msg}"));
            }
            _ => {}
        }
    }
    Err("the stream ended without an image_generation result".into())
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
            basis: "ChatGPT plan quota: there is no per-image price and no usage report, so spend is recorded as unknown and no operator allowance is ever charged".into(),
            usage_unknown: true,
            funding: Funding::PlanQuota,
        })
    }

    fn generate(&self, req: &ImageRequest, _ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.edit.is_some() {
            return Err(ProviderError::unsupported("generate was called with an edit request"));
        }
        self.call(req)
    }

    fn edit(&self, req: &ImageRequest, _ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.edit.is_none() {
            return Err(ProviderError::unsupported("edit was called without a base image"));
        }
        if req.edit.as_ref().map(|e| e.mask_png.is_some()).unwrap_or(false) {
            return Err(ProviderError::unsupported("this provider has no mask support"));
        }
        self.call(req)
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
            model: DEFAULT_MODEL.into(),
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
        ChatGptLoginProvider::new(store, fixture, Arc::new(FixedClock(now)), DEFAULT_MODEL.into())
    }

    fn creds(expires_ms: Option<u64>) -> Credentials {
        Credentials { access: "access-synthetic-1".into(), refresh: Some("refresh-synthetic-1".into()), account_id: Some("acct-synthetic".into()), expires_ms, last_refresh: "t".into() }
    }

    fn sse(png_b64: &str) -> String {
        format!(
            "event: response.created\ndata: {{\"type\":\"response.created\"}}\n\n: keepalive\n\ndata: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"image_generation_call\",\"result\":\"{png_b64}\"}}}}\n\ndata: [DONE]\n\n"
        )
    }

    #[test]
    fn sse_parsing_finds_the_image_and_surfaces_errors() {
        assert_eq!(parse_sse(&sse("QUJD")).unwrap(), "QUJD");
        assert_eq!(parse_sse(&sse("QUJD").replace('\n', "\r\n")).unwrap(), "QUJD", "CRLF streams parse too");
        assert!(parse_sse("data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"quota\"}}}\n\n").unwrap_err().contains("quota"));
        assert!(parse_sse("data: {\"type\":\"error\",\"error\":{\"message\":\"boom\"}}\n\n").unwrap_err().contains("boom"));
        assert!(parse_sse("data: [DONE]\n\n").is_err());
        assert!(parse_sse("data: not json\n\n").is_err());
    }

    #[test]
    fn the_request_matches_what_upstream_sent() {
        let d = dir();
        let p = provider(&d, Arc::new(FixtureTransport::new()), None, 0);
        let mut r = request();
        r.edit = Some(EditSpec { base_png: vec![1], mask_png: None });
        let body = p.build_body(&r, Some((928, 816)));
        assert_eq!(body["model"], "gpt-5.5");
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
        fixture.respond(Method::Post, "/oauth/token", 200, &[], br#"{"access_token":"access-synthetic-2","refresh_token":"refresh-synthetic-2","expires_in":3600}"#.to_vec());
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
        assert_eq!(p.generate(&request(), &CallContext::new()).unwrap_err().kind, ErrorKind::RateLimited);
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
