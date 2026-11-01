//! Google Gemini API adapter (image models).
//!
//! The Gemini docs (retrieved 2026-10-31) document `POST /v1beta/interactions` with `model`, an `input` array of
//! typed blocks (`text`, and `image` blocks carrying `mime_type` and base64 `data`) and a `response_format` of
//! `{type: "image", mime_type, aspect_ratio, image_size}`. The page shows no raw image response, so the parser
//! reads the documented content schema: image blocks inside `steps[].content[]`. Real recordings are still
//! pending (no key with balance); the fixtures in the tests are synthetic.
//!
//! Imagen is shut down and is not offered. Gemini has no native transparency: alpha must be proven from pixels
//! or produced by a matte workflow.

use std::sync::Arc;

use imagegen_core::config::ProviderConfig;
use imagegen_core::error::RetryClass;
use serde_json::{json, Value};

use crate::common::{b64, b64_decode, basis, check_model, descriptor_from, per_image_price, status_error, ApiKey};
use crate::sizing::nearest_ratio;
use crate::transport::{send, HostPolicy, HttpRequest, HttpTransport};
use crate::types::{
    CallContext, CostEstimate, ErrorKind, Funding, ImageProvider, ImageRequest, ProviderDescriptor, ProviderError, ProviderOutput, SizeRule,
};

/// Default API base.
pub const DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com";

/// Adapter state.
pub struct GeminiProvider {
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    base: String,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
}

impl GeminiProvider {
    /// Build the adapter.
    pub fn new(cfg: ProviderConfig, key: Option<String>, transport: Arc<dyn HttpTransport>, policy: HostPolicy) -> Self {
        let base = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_BASE.to_string()).trim_end_matches('/').to_string();
        GeminiProvider { desc: descriptor_from(include_str!("../data/gemini.json")), cfg, key: key.map(ApiKey::new), base, transport, policy }
    }

    fn secrets(&self) -> Vec<String> {
        self.key.iter().map(|k| k.expose().to_string()).collect()
    }

    /// Aspect ratio and tier for a request.
    fn format_for(&self, req: &ImageRequest) -> Result<(String, String), ProviderError> {
        let model = check_model(&self.desc, req)?;
        match &model.sizes {
            SizeRule::AspectTiers { ratios, tier } => {
                let ratio = nearest_ratio(ratios, [req.width, req.height]).ok_or_else(|| ProviderError::unsupported("no aspect ratio available"))?;
                Ok((ratio, tier.clone()))
            }
            _ => Err(ProviderError::unsupported("the Gemini table must use aspect ratios and tiers")),
        }
    }

    /// Request body, public for wire-format tests.
    pub fn build_body(&self, req: &ImageRequest) -> Result<Value, ProviderError> {
        let (ratio, tier) = self.format_for(req)?;
        let mut input = vec![json!({"type": "text", "text": req.prompt})];
        let image_block = |png: &[u8]| json!({"type": "image", "mime_type": "image/png", "data": b64(png)});
        if let Some(edit) = &req.edit {
            input.push(image_block(&edit.base_png));
        }
        for r in &req.refs {
            input.push(image_block(&r.png));
        }
        Ok(json!({
            "model": req.model,
            "input": input,
            "response_format": {"type": "image", "mime_type": "image/png", "aspect_ratio": ratio, "image_size": tier},
            "store": false,
        }))
    }

    fn call(&self, req: &ImageRequest) -> Result<ProviderOutput, ProviderError> {
        let key = self.key.as_ref().ok_or_else(|| ProviderError::not_configured("GEMINI_API_KEY (or GOOGLE_API_KEY) is not set"))?;
        let body = self.build_body(req)?;
        let http = HttpRequest::post_json(format!("{}/v1beta/interactions", self.base), &body).header("x-goog-api-key", key.expose());
        let resp = send(&*self.transport, &self.policy, &http)?;
        let secrets = self.secrets();
        if resp.status != 200 {
            return Err(status_error(resp.status, &resp.body, &secrets));
        }
        let malformed = |why: &str| {
            ProviderError::new(
                ErrorKind::MalformedResponse,
                RetryClass::ReconcileFirst,
                format!("{why}. The request was accepted, so a charge may have been made"),
            )
            .with_status(200)
        };
        let v: Value = serde_json::from_slice(&resp.body).map_err(|e| malformed(&format!("the response is not valid JSON ({e})")))?;
        if let Some(status) = v.get("status").and_then(Value::as_str) {
            if status != "completed" {
                let detail = v.get("errors").map(|e| e.to_string()).unwrap_or_default();
                let blocked = detail.to_ascii_lowercase().contains("safety") || detail.to_ascii_lowercase().contains("blocked");
                return Err(ProviderError::new(
                    if blocked { ErrorKind::ModerationBlocked } else { ErrorKind::ServerError },
                    if blocked { RetryClass::Terminal } else { RetryClass::ReconcileFirst },
                    format!("the interaction ended with status '{status}' {}", crate::transport::redact(&detail, &secrets)),
                ));
            }
        }
        let (data, mime) = find_image_block(&v).ok_or_else(|| malformed("the response has no image block in steps[].content[]"))?;
        let image = b64_decode(&data)?;
        let (usd, _) = per_image_price(&self.desc.model(&req.model).map(|m| m.cost.clone()).unwrap_or(crate::types::CostModel::Free), None).unzip();
        Ok(ProviderOutput {
            image,
            mime: mime.unwrap_or_else(|| "image/png".into()),
            provider_request_id: v.get("id").and_then(Value::as_str).map(str::to_string),
            usage: v.get("usage").cloned(),
            actual_usd: usd,
            usage_unknown: usd.is_none(),
            model_echo: v.get("model").and_then(Value::as_str).map(str::to_string),
            notes: vec!["Gemini outputs carry a SynthID watermark".into()],
            job: None,
        })
    }
}

/// First image block in `steps[].content[]`, or in a top-level `outputs[]` as a fallback.
fn find_image_block(v: &Value) -> Option<(String, Option<String>)> {
    let pick = |block: &Value| -> Option<(String, Option<String>)> {
        if block.get("type").and_then(Value::as_str) == Some("image") {
            let data = block.get("data").and_then(Value::as_str)?;
            return Some((data.to_string(), block.get("mime_type").and_then(Value::as_str).map(str::to_string)));
        }
        None
    };
    for step in v.get("steps").and_then(Value::as_array).into_iter().flatten() {
        for block in step.get("content").and_then(Value::as_array).into_iter().flatten() {
            if let Some(found) = pick(block) {
                return Some(found);
            }
        }
    }
    v.get("outputs").and_then(Value::as_array).into_iter().flatten().find_map(pick)
}

impl ImageProvider for GeminiProvider {
    fn id(&self) -> &str {
        "gemini"
    }

    fn descriptor(&self) -> &ProviderDescriptor {
        &self.desc
    }

    fn is_configured(&self) -> bool {
        self.key.is_some()
    }

    fn default_model(&self, _editing: bool) -> String {
        self.cfg.model.clone().or_else(|| self.desc.recommended().map(|m| m.id.clone())).unwrap_or_else(|| "gemini-nano-banana-2.1".into())
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        // Gemini picks pixels from the aspect ratio and tier; report the nominal 1K size for the ratio.
        let (ratio, _) = self.format_for(req)?;
        Ok(crate::sizing::tier_size(&ratio))
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        let model = check_model(&self.desc, req)?;
        let (usd, _) = per_image_price(&model.cost, None).ok_or_else(|| ProviderError::unsupported("no price in the table"))?;
        let proxy = matches!(model.cost, crate::types::CostModel::UnpublishedProxy { .. });
        Ok(CostEstimate {
            usd,
            basis: basis("Gemini", &format!("{} at 1K{}", model.id, if proxy { " (proxy price: none retrieved for this model)" } else { "" })),
            usage_unknown: proxy,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{FixtureTransport, Method};
    use crate::types::{EditSpec, ProviderRef};
    use imagegen_core::pixels::{encode_png, Rgba};
    use imagegen_core::refs::RefRole;

    // SYNTHETIC fixtures: the response envelope is inferred from the documented content schema, not recorded.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [200, 10, 30, 255])).unwrap()
    }

    fn provider(fixture: Arc<FixtureTransport>, key: Option<&str>) -> GeminiProvider {
        GeminiProvider::new(ProviderConfig::default(), key.map(str::to_string), fixture, HostPolicy::api(&["generativelanguage.googleapis.com"]))
    }

    fn request(model: &str) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "an iron wall".into(),
            negative: vec![],
            width: 397,
            height: 153,
            transparent: false,
            quality: None,
            refs: vec![],
            edit: None,
            idempotency_key: None,
            seed: None,
        }
    }

    #[test]
    fn body_uses_the_interactions_format_with_uppercase_k_tier() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("g-test-000000"));
        let mut r = request("gemini-nano-banana-2.1");
        r.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }];
        let body = p.build_body(&r).unwrap();
        assert_eq!(body["model"], "gemini-nano-banana-2.1");
        assert_eq!(body["input"][0]["type"], "text");
        assert_eq!(body["input"][1]["type"], "image");
        assert_eq!(body["input"][1]["mime_type"], "image/png");
        assert_eq!(body["response_format"]["type"], "image");
        assert_eq!(body["response_format"]["aspect_ratio"], "21:9");
        assert_eq!(body["response_format"]["image_size"], "1K");
        assert_eq!(body["store"], false, "do not ask the service to retain the interaction");
    }

    #[test]
    fn edit_puts_the_base_image_before_the_references() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("g-test-000000"));
        let mut r = request("gemini-nano-banana-2.1");
        r.edit = Some(EditSpec { base_png: vec![1, 1, 1], mask_png: None });
        r.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: vec![2, 2, 2], sha256: "x".into() }];
        let body = p.build_body(&r).unwrap();
        let first = body["input"][1]["data"].as_str().unwrap();
        assert_eq!(b64_decode(first).unwrap(), vec![1, 1, 1]);
        assert_eq!(b64_decode(body["input"][2]["data"].as_str().unwrap()).unwrap(), vec![2, 2, 2]);
    }

    #[test]
    fn successful_response_is_parsed_from_steps() {
        let fixture = Arc::new(FixtureTransport::new());
        let body = json!({"id": "v1_synthetic", "object": "interaction", "status": "completed", "model": "gemini-nano-banana-2.1",
            "steps": [{"type": "model_output", "content": [{"type": "text", "text": "here"}, {"type": "image", "mime_type": "image/png", "data": b64(&png())}]}],
            "usage": {"total_tokens": 1300}});
        fixture.respond(Method::Post, "/v1beta/interactions", 200, &[], serde_json::to_vec(&body).unwrap());
        let p = provider(fixture.clone(), Some("g-test-000000"));
        let out = p.generate(&request("gemini-nano-banana-2.1"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.provider_request_id.as_deref(), Some("v1_synthetic"));
        assert_eq!(out.actual_usd, Some(0.0336));
        let sent = &fixture.requests()[0];
        assert!(sent.headers.iter().any(|(k, v)| k == "x-goog-api-key" && v == "g-test-000000"));
        assert!(!sent.url.contains("g-test-000000"), "the key travels in a header, never in the URL");
    }

    #[test]
    fn failed_or_image_less_interactions_are_errors() {
        for (body, kind) in [
            (json!({"status": "failed", "errors": [{"code": "x", "message": "blocked by safety"}], "steps": []}), ErrorKind::ModerationBlocked),
            (json!({"status": "failed", "errors": [{"message": "internal"}], "steps": []}), ErrorKind::ServerError),
            (
                json!({"status": "completed", "steps": [{"type": "model_output", "content": [{"type": "text", "text": "no image"}]}]}),
                ErrorKind::MalformedResponse,
            ),
        ] {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/interactions", 200, &[], serde_json::to_vec(&body).unwrap());
            let p = provider(fixture, Some("g-test-000000"));
            let err = p.generate(&request("gemini-nano-banana-2.1"), &CallContext::new()).unwrap_err();
            assert_eq!(err.kind, kind, "{err}");
        }
    }

    #[test]
    fn http_errors_never_leak_the_key() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/interactions", 403, &[], r#"{"error":{"message":"API key g-test-secret-999 not valid"}}"#);
        let p = provider(fixture, Some("g-test-secret-999"));
        let err = p.generate(&request("gemini-nano-banana-2.1"), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Auth);
        assert!(!err.message.contains("g-test-secret-999"));
    }

    #[test]
    fn deprecated_model_estimates_are_flagged_as_proxies_and_masks_are_refused() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("g-test-000000"));
        let e = p.estimate(&request("gemini-2.5-flash-image")).unwrap();
        assert!(e.usage_unknown && e.basis.contains("proxy"));
        let mut r = request("gemini-nano-banana-2.1");
        r.edit = Some(EditSpec { base_png: png(), mask_png: Some(png()) });
        assert!(p.edit(&r, &CallContext::new()).unwrap_err().message.contains("mask"));
        assert_eq!(p.default_model(true), "gemini-nano-banana-2.1");
    }

    #[test]
    fn prepare_reports_a_one_megapixel_size_for_the_ratio() {
        let p = provider(Arc::new(FixtureTransport::new()), None);
        let (w, h) = p.prepare(&request("gemini-nano-banana-2.1")).unwrap();
        let px = f64::from(w) * f64::from(h);
        assert!((px / 1_048_576.0 - 1.0).abs() < 0.05, "{w}x{h}");
        assert!(w > h);
    }
}
