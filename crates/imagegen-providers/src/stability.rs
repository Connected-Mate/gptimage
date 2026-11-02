//! Stability AI REST API (`stability`): synchronous Stable Image generation.
//!
//! Built from the vendor documentation read on 2026-11-02 and **not live-verified**: no key was available and no
//! call was made. Every request is `multipart/form-data` with `Authorization: Bearer` and `Accept: image/*`, so
//! the answer is the raw image with `finish-reason`, `seed` and `x-request-id` headers. A `finish-reason` of
//! `CONTENT_FILTERED` means Stability blurred the image, which is refused here instead of being returned.
//!
//! Routes: `generate/core`, `generate/ultra` and `generate/sd3` for generation. An edit is an image-to-image
//! call (ultra, sd3.5) or a control call (`control/structure` keeps the base image's structure). One geometry or
//! style anchor reference maps to `control/structure` and `control/style`. Native transparency is not documented.
//! The multipart encoder here is shared with the other form-based adapters.

use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource, ProviderConfig};
use imagegen_core::error::RetryClass;
use imagegen_core::refs::RefRole;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::common::{b64_decode, check_model, descriptor_from, per_image_price, sniff_mime, status_error, ApiKey};
use crate::registry::{policy_for, read_key, ProviderRegistry};
use crate::sizing::{nearest_ratio, tier_size};
use crate::transport::{redact, send, HostPolicy, HttpRequest, HttpTransport, Method};
use crate::types::{
    CallContext, CostEstimate, ErrorKind, Funding, ImageProvider, ImageRequest, ProviderDescriptor, ProviderError, ProviderOutput, SizeRule,
};

/// Default API base.
pub const DEFAULT_BASE: &str = "https://api.stability.ai";

/// Image-to-image strength used for edits (an adapter default: 0 returns the input, 1 ignores it).
pub const EDIT_STRENGTH: &str = "0.65";

const MAX_SEED: u64 = 4_294_967_294;

/// A `multipart/form-data` body under construction. The boundary is the hash of the content, so it cannot
/// occur inside a part and a test can compare bodies byte for byte.
#[derive(Debug, Default, Clone)]
pub(crate) struct Multipart {
    parts: Vec<Part>,
}

#[derive(Debug, Clone)]
struct Part {
    name: String,
    filename: Option<String>,
    mime: Option<String>,
    data: Vec<u8>,
}

impl Multipart {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A text field.
    pub(crate) fn text(&mut self, name: &str, value: impl Into<String>) -> &mut Self {
        self.parts.push(Part { name: name.into(), filename: None, mime: None, data: value.into().into_bytes() });
        self
    }

    /// A file field.
    pub(crate) fn file(&mut self, name: &str, filename: &str, mime: &str, data: &[u8]) -> &mut Self {
        self.parts.push(Part { name: name.into(), filename: Some(filename.into()), mime: Some(mime.into()), data: data.to_vec() });
        self
    }

    /// Text fields as `(name, value)`, for tests. Files are left out.
    #[cfg(test)]
    pub(crate) fn text_fields(&self) -> Vec<(String, String)> {
        self.parts.iter().filter(|p| p.filename.is_none()).map(|p| (p.name.clone(), String::from_utf8_lossy(&p.data).into_owned())).collect()
    }

    /// Names of the file fields.
    #[cfg(test)]
    pub(crate) fn file_names(&self) -> Vec<String> {
        self.parts.iter().filter(|p| p.filename.is_some()).map(|p| p.name.clone()).collect()
    }

    /// Value of a text field.
    #[cfg(test)]
    pub(crate) fn get(&self, name: &str) -> Option<String> {
        self.text_fields().into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    /// Encode as a POST request body with its `Content-Type` header.
    pub(crate) fn into_request(self, url: impl Into<String>) -> HttpRequest {
        let mut hasher = Sha256::new();
        for p in &self.parts {
            hasher.update(p.name.as_bytes());
            hasher.update(&p.data);
        }
        let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let boundary = format!("hoi4imagegen{}", &digest[..32]);
        let mut body = Vec::new();
        for p in &self.parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            match (&p.filename, &p.mime) {
                (Some(f), Some(m)) => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{}\"; filename=\"{f}\"\r\nContent-Type: {m}\r\n\r\n", p.name).as_bytes(),
                ),
                _ => body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", p.name).as_bytes()),
            }
            body.extend_from_slice(&p.data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        HttpRequest {
            method: Method::Post,
            url: url.into(),
            headers: vec![("Content-Type".into(), format!("multipart/form-data; boundary={boundary}"))],
            body,
        }
    }
}

/// Adapter state.
pub struct StabilityProvider {
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    base: String,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
}

impl StabilityProvider {
    /// Build the adapter.
    pub fn new(cfg: ProviderConfig, key: Option<String>, transport: Arc<dyn HttpTransport>, policy: HostPolicy) -> Self {
        let base = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_BASE.to_string()).trim_end_matches('/').to_string();
        StabilityProvider { desc: descriptor_from(include_str!("../data/stability.json")), cfg, key: key.map(ApiKey::new), base, transport, policy }
    }

    fn secrets(&self) -> Vec<String> {
        self.key.iter().map(|k| k.expose().to_string()).collect()
    }

    fn ratio(&self, req: &ImageRequest) -> Result<String, ProviderError> {
        let model = check_model(&self.desc, req)?;
        match &model.sizes {
            SizeRule::AspectTiers { ratios, .. } => {
                nearest_ratio(ratios, [req.width, req.height]).ok_or_else(|| ProviderError::unsupported("no supported aspect ratio"))
            }
            _ => Err(ProviderError::unsupported("the model table has no aspect ratios")),
        }
    }

    /// Reject what the table cannot express: references on a plain generation, and the role a control route needs.
    fn check_inputs(&self, req: &ImageRequest) -> Result<(), ProviderError> {
        check_model(&self.desc, req)?;
        let id = req.model.as_str();
        let controls = matches!(id, "control-structure" | "control-style");
        if req.edit.is_none() && !req.refs.is_empty() && !controls {
            return Err(ProviderError::unsupported(format!(
                "{id} does not take reference images. Use control-style for a style anchor or control-structure for a geometry anchor"
            )));
        }
        if id == "control-style" {
            if req.edit.is_some() {
                return Err(ProviderError::unsupported("control-style generates from a style anchor and cannot edit an image"));
            }
            match req.refs.as_slice() {
                [r] if r.role == RefRole::StyleAnchor => {}
                _ => return Err(ProviderError::unsupported("control-style needs exactly one style anchor reference")),
            }
        }
        if id == "control-structure" && req.edit.is_none() {
            match req.refs.as_slice() {
                [r] if r.role == RefRole::GeometryAnchor => {}
                _ => {
                    return Err(ProviderError::unsupported("control-structure needs a base image to edit, or exactly one geometry anchor reference"))
                }
            }
        }
        Ok(())
    }

    /// The route and form for a request. Public so contract tests can snapshot the wire format.
    pub(crate) fn build_form(&self, req: &ImageRequest) -> Result<(String, Multipart), ProviderError> {
        self.check_inputs(req)?;
        let ratio = self.ratio(req)?;
        let mut f = Multipart::new();
        f.text("prompt", req.prompt.clone());
        f.text("output_format", "png");
        if !req.negative.is_empty() {
            f.text("negative_prompt", req.negative.join(", "));
        }
        if let Some(seed) = req.seed {
            f.text("seed", (seed % (MAX_SEED + 1)).to_string());
        }
        let input = req.edit.as_ref().map(|e| e.base_png.as_slice()).or_else(|| req.refs.first().map(|r| r.png.as_slice()));
        let path = match req.model.as_str() {
            "stable-image-core" => {
                f.text("aspect_ratio", ratio);
                "/v2beta/stable-image/generate/core"
            }
            "stable-image-ultra" => {
                match input {
                    Some(png) => {
                        f.file("image", "image.png", "image/png", png);
                        f.text("strength", EDIT_STRENGTH);
                    }
                    None => {
                        f.text("aspect_ratio", ratio);
                    }
                }
                "/v2beta/stable-image/generate/ultra"
            }
            "control-structure" => {
                f.file("image", "image.png", "image/png", input.unwrap_or_default());
                "/v2beta/stable-image/control/structure"
            }
            "control-style" => {
                f.file("image", "image.png", "image/png", input.unwrap_or_default());
                f.text("aspect_ratio", ratio);
                "/v2beta/stable-image/control/style"
            }
            sd3 => {
                f.text("model", sd3);
                match input {
                    Some(png) => {
                        f.text("mode", "image-to-image");
                        f.file("image", "image.png", "image/png", png);
                        f.text("strength", EDIT_STRENGTH);
                    }
                    None => {
                        f.text("mode", "text-to-image");
                        f.text("aspect_ratio", ratio);
                    }
                }
                "/v2beta/stable-image/generate/sd3"
            }
        };
        Ok((path.to_string(), f))
    }

    fn map_error(&self, status: u16, body: &[u8]) -> ProviderError {
        let secrets = self.secrets();
        let name = serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_string));
        match (status, name.as_deref()) {
            (403, Some("content_moderation")) | (422, Some("public_figure")) => {
                let mut e = status_error(400, body, &secrets);
                e.kind = ErrorKind::ModerationBlocked;
                e.retry = RetryClass::Terminal;
                e.message = e.message.replacen("HTTP 400", &format!("HTTP {status}"), 1);
                e.status = Some(status);
                e
            }
            (402, _) => {
                let mut e = status_error(429, body, &secrets);
                e.retry = RetryClass::RequiresAuthorization;
                e.message = e.message.replacen("HTTP 429", "HTTP 402 (out of credits)", 1);
                e.status = Some(402);
                e
            }
            _ => status_error(status, body, &secrets),
        }
    }

    fn call(&self, req: &ImageRequest) -> Result<ProviderOutput, ProviderError> {
        let key = self.key.as_ref().ok_or_else(|| ProviderError::not_configured("STABILITY_API_KEY is not set"))?;
        let (path, form) = self.build_form(req)?;
        let http = form
            .into_request(format!("{}{path}", self.base))
            .header("Authorization", &format!("Bearer {}", key.expose()))
            .header("Accept", "image/*");
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
        let content_type = resp.header("content-type").unwrap_or("").to_ascii_lowercase();
        let (image, finish, seed) = if content_type.starts_with("application/json") {
            // documented JSON shape, returned when Accept is application/json
            let v: Value = serde_json::from_slice(&resp.body).map_err(|e| malformed(&format!("the response is not valid JSON ({e})")))?;
            let img = v.get("image").and_then(Value::as_str).ok_or_else(|| malformed("the response has no image field"))?;
            (
                b64_decode(img)?,
                v.get("finish_reason").and_then(Value::as_str).map(str::to_string),
                v.get("seed").and_then(Value::as_u64).map(|s| s.to_string()),
            )
        } else {
            (resp.body.clone(), resp.header("finish-reason").map(str::to_string), resp.header("seed").map(str::to_string))
        };
        if finish.as_deref() == Some("CONTENT_FILTERED") {
            return Err(ProviderError::new(
                ErrorKind::ModerationBlocked,
                RetryClass::Terminal,
                "Stability flagged the result (finish-reason CONTENT_FILTERED) and returned a blurred image, which is not used",
            )
            .with_status(200));
        }
        let mime = sniff_mime(&image);
        if mime == "application/octet-stream" {
            return Err(malformed("the response body is not a PNG, JPEG or WebP image"));
        }
        let model = self.desc.model(&req.model);
        let actual_usd = model.and_then(|m| per_image_price(&m.cost, None)).map(|p| p.0);
        let mut notes = Vec::new();
        if let Some(s) = seed {
            notes.push(format!("seed: {}", redact(&s, &secrets)));
        }
        Ok(ProviderOutput {
            image,
            mime: mime.to_string(),
            provider_request_id: resp.header("x-request-id").map(str::to_string),
            usage: None,
            usage_unknown: actual_usd.is_none(),
            actual_usd,
            model_echo: Some(req.model.clone()),
            notes,
            job: None,
        })
    }
}

impl ImageProvider for StabilityProvider {
    fn id(&self) -> &str {
        "stability"
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
            "control-structure".to_string()
        } else {
            self.desc.recommended().map(|m| m.id.clone()).unwrap_or_else(|| "stable-image-core".into())
        }
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        self.check_inputs(req)?;
        Ok(tier_size(&self.ratio(req)?))
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        self.check_inputs(req)?;
        let model = check_model(&self.desc, req)?;
        let (usd, _) =
            per_image_price(&model.cost, None).ok_or_else(|| ProviderError::unsupported(format!("{} has no price in the table", model.id)))?;
        Ok(CostEstimate {
            usd,
            basis: format!(
                "Stability price table retrieved {}: {} costs {:.1} credits per successful call at $0.01 per credit (flat, failed calls are not charged)",
                self.desc.retrieved,
                model.id,
                usd / 0.01
            ),
            usage_unknown: false,
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
    if config.provider("stability").enabled == Some(false) {
        return;
    }
    let cfg = config.provider("stability");
    match policy_for(&["api.stability.ai"], cfg.base_url.as_deref(), env) {
        Ok(policy) => {
            let key = read_key("stability", &cfg, env);
            reg.insert(Arc::new(StabilityProvider::new(cfg, key, transport, policy)));
        }
        Err(why) => reg.problem("stability", why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FixtureTransport;
    use crate::types::{EditSpec, ProviderRef};
    use imagegen_core::config::MapEnv;
    use imagegen_core::pixels::{encode_png, Rgba};
    use serde_json::json;

    // DOCUMENTED SHAPE, NOT RECORDED: every body below follows the field tables and response descriptions on
    // docs.stability.ai (read 2026-11-02). Nothing was returned by Stability.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [10, 200, 30, 255])).unwrap()
    }

    fn provider(fixture: Arc<FixtureTransport>, key: Option<&str>) -> StabilityProvider {
        StabilityProvider::new(ProviderConfig::default(), key.map(str::to_string), fixture, HostPolicy::api(&["api.stability.ai"]))
    }

    fn request(model: &str) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "an iron wall".into(),
            negative: vec!["text".into(), "watermark".into()],
            width: 397,
            height: 153,
            transparent: false,
            quality: None,
            refs: vec![],
            edit: None,
            idempotency_key: None,
            seed: Some(7),
        }
    }

    fn anchor(role: RefRole) -> ProviderRef {
        ProviderRef { role, png: png(), sha256: "x".into() }
    }

    fn respond_image(fixture: &FixtureTransport, headers: &[(&str, &str)]) {
        let mut all = vec![("content-type", "image/png"), ("finish-reason", "SUCCESS")];
        all.extend_from_slice(headers);
        fixture.respond(Method::Post, "/v2beta/", 200, &all, png());
    }

    #[test]
    fn documented_shape_core_form_uses_the_nearest_ratio_and_the_documented_fields() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("sk-test-000000"));
        let (path, form) = p.build_form(&request("stable-image-core")).unwrap();
        assert_eq!(path, "/v2beta/stable-image/generate/core");
        assert_eq!(form.get("prompt").as_deref(), Some("an iron wall"));
        assert_eq!(form.get("aspect_ratio").as_deref(), Some("21:9"));
        assert_eq!(form.get("output_format").as_deref(), Some("png"));
        assert_eq!(form.get("negative_prompt").as_deref(), Some("text, watermark"));
        assert_eq!(form.get("seed").as_deref(), Some("7"));
        assert!(form.file_names().is_empty());
        let (w, h) = p.prepare(&request("stable-image-core")).unwrap();
        assert!(w > h && (f64::from(w) * f64::from(h) / 1_048_576.0 - 1.0).abs() < 0.05, "{w}x{h}");
    }

    #[test]
    fn documented_shape_sd3_text_and_image_modes() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("sk-test-000000"));
        let (path, form) = p.build_form(&request("sd3.5-large-turbo")).unwrap();
        assert_eq!(path, "/v2beta/stable-image/generate/sd3");
        assert_eq!(form.get("model").as_deref(), Some("sd3.5-large-turbo"));
        assert_eq!(form.get("mode").as_deref(), Some("text-to-image"));
        assert!(form.get("aspect_ratio").is_some());
        let mut r = request("sd3.5-large");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let (_, form) = p.build_form(&r).unwrap();
        assert_eq!(form.get("mode").as_deref(), Some("image-to-image"));
        assert_eq!(form.get("strength").as_deref(), Some(EDIT_STRENGTH));
        assert!(form.get("aspect_ratio").is_none(), "aspect_ratio does not apply to image-to-image");
        assert_eq!(form.file_names(), vec!["image"]);
    }

    #[test]
    fn documented_shape_ultra_edit_requires_strength_with_the_image() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("sk-test-000000"));
        let mut r = request("stable-image-ultra");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let (path, form) = p.build_form(&r).unwrap();
        assert_eq!(path, "/v2beta/stable-image/generate/ultra");
        assert!(form.get("strength").is_some() && form.file_names() == vec!["image"]);
    }

    #[test]
    fn documented_shape_control_routes_take_one_anchor() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("sk-test-000000"));
        let mut r = request("control-style");
        r.refs = vec![anchor(RefRole::StyleAnchor)];
        let (path, form) = p.build_form(&r).unwrap();
        assert_eq!(path, "/v2beta/stable-image/control/style");
        assert!(form.get("aspect_ratio").is_some() && form.file_names() == vec!["image"]);
        let mut r = request("control-structure");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let (path, form) = p.build_form(&r).unwrap();
        assert_eq!(path, "/v2beta/stable-image/control/structure");
        assert!(form.get("aspect_ratio").is_none() && form.get("strength").is_none());
        let mut r = request("control-structure");
        r.refs = vec![anchor(RefRole::GeometryAnchor)];
        assert!(p.build_form(&r).is_ok());
    }

    #[test]
    fn unsupported_combinations_fail_before_any_request() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider(fixture.clone(), Some("sk-test-000000"));
        let ctx = CallContext::new();
        // core cannot edit and takes no references
        let mut r = request("stable-image-core");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        assert_eq!(p.edit(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        let mut r = request("stable-image-core");
        r.refs = vec![anchor(RefRole::StyleAnchor)];
        assert_eq!(p.generate(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        // a reference on ultra would be a different thing (image-to-image), so it is refused
        let mut r = request("stable-image-ultra");
        r.refs = vec![anchor(RefRole::StyleAnchor)];
        assert!(p.generate(&r, &ctx).unwrap_err().message.contains("control-style"));
        // masks, wrong roles, style edits, unknown models
        let mut r = request("sd3.5-large");
        r.edit = Some(EditSpec { base_png: png(), mask_png: Some(png()) });
        assert!(p.edit(&r, &ctx).unwrap_err().message.contains("mask"));
        let mut r = request("control-style");
        r.refs = vec![anchor(RefRole::GeometryAnchor)];
        assert_eq!(p.generate(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        let mut r = request("control-style");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        assert_eq!(p.edit(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        assert_eq!(p.generate(&request("stable-image-core-v1:1"), &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn documented_shape_success_sends_bearer_and_accept_and_reads_headers() {
        let fixture = Arc::new(FixtureTransport::new());
        respond_image(&fixture, &[("seed", "1234"), ("x-request-id", "req-doc-1")]);
        let p = provider(fixture.clone(), Some("sk-test-000000"));
        let out = p.generate(&request("stable-image-ultra"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!(out.provider_request_id.as_deref(), Some("req-doc-1"));
        assert_eq!(out.actual_usd, Some(0.08));
        assert!(!out.usage_unknown && out.notes == vec!["seed: 1234".to_string()]);
        let sent = &fixture.requests()[0];
        assert_eq!(sent.url, "https://api.stability.ai/v2beta/stable-image/generate/ultra");
        assert!(sent.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer sk-test-000000"));
        assert!(sent.headers.iter().any(|(k, v)| k == "Accept" && v == "image/*"));
        let ct = sent.headers.iter().find(|(k, _)| k == "Content-Type").unwrap().1.clone();
        let boundary = ct.strip_prefix("multipart/form-data; boundary=").unwrap();
        let text = String::from_utf8_lossy(&sent.body);
        assert!(text.contains(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\nan iron wall\r\n")));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }

    #[test]
    fn documented_shape_json_response_is_decoded_too() {
        let fixture = Arc::new(FixtureTransport::new());
        let body = json!({"image": crate::common::b64(&png()), "seed": 5, "finish_reason": "SUCCESS"});
        fixture.respond(Method::Post, "/v2beta/", 200, &[("content-type", "application/json; type=image/png")], serde_json::to_vec(&body).unwrap());
        let p = provider(fixture, Some("sk-test-000000"));
        let out = p.generate(&request("stable-image-core"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.notes, vec!["seed: 5".to_string()]);
    }

    #[test]
    fn documented_shape_content_filtered_result_is_refused() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/v2beta/", 200, &[("content-type", "image/png"), ("finish-reason", "CONTENT_FILTERED")], png());
        let p = provider(fixture, Some("sk-test-000000"));
        let err = p.generate(&request("stable-image-core"), &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.retry), (ErrorKind::ModerationBlocked, RetryClass::Terminal));
    }

    #[test]
    fn documented_shape_errors_map_and_never_leak_the_key() {
        let key = "sk-test-secret-123456";
        let cases: [(u16, String, ErrorKind, RetryClass); 8] = [
            (401, format!(r#"{{"name":"unauthorized","errors":["bad key {key}"]}}"#), ErrorKind::Auth, RetryClass::RequiresAuthorization),
            (402, r#"{"name":"insufficient_balance","errors":["out of credits"]}"#.into(), ErrorKind::RateLimited, RetryClass::RequiresAuthorization),
            (403, r#"{"name":"content_moderation","errors":["flagged"]}"#.into(), ErrorKind::ModerationBlocked, RetryClass::Terminal),
            (422, r#"{"name":"public_figure","errors":["likeness"]}"#.into(), ErrorKind::ModerationBlocked, RetryClass::Terminal),
            (422, r#"{"name":"invalid_language","errors":["English only"]}"#.into(), ErrorKind::BadRequest, RetryClass::Terminal),
            (413, r#"{"name":"payload_too_large","errors":["10 MiB"]}"#.into(), ErrorKind::BadRequest, RetryClass::Terminal),
            (429, r#"{"name":"rate_limit_exceeded","errors":["slow down"]}"#.into(), ErrorKind::RateLimited, RetryClass::SafeSameInputs),
            (500, r#"{"name":"internal_error","errors":["boom"]}"#.into(), ErrorKind::ServerError, RetryClass::ReconcileFirst),
        ];
        for (status, body, kind, retry) in cases {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/v2beta/", status, &[], body);
            let p = provider(fixture, Some(key));
            let err = p.generate(&request("stable-image-core"), &CallContext::new()).unwrap_err();
            assert_eq!((err.kind, err.retry), (kind, retry), "status {status}");
            assert!(!err.message.contains(key), "key leaked: {}", err.message);
            assert_eq!(err.status, Some(status));
        }
    }

    #[test]
    fn documented_shape_malformed_success_demands_reconciliation() {
        for (ct, body) in [("image/png", b"not an image".to_vec()), ("application/json", br#"{"finish_reason":"SUCCESS"}"#.to_vec())] {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/v2beta/", 200, &[("content-type", ct)], body);
            let p = provider(fixture, Some("sk-test-000000"));
            let err = p.generate(&request("stable-image-core"), &CallContext::new()).unwrap_err();
            assert_eq!((err.kind, err.retry), (ErrorKind::MalformedResponse, RetryClass::ReconcileFirst));
        }
    }

    #[test]
    fn missing_key_fails_before_any_request_and_a_foreign_base_is_blocked() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider(fixture.clone(), None);
        assert!(!p.is_configured());
        assert_eq!(p.generate(&request("stable-image-core"), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);
        let cfg = ProviderConfig { base_url: Some("https://evil.example".into()), ..Default::default() };
        let p = StabilityProvider::new(cfg, Some("sk-test-000000".into()), fixture.clone(), HostPolicy::api(&["api.stability.ai"]));
        assert_eq!(p.generate(&request("stable-image-core"), &CallContext::new()).unwrap_err().kind, ErrorKind::Blocked);
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn estimates_come_from_the_credit_table() {
        let p = provider(Arc::new(FixtureTransport::new()), None);
        for (model, usd) in
            [("stable-image-core", 0.03), ("stable-image-ultra", 0.08), ("sd3.5-large", 0.065), ("sd3.5-large-turbo", 0.04), ("sd3.5-medium", 0.035)]
        {
            let e = p.estimate(&request(model)).unwrap();
            assert!((e.usd - usd).abs() < 1e-12, "{model}: {}", e.usd);
            assert!(!e.usage_unknown && e.funding == Funding::UserKey);
            assert!(e.basis.contains("2026-11-02") && e.basis.contains("credits"), "{}", e.basis);
        }
        let mut r = request("control-style");
        r.refs = vec![anchor(RefRole::StyleAnchor)];
        assert!((p.estimate(&r).unwrap().usd - 0.05).abs() < 1e-12);
    }

    #[test]
    fn table_is_marked_not_live_verified_and_has_one_recommended_model() {
        let raw: Value = serde_json::from_str(include_str!("../data/stability.json")).unwrap();
        assert_eq!(raw["live_verified"], json!(false));
        assert!(raw["doc_urls"].as_array().map(|a| !a.is_empty()).unwrap_or(false) && raw["checked"] == json!("2026-11-02"));
        let d = descriptor_from(include_str!("../data/stability.json"));
        assert_eq!(d.models.iter().filter(|m| m.recommended).count(), 1);
        assert!(d.terms_note.contains("Not live-verified") && d.models.iter().all(|m| m.notes.contains("Not live-verified")));
        let p = provider(Arc::new(FixtureTransport::new()), None);
        assert_eq!(p.default_model(false), "stable-image-core");
        assert_eq!(p.default_model(true), "control-structure");
    }

    #[test]
    fn registry_builds_it_with_or_without_a_key_and_honours_disable() {
        let fixture: Arc<dyn HttpTransport> = Arc::new(FixtureTransport::new());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::default(), fixture.clone());
        assert!(reg.get("stability").is_some() && !reg.get("stability").unwrap().is_configured());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::from_pairs(&[("STABILITY_API_KEY", "sk-test-1")]), fixture.clone());
        assert!(reg.get("stability").unwrap().is_configured());
        let cfg = Config::parse("[providers.stability]\nenabled = false\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &cfg, &MapEnv::default(), fixture.clone());
        assert!(reg.get("stability").is_none());
        let cfg = Config::parse("[providers.stability]\nbase_url = \"https://evil.example\"\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &cfg, &MapEnv::default(), fixture);
        assert!(reg.get("stability").is_none() && reg.problems().contains_key("stability"));
    }
}
