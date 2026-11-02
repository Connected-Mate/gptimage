//! Ideogram API v2 (`ideogram`): synchronous generation with URL results.
//!
//! Built from the vendor documentation read on 2026-11-02 and **not live-verified**: no key was available and no
//! call was made. The v2 API is current. The `v1/ideogram-v3/generate` route in the older docs is marked legacy.
//! Auth is the `Api-Key` header. Images come back as `data[].url`, which expires, so the adapter downloads it at
//! once from `ideogram.ai` hosts without sending the key. `is_image_safe: false` (an empty url) is a refusal.
//!
//! Routes used: `generate/ideogram-4-5` (multipart, also the edit route when `images` is sent, with
//! `background=transparent` for alpha), `generate/ideogram-4` (JSON), `generate/ideogram-4-transparent` (JSON)
//! and `generate/ideogram-3` (multipart, with `style_reference_images`). Prices are not published in the docs.

use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource, ProviderConfig};
use imagegen_core::error::RetryClass;
use imagegen_core::refs::RefRole;
use serde_json::{json, Value};

use crate::common::{check_model, descriptor_from, per_image_price, sniff_mime, status_error, ApiKey};
use crate::registry::{policy_for, read_key, ProviderRegistry};
use crate::sizing::{choose_size, nearest_ratio, tier_size};
use crate::stability::Multipart;
use crate::transport::{redact, send, HostPolicy, HttpRequest, HttpTransport};
use crate::types::{
    CallContext, CostEstimate, ErrorKind, Funding, ImageProvider, ImageRequest, ModelInfo, ProviderDescriptor, ProviderError, ProviderOutput,
    SizeRule,
};

/// Default API base.
pub const DEFAULT_BASE: &str = "https://api.ideogram.ai";

const MAX_SEED: u64 = 2_147_483_647;

/// What goes on the wire.
#[derive(Debug, Clone)]
pub(crate) enum Wire {
    Json(Value),
    Form(Multipart),
}

/// Adapter state.
pub struct IdeogramProvider {
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    base: String,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
}

impl IdeogramProvider {
    /// Build the adapter.
    pub fn new(cfg: ProviderConfig, key: Option<String>, transport: Arc<dyn HttpTransport>, policy: HostPolicy) -> Self {
        let base = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_BASE.to_string()).trim_end_matches('/').to_string();
        IdeogramProvider { desc: descriptor_from(include_str!("../data/ideogram.json")), cfg, key: key.map(ApiKey::new), base, transport, policy }
    }

    fn secrets(&self) -> Vec<String> {
        self.key.iter().map(|k| k.expose().to_string()).collect()
    }

    fn quality(&self, req: &ImageRequest) -> String {
        req.quality.clone().or_else(|| self.cfg.quality.clone()).unwrap_or_else(|| "medium".to_string())
    }

    fn speed(&self, req: &ImageRequest) -> &'static str {
        match self.quality(req).as_str() {
            "low" => "turbo",
            "high" => "quality",
            _ => "default",
        }
    }

    fn check_inputs<'a>(&'a self, req: &ImageRequest) -> Result<&'a ModelInfo, ProviderError> {
        let model = check_model(&self.desc, req)?;
        let id = req.model.as_str();
        match id {
            "ideogram-4-5" if req.edit.is_none() && !req.refs.is_empty() => Err(ProviderError::unsupported(
                "ideogram-4-5 treats the first image it receives as the one to edit, so references cannot guide a plain generation. Use ideogram-3 for style anchors or send an edit",
            )),
            "ideogram-3" if req.edit.is_some() => Err(ProviderError::unsupported("ideogram-3 is offered for generation with style references only. Use ideogram-4-5 to edit")),
            "ideogram-3" if req.refs.iter().any(|r| r.role != RefRole::StyleAnchor) => {
                Err(ProviderError::unsupported("ideogram-3 takes style anchor references only (style_reference_images)"))
            }
            _ => Ok(model),
        }
    }

    fn ratio(&self, model: &ModelInfo, req: &ImageRequest) -> Result<String, ProviderError> {
        match &model.sizes {
            SizeRule::AspectTiers { ratios, .. } => {
                nearest_ratio(ratios, [req.width, req.height]).ok_or_else(|| ProviderError::unsupported("no supported aspect ratio"))
            }
            _ => Err(ProviderError::unsupported("the model table has no aspect ratios")),
        }
    }

    /// Route and body for a request. Used by contract tests to check the wire format.
    pub(crate) fn build(&self, req: &ImageRequest) -> Result<(String, Wire), ProviderError> {
        let model = self.check_inputs(req)?;
        let seed = req.seed.map(|s| s % (MAX_SEED + 1));
        match req.model.as_str() {
            "ideogram-4-5" => {
                let mut f = Multipart::new();
                f.text("prompt", req.prompt.clone());
                f.text("magic_prompt", "off");
                f.text("num_images", "1");
                if let Some(edit) = &req.edit {
                    f.file("images", "base.png", "image/png", &edit.base_png);
                    for (i, r) in req.refs.iter().enumerate() {
                        f.file("images", &format!("ref{i}.png"), "image/png", &r.png);
                    }
                    f.text("size", "source");
                    f.text(
                        "quality",
                        match self.quality(req).as_str() {
                            "low" => "very_low",
                            "high" => "high",
                            _ => "medium",
                        },
                    );
                } else {
                    let (w, h) = choose_size(&model.sizes, [req.width, req.height])?;
                    f.text("size", format!("{w}x{h}"));
                    f.text(
                        "quality",
                        match self.quality(req).as_str() {
                            "high" => "high",
                            _ => "medium",
                        },
                    );
                }
                if req.transparent {
                    f.text("background", "transparent");
                }
                if let Some(s) = seed {
                    f.text("seed", s.to_string());
                }
                Ok(("/v2/image/generate/ideogram-4-5".into(), Wire::Form(f)))
            }
            "ideogram-4" => {
                let (w, h) = choose_size(&model.sizes, [req.width, req.height])?;
                let mut body = json!({
                    "prompt": req.prompt,
                    "magic_prompt": "off",
                    "num_images": 1,
                    "resolution": format!("{w}x{h}"),
                    "rendering_speed": self.speed(req),
                });
                if let Some(s) = seed {
                    body["seed"] = json!(s);
                }
                Ok(("/v2/image/generate/ideogram-4".into(), Wire::Json(body)))
            }
            "ideogram-4-transparent" => {
                let ratio = self.ratio(model, req)?.replace(':', "x");
                let mut body = json!({
                    "prompt": req.prompt,
                    "magic_prompt": "off",
                    "num_images": 1,
                    "aspect_ratio": ratio,
                    "output_resolution": "1k",
                    "rendering_speed": self.speed(req),
                });
                if let Some(s) = seed {
                    body["seed"] = json!(s);
                }
                Ok(("/v2/image/generate/ideogram-4-transparent".into(), Wire::Json(body)))
            }
            _ => {
                let mut f = Multipart::new();
                f.text("prompt", req.prompt.clone());
                f.text("magic_prompt", "off");
                f.text("num_images", "1");
                f.text("aspect_ratio", self.ratio(model, req)?.replace(':', "x"));
                f.text("rendering_speed", self.speed(req));
                if !req.negative.is_empty() {
                    f.text("negative_prompt", req.negative.join(", "));
                }
                if let Some(s) = seed {
                    f.text("seed", s.to_string());
                }
                for (i, r) in req.refs.iter().enumerate() {
                    f.file("style_reference_images", &format!("style{i}.png"), "image/png", &r.png);
                }
                Ok(("/v2/image/generate/ideogram-3".into(), Wire::Form(f)))
            }
        }
    }

    fn map_error(&self, status: u16, body: &[u8]) -> ProviderError {
        let secrets = self.secrets();
        let reason = serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("reject_reason").and_then(Value::as_str).map(str::to_string));
        match status {
            402 => {
                let mut e = status_error(429, body, &secrets);
                e.retry = RetryClass::RequiresAuthorization;
                e.message = e.message.replacen("HTTP 429", "HTTP 402 (out of credits or quota)", 1);
                e.status = Some(402);
                e
            }
            422 => {
                let mut e = status_error(400, body, &secrets);
                e.kind = ErrorKind::ModerationBlocked;
                e.message = e.message.replacen("HTTP 400", "HTTP 422 (prompt failed safety checks)", 1);
                e.status = Some(422);
                e
            }
            429 => {
                let mut e = status_error(429, body, &secrets);
                if matches!(
                    reason.as_deref(),
                    Some("insufficient_funds" | "subscription_required" | "daily_limit" | "priority_credit_required" | "feature_limit")
                ) {
                    e.retry = RetryClass::RequiresAuthorization;
                }
                e
            }
            _ => status_error(status, body, &secrets),
        }
    }

    fn call(&self, req: &ImageRequest) -> Result<ProviderOutput, ProviderError> {
        let key = self.key.as_ref().ok_or_else(|| ProviderError::not_configured("IDEOGRAM_API_KEY is not set"))?;
        let (path, wire) = self.build(req)?;
        let url = format!("{}{path}", self.base);
        let http = match wire {
            Wire::Json(body) => HttpRequest::post_json(url, &body),
            Wire::Form(form) => form.into_request(url),
        }
        .header("Api-Key", key.expose());
        let resp = send(&*self.transport, &self.policy, &http)?;
        if resp.status != 200 {
            return Err(self.map_error(resp.status, &resp.body));
        }
        let secrets = self.secrets();
        let malformed = |why: &str| {
            ProviderError::new(
                ErrorKind::MalformedResponse,
                RetryClass::ReconcileFirst,
                format!("{why}. The request was accepted, so a charge may have been made: check the account before repeating it"),
            )
            .with_status(200)
        };
        let v: Value = serde_json::from_slice(&resp.body).map_err(|e| malformed(&format!("the response is not valid JSON ({e})")))?;
        let item = v.pointer("/data/0").ok_or_else(|| malformed("the response has no data[0]"))?;
        let url = item.get("url").and_then(Value::as_str).unwrap_or("");
        if item.get("is_image_safe").and_then(Value::as_bool) == Some(false) || url.is_empty() {
            let why = if item.get("is_image_safe").and_then(Value::as_bool) == Some(false) {
                "Ideogram marked the image unsafe (is_image_safe false) and returned no url"
            } else {
                return Err(malformed("data[0] has no url"));
            };
            return Err(ProviderError::new(ErrorKind::ModerationBlocked, RetryClass::Terminal, why).with_status(200));
        }
        // The image link is a public, expiring URL: the key is not sent to it.
        let dl = send(&*self.transport, &self.policy, &HttpRequest::get(url)).map_err(|mut e| {
            e.message = format!(
                "{} (the image was generated and may be billed: fetch it again from the same link before it expires, or reconcile)",
                e.message
            );
            e.retry = RetryClass::ReconcileFirst;
            e
        })?;
        if dl.status != 200 {
            let mut e = status_error(dl.status, &dl.body, &secrets);
            e.retry = RetryClass::ReconcileFirst;
            return Err(e);
        }
        let mime = dl.header("content-type").map(|c| c.split(';').next().unwrap_or("").trim().to_string()).filter(|c| c.starts_with("image/"));
        let mime = mime.unwrap_or_else(|| sniff_mime(&dl.body).to_string());
        if mime == "application/octet-stream" {
            return Err(malformed("the downloaded file is not a PNG, JPEG or WebP image"));
        }
        let mut notes = Vec::new();
        if let Some(p) = item.get("prompt").and_then(Value::as_str) {
            notes.push(format!("final prompt: {}", redact(p, &secrets)));
        }
        if !req.negative.is_empty() && !matches!(req.model.as_str(), "ideogram-3") {
            notes.push("negative prompt not sent: this endpoint documents no negative_prompt".to_string());
        }
        // the table price is a stand-in, so the cost is never reported as an actual figure
        Ok(ProviderOutput {
            image: dl.body,
            mime,
            provider_request_id: v.get("generation_id").and_then(Value::as_str).map(str::to_string),
            usage: None,
            actual_usd: None,
            usage_unknown: true,
            model_echo: Some(req.model.clone()),
            notes,
            job: None,
        })
    }
}

impl ImageProvider for IdeogramProvider {
    fn id(&self) -> &str {
        "ideogram"
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
        let _ = editing;
        self.desc.recommended().map(|m| m.id.clone()).unwrap_or_else(|| "ideogram-4-5".into())
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        let model = self.check_inputs(req)?;
        match &model.sizes {
            SizeRule::Fixed { .. } => choose_size(&model.sizes, [req.width, req.height]),
            _ => Ok(tier_size(&self.ratio(model, req)?)),
        }
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        let model = self.check_inputs(req)?;
        let (usd, _) =
            per_image_price(&model.cost, None).ok_or_else(|| ProviderError::unsupported(format!("{} has no price in the table", model.id)))?;
        Ok(CostEstimate {
            usd,
            basis: format!(
                "Ideogram table retrieved {}: no per-image price is published in the API docs, so {usd} USD is a planning stand-in set in this table, not a vendor figure. Credits cost 1 cent each. dry_run=true returns the exact quote",
                self.desc.retrieved
            ),
            usage_unknown: true,
            funding: Funding::UserKey,
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
        self.call(req)
    }
}

/// Register this module's providers unless the project config disables them.
pub fn register(reg: &mut ProviderRegistry, config: &Config, env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) {
    if config.provider("ideogram").enabled == Some(false) {
        return;
    }
    let cfg = config.provider("ideogram");
    match policy_for(&["api.ideogram.ai", ".ideogram.ai"], cfg.base_url.as_deref(), env) {
        Ok(policy) => {
            let key = read_key("ideogram", &cfg, env);
            reg.insert(Arc::new(IdeogramProvider::new(cfg, key, transport, policy)));
        }
        Err(why) => reg.problem("ideogram", why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{FixtureTransport, Method};
    use crate::types::{EditSpec, ProviderRef};
    use imagegen_core::config::MapEnv;
    use imagegen_core::pixels::{encode_png, Rgba};

    // DOCUMENTED SHAPE, NOT RECORDED: bodies below follow the field and response tables on developer.ideogram.ai
    // (read 2026-11-02). Nothing was returned by Ideogram.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [10, 200, 30, 255])).unwrap()
    }

    fn provider(fixture: Arc<FixtureTransport>, key: Option<&str>) -> IdeogramProvider {
        IdeogramProvider::new(ProviderConfig::default(), key.map(str::to_string), fixture, HostPolicy::api(&["api.ideogram.ai", ".ideogram.ai"]))
    }

    fn request(model: &str) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "an iron wall".into(),
            negative: vec!["text".into()],
            width: 100,
            height: 88,
            transparent: false,
            quality: None,
            refs: vec![],
            edit: None,
            idempotency_key: None,
            seed: Some(11),
        }
    }

    fn style(n: usize) -> Vec<ProviderRef> {
        (0..n).map(|_| ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }).collect()
    }

    fn api_ok(url: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"generation_id": "gen-doc-1", "seed": 11, "data": [{"prompt": "final prompt", "resolution": "1024x1024", "is_image_safe": true, "seed": 11, "url": url}]}))
            .unwrap()
    }

    fn fixture_ok(path: &str) -> Arc<FixtureTransport> {
        let f = Arc::new(FixtureTransport::new());
        f.respond(Method::Post, path, 200, &[], api_ok("https://ideogram.ai/api/images/ephemeral/abc.png"));
        f.respond(Method::Get, "ideogram.ai/api/images/ephemeral/", 200, &[("content-type", "image/png")], png());
        f
    }

    #[test]
    fn documented_shape_45_generation_form_with_transparent_background() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("id-test-000000"));
        let mut r = request("ideogram-4-5");
        r.transparent = true;
        let Wire::Form(f) = p.build(&r).unwrap().1 else { panic!("form expected") };
        assert_eq!(f.get("background").as_deref(), Some("transparent"));
        assert_eq!(f.get("size").as_deref(), Some("1120x896"));
        assert_eq!(f.get("quality").as_deref(), Some("medium"));
        assert_eq!(f.get("magic_prompt").as_deref(), Some("off"));
        assert_eq!(f.get("seed").as_deref(), Some("11"));
        assert!(f.file_names().is_empty() && f.get("negative_prompt").is_none() && f.get("mask").is_none());
        assert_eq!(p.prepare(&r).unwrap(), (1120, 896));
    }

    #[test]
    fn documented_shape_45_edit_sends_base_first_then_references() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("id-test-000000"));
        let mut r = request("ideogram-4-5");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        r.refs = style(2);
        let (path, Wire::Form(f)) = p.build(&r).unwrap() else { panic!("form expected") };
        assert_eq!(path, "/v2/image/generate/ideogram-4-5");
        assert_eq!(f.file_names(), vec!["images", "images", "images"]);
        assert_eq!(f.get("size").as_deref(), Some("source"));
        r.quality = Some("low".into());
        let Wire::Form(f) = p.build(&r).unwrap().1 else { panic!() };
        assert_eq!(f.get("quality").as_deref(), Some("very_low"));
        // very_low needs a source image, so a plain generation never asks for it
        let mut g = request("ideogram-4-5");
        g.quality = Some("low".into());
        let Wire::Form(f) = p.build(&g).unwrap().1 else { panic!() };
        assert_eq!(f.get("quality").as_deref(), Some("medium"));
    }

    #[test]
    fn documented_shape_4_and_transparent_are_json() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("id-test-000000"));
        let mut r = request("ideogram-4");
        r.quality = Some("high".into());
        let (path, Wire::Json(b)) = p.build(&r).unwrap() else { panic!("json expected") };
        assert_eq!(path, "/v2/image/generate/ideogram-4");
        assert_eq!((b["resolution"].as_str(), b["rendering_speed"].as_str(), b["seed"].as_u64()), (Some("1120x896"), Some("quality"), Some(11)));
        let (path, Wire::Json(b)) = p.build(&request("ideogram-4-transparent")).unwrap() else { panic!("json expected") };
        assert_eq!(path, "/v2/image/generate/ideogram-4-transparent");
        assert_eq!(
            (b["aspect_ratio"].as_str(), b["output_resolution"].as_str(), b["rendering_speed"].as_str()),
            (Some("5x4"), Some("1k"), Some("default"))
        );
        assert!(b.get("negative_prompt").is_none());
    }

    #[test]
    fn documented_shape_3_sends_negative_prompt_and_style_references() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("id-test-000000"));
        let mut r = request("ideogram-3");
        r.refs = style(2);
        r.quality = Some("low".into());
        let (path, Wire::Form(f)) = p.build(&r).unwrap() else { panic!("form expected") };
        assert_eq!(path, "/v2/image/generate/ideogram-3");
        assert_eq!(f.get("negative_prompt").as_deref(), Some("text"));
        assert_eq!(f.get("aspect_ratio").as_deref(), Some("5x4"));
        assert_eq!(f.get("rendering_speed").as_deref(), Some("turbo"));
        assert_eq!(f.file_names(), vec!["style_reference_images", "style_reference_images"]);
    }

    #[test]
    fn unsupported_combinations_fail_before_any_request() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider(fixture.clone(), Some("id-test-000000"));
        let ctx = CallContext::new();
        let mut r = request("ideogram-4-5");
        r.refs = style(1);
        assert!(p.generate(&r, &ctx).unwrap_err().message.contains("first image"));
        let mut r = request("ideogram-4");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        assert_eq!(p.edit(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        let mut r = request("ideogram-3");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        assert_eq!(p.edit(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        let mut r = request("ideogram-3");
        r.refs = vec![ProviderRef { role: RefRole::GeometryAnchor, png: png(), sha256: "x".into() }];
        assert_eq!(p.generate(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        let mut r = request("ideogram-4-5");
        r.edit = Some(EditSpec { base_png: png(), mask_png: Some(png()) });
        assert!(p.edit(&r, &ctx).unwrap_err().message.contains("mask"));
        assert_eq!(p.generate(&request("ideogram-v3"), &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn documented_shape_success_sends_the_api_key_header_and_downloads_without_it() {
        let fixture = fixture_ok("/v2/image/generate/ideogram-4-5");
        let p = provider(fixture.clone(), Some("id-test-000000"));
        let out = p.generate(&request("ideogram-4-5"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!(out.provider_request_id.as_deref(), Some("gen-doc-1"));
        assert!(out.usage_unknown && out.actual_usd.is_none(), "no published price, so no actual cost");
        assert!(out.notes.iter().any(|n| n.contains("final prompt: final prompt")));
        assert!(out.notes.iter().any(|n| n.contains("negative prompt not sent")));
        let reqs = fixture.requests();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].url, "https://api.ideogram.ai/v2/image/generate/ideogram-4-5");
        assert!(reqs[0].headers.iter().any(|(k, v)| k == "Api-Key" && v == "id-test-000000"));
        assert!(reqs[0].headers.iter().any(|(k, v)| k == "Content-Type" && v.starts_with("multipart/form-data; boundary=")));
        assert!(reqs[1].headers.is_empty(), "the key must not go to the image host");
    }

    #[test]
    fn documented_shape_json_route_sends_json() {
        let fixture = fixture_ok("/v2/image/generate/ideogram-4-transparent");
        let p = provider(fixture.clone(), Some("id-test-000000"));
        p.generate(&request("ideogram-4-transparent"), &CallContext::new()).unwrap();
        let sent = &fixture.requests()[0];
        assert!(sent.headers.iter().any(|(k, v)| k == "Content-Type" && v == "application/json"));
        let v: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(v["output_resolution"], "1k");
    }

    #[test]
    fn documented_shape_unsafe_image_is_a_refusal_and_download_hosts_are_checked() {
        let fixture = Arc::new(FixtureTransport::new());
        let body = json!({"generation_id": "g", "seed": 1, "data": [{"prompt": "p", "resolution": "1024x1024", "is_image_safe": false, "seed": 1, "url": ""}]});
        fixture.respond(Method::Post, "/v2/", 200, &[], serde_json::to_vec(&body).unwrap());
        let p = provider(fixture, Some("id-test-000000"));
        let err = p.generate(&request("ideogram-4"), &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.retry), (ErrorKind::ModerationBlocked, RetryClass::Terminal));
        // an image link on a foreign host is blocked, and the error says the image may be billed
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/v2/", 200, &[], api_ok("https://evil.example/x.png"));
        let p = provider(fixture.clone(), Some("id-test-000000"));
        let err = p.generate(&request("ideogram-4"), &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.retry), (ErrorKind::Blocked, RetryClass::ReconcileFirst));
        assert!(err.message.contains("billed") && fixture.requests().len() == 1);
    }

    #[test]
    fn documented_shape_errors_map_and_never_leak_the_key() {
        let key = "id-test-secret-123456";
        let cases: [(u16, String, ErrorKind, RetryClass); 8] = [
            (401, format!(r#"{{"error":"bad key {key}"}}"#), ErrorKind::Auth, RetryClass::RequiresAuthorization),
            (402, r#"{"error":"no credits","reject_reason":"insufficient_funds"}"#.into(), ErrorKind::RateLimited, RetryClass::RequiresAuthorization),
            (422, r#"{"error":"prompt failed the safety check"}"#.into(), ErrorKind::ModerationBlocked, RetryClass::Terminal),
            (400, r#"{"error":"bad resolution"}"#.into(), ErrorKind::BadRequest, RetryClass::Terminal),
            (
                429,
                r#"{"error":"busy","reject_reason":"inflight_limit","max_inflight_requests":10}"#.into(),
                ErrorKind::RateLimited,
                RetryClass::SafeSameInputs,
            ),
            (429, r#"{"error":"daily","reject_reason":"daily_limit"}"#.into(), ErrorKind::RateLimited, RetryClass::RequiresAuthorization),
            (500, "boom".into(), ErrorKind::ServerError, RetryClass::ReconcileFirst),
            (503, "later".into(), ErrorKind::ServerError, RetryClass::ReconcileFirst),
        ];
        for (status, body, kind, retry) in cases {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/v2/", status, &[], body);
            let p = provider(fixture, Some(key));
            let err = p.generate(&request("ideogram-4"), &CallContext::new()).unwrap_err();
            assert_eq!((err.kind, err.retry), (kind, retry), "status {status}");
            assert!(!err.message.contains(key), "key leaked: {}", err.message);
            assert_eq!(err.status, Some(status));
        }
    }

    #[test]
    fn documented_shape_malformed_success_and_bad_downloads_are_not_silent() {
        for body in [b"nope".to_vec(), br#"{"data":[]}"#.to_vec(), br#"{"data":[{"is_image_safe":true}]}"#.to_vec()] {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/v2/", 200, &[], body);
            let p = provider(fixture, Some("id-test-000000"));
            let err = p.generate(&request("ideogram-4"), &CallContext::new()).unwrap_err();
            assert_eq!((err.kind, err.retry), (ErrorKind::MalformedResponse, RetryClass::ReconcileFirst));
        }
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/v2/", 200, &[], api_ok("https://ideogram.ai/x.png"));
        fixture.respond(Method::Get, "ideogram.ai/x.png", 200, &[], b"html page".to_vec());
        let p = provider(fixture, Some("id-test-000000"));
        assert_eq!(p.generate(&request("ideogram-4"), &CallContext::new()).unwrap_err().kind, ErrorKind::MalformedResponse);
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/v2/", 200, &[], api_ok("https://ideogram.ai/x.png"));
        fixture.respond(Method::Get, "ideogram.ai/x.png", 404, &[], b"gone".to_vec());
        let p = provider(fixture, Some("id-test-000000"));
        assert_eq!(p.generate(&request("ideogram-4"), &CallContext::new()).unwrap_err().retry, RetryClass::ReconcileFirst);
    }

    #[test]
    fn missing_key_fails_first_and_a_foreign_base_is_blocked() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider(fixture.clone(), None);
        assert!(!p.is_configured());
        assert_eq!(p.generate(&request("ideogram-4"), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);
        let cfg = ProviderConfig { base_url: Some("https://evil.example".into()), ..Default::default() };
        let p = IdeogramProvider::new(cfg, Some("id-test-000000".into()), fixture.clone(), HostPolicy::api(&["api.ideogram.ai"]));
        assert_eq!(p.generate(&request("ideogram-4"), &CallContext::new()).unwrap_err().kind, ErrorKind::Blocked);
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn estimates_are_labelled_stand_ins_never_vendor_prices() {
        let p = provider(Arc::new(FixtureTransport::new()), None);
        for m in ["ideogram-4-5", "ideogram-4", "ideogram-4-transparent", "ideogram-3"] {
            let e = p.estimate(&request(m)).unwrap();
            assert!(e.usage_unknown, "{m}");
            assert!(e.basis.contains("stand-in") && e.basis.contains("not a vendor figure") && e.basis.contains("2026-11-02"), "{}", e.basis);
        }
    }

    #[test]
    fn table_is_marked_not_live_verified_and_lists_the_documented_transparency() {
        let raw: Value = serde_json::from_str(include_str!("../data/ideogram.json")).unwrap();
        assert_eq!(raw["live_verified"], json!(false));
        assert!(raw["doc_urls"].as_array().map(|a| a.len() > 3).unwrap_or(false) && raw["checked"] == json!("2026-11-02"));
        let d = descriptor_from(include_str!("../data/ideogram.json"));
        assert_eq!(d.models.iter().filter(|m| m.recommended).count(), 1);
        assert!(d.terms_note.contains("Not live-verified") && d.models.iter().all(|m| m.notes.contains("Not live-verified")));
        use crate::types::AlphaSupport;
        assert_eq!(d.model("ideogram-4-5").unwrap().alpha, AlphaSupport::Ga);
        assert_eq!(d.model("ideogram-4-transparent").unwrap().alpha, AlphaSupport::Ga);
        assert_eq!(d.model("ideogram-4").unwrap().alpha, AlphaSupport::None);
        assert_eq!(provider(Arc::new(FixtureTransport::new()), None).default_model(true), "ideogram-4-5");
    }

    #[test]
    fn registry_builds_it_and_allows_its_image_hosts() {
        let fixture: Arc<dyn HttpTransport> = Arc::new(FixtureTransport::new());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::from_pairs(&[("IDEOGRAM_API_KEY", "id-test-1")]), fixture.clone());
        assert!(reg.get("ideogram").unwrap().is_configured());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::default(), fixture.clone());
        assert!(!reg.get("ideogram").unwrap().is_configured());
        let policy = policy_for(&["api.ideogram.ai", ".ideogram.ai"], None, &MapEnv::default()).unwrap();
        assert!(policy.check("https://ideogram.ai/api/images/x.png").is_ok() && policy.check("https://cdn.ideogram.ai/x.png").is_ok());
        assert!(policy.check("https://notideogram.ai/x.png").is_err());
        let cfg = Config::parse("[providers.ideogram]\nenabled = false\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &cfg, &MapEnv::default(), fixture);
        assert!(reg.get("ideogram").is_none());
    }
}
