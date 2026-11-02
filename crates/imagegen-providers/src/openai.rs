//! OpenAI Images API adapter (documented, API-key route).
//!
//! `POST /v1/images/generations` for generation and `POST /v1/images/edits` (JSON body with `images` and an
//! optional `mask`) for edits and reference-guided work. GPT Image models return base64 in `data[].b64_json`.
//! This is the documented API. The undocumented ChatGPT-login backend lives in a separate opt-in crate.

use std::sync::Arc;

use imagegen_core::config::ProviderConfig;
use imagegen_core::error::RetryClass;
use serde_json::{json, Value};

use crate::common::{b64_decode, basis, check_model, data_url, descriptor_from, megapixels, per_image_price, status_error, ApiKey};
use crate::sizing::choose_size;
use crate::transport::{redact, send, HostPolicy, HttpRequest, HttpTransport};
use crate::types::{
    AlphaSupport, CallContext, CostEstimate, CostModel, ErrorKind, Funding, ImageProvider, ImageRequest, ProviderDescriptor, ProviderError,
    ProviderOutput,
};

/// Default API base.
pub const DEFAULT_BASE: &str = "https://api.openai.com";

/// Adapter state.
pub struct OpenAiProvider {
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    base: String,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
    safety_identifier: Option<String>,
}

impl OpenAiProvider {
    /// Build the adapter. `policy` decides which hosts it may reach (the registry widens it only for a base URL
    /// the user explicitly allowed).
    pub fn new(
        cfg: ProviderConfig,
        key: Option<String>,
        transport: Arc<dyn HttpTransport>,
        policy: HostPolicy,
        safety_identifier: Option<String>,
    ) -> Self {
        let base = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_BASE.to_string()).trim_end_matches('/').to_string();
        OpenAiProvider {
            desc: descriptor_from(include_str!("../data/openai.json")),
            cfg,
            key: key.map(ApiKey::new),
            base,
            transport,
            policy,
            safety_identifier,
        }
    }

    fn secrets(&self) -> Vec<String> {
        self.key.iter().map(|k| k.expose().to_string()).collect()
    }

    fn quality(&self, req: &ImageRequest) -> String {
        req.quality.clone().or_else(|| self.cfg.quality.clone()).unwrap_or_else(|| "low".to_string())
    }

    fn size_string(&self, req: &ImageRequest) -> Result<String, ProviderError> {
        let (w, h) = self.prepare(req)?;
        Ok(format!("{w}x{h}"))
    }

    /// The JSON body for a request. Public so contract tests can snapshot the wire format.
    pub fn build_body(&self, req: &ImageRequest) -> Result<(String, Value), ProviderError> {
        let model = check_model(&self.desc, req)?;
        let size = self.size_string(req)?;
        let mut body = json!({
            "model": req.model,
            "prompt": req.prompt,
            "n": 1,
            "size": size,
            "quality": self.quality(req),
            "output_format": "png",
        });
        if req.transparent && model.alpha != AlphaSupport::None {
            body["background"] = json!("transparent");
        }
        if let Some(id) = &self.safety_identifier {
            body["safety_identifier"] = json!(id);
        }
        match &req.edit {
            None if req.refs.is_empty() => Ok(("/v1/images/generations".into(), body)),
            _ => {
                let mut images = Vec::new();
                if let Some(edit) = &req.edit {
                    images.push(json!({"image_url": data_url("image/png", &edit.base_png)}));
                }
                for r in &req.refs {
                    images.push(json!({"image_url": data_url("image/png", &r.png)}));
                }
                body["images"] = Value::Array(images);
                if let Some(mask) = req.edit.as_ref().and_then(|e| e.mask_png.as_ref()) {
                    body["mask"] = json!({"image_url": data_url("image/png", mask)});
                }
                Ok(("/v1/images/edits".into(), body))
            }
        }
    }

    fn call(&self, req: &ImageRequest) -> Result<ProviderOutput, ProviderError> {
        let key = self.key.as_ref().ok_or_else(|| ProviderError::not_configured("OPENAI_API_KEY is not set"))?;
        let (path, body) = self.build_body(req)?;
        let http = HttpRequest::post_json(format!("{}{path}", self.base), &body).header("Authorization", &format!("Bearer {}", key.expose()));
        let resp = send(&*self.transport, &self.policy, &http)?;
        let secrets = self.secrets();
        if resp.status != 200 {
            return Err(status_error(resp.status, &resp.body, &secrets));
        }
        let malformed = |why: &str| {
            ProviderError::new(
                ErrorKind::MalformedResponse,
                RetryClass::ReconcileFirst,
                format!("{why}. The request was accepted, so a charge may have been made: check the account before repeating it"),
            )
            .with_status(200)
        };
        let v: Value = serde_json::from_slice(&resp.body).map_err(|e| malformed(&format!("the response is not valid JSON ({e})")))?;
        let b64 = v.pointer("/data/0/b64_json").and_then(Value::as_str).ok_or_else(|| malformed("the response has no data[0].b64_json"))?;
        let image = b64_decode(b64)?;
        let format = v.get("output_format").and_then(Value::as_str).unwrap_or("png");
        let usage = v.get("usage").cloned();
        let (actual_usd, usage_unknown) = self.actual_cost(req, usage.as_ref());
        let mut notes = Vec::new();
        if let Some(rp) = v.pointer("/data/0/revised_prompt").and_then(Value::as_str) {
            notes.push(format!("revised prompt: {}", redact(rp, &secrets)));
        }
        Ok(ProviderOutput {
            image,
            mime: format!("image/{}", if format == "jpg" { "jpeg" } else { format }),
            provider_request_id: resp.header("x-request-id").map(str::to_string),
            usage,
            actual_usd,
            usage_unknown,
            model_echo: Some(req.model.clone()),
            notes,
            job: None,
        })
    }

    /// Actual cost: the table price for per-image models; token arithmetic for models priced by tokens when
    /// the response carries the breakdown. Missing usage is `usage_unknown`, never zero.
    fn actual_cost(&self, req: &ImageRequest, usage: Option<&Value>) -> (Option<f64>, bool) {
        let Some(model) = self.desc.model(&req.model) else { return (None, true) };
        if let CostModel::PerImage { .. } = model.cost {
            return (per_image_price(&model.cost, Some(&self.quality(req))).map(|p| p.0), false);
        }
        let Some(u) = usage else { return (None, true) };
        let text_in = u.pointer("/input_tokens_details/text_tokens").and_then(Value::as_f64);
        let image_in = u.pointer("/input_tokens_details/image_tokens").and_then(Value::as_f64);
        let out = u.get("output_tokens").and_then(Value::as_f64);
        match (text_in, image_in, out) {
            // token rates on the vendor pricing page, 2026-10-31: text input $5, image input $8, image output $30 per 1M
            (Some(t), Some(i), Some(o)) => (Some((t * 5.0 + i * 8.0 + o * 30.0) / 1_000_000.0), false),
            _ => (None, true),
        }
    }
}

impl ImageProvider for OpenAiProvider {
    fn id(&self) -> &str {
        "openai"
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
            "gpt-image-2.5-sunburst".to_string()
        } else {
            self.desc.recommended().map(|m| m.id.clone()).unwrap_or_else(|| "gpt-image-2.5-flare".into())
        }
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        let model = check_model(&self.desc, req)?;
        choose_size(&model.sizes, [req.width, req.height])
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        let model = check_model(&self.desc, req)?;
        let quality = self.quality(req);
        let (w, h) = self.prepare(req)?;
        let (usd, q) = per_image_price(&model.cost, Some(&quality))
            .ok_or_else(|| ProviderError::unsupported(format!("{} has no price for quality '{quality}' in the table", model.id)))?;
        // table prices are for about 1 MP; larger outputs scale linearly
        let scale = megapixels(w, h).max(1.0);
        let proxy = matches!(model.cost, CostModel::UnpublishedProxy { .. });
        let basis_text = if let CostModel::UnpublishedProxy { proxy_of, .. } = &model.cost {
            basis(
                "OpenAI",
                &format!(
                    "{q} quality, using the {proxy_of} row as a proxy because {} has no published per-image price; tokens settle the real figure",
                    model.id
                ),
            )
        } else {
            basis("OpenAI", &format!("{} {q} quality at about 1 MP", model.id))
        };
        Ok(CostEstimate { usd: (usd * scale * 1e6).round() / 1e6, basis: basis_text, usage_unknown: proxy, funding: Funding::UserKey })
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
    use crate::common::b64;
    use crate::transport::{FixtureTransport, Method};
    use crate::types::{EditSpec, ProviderRef};
    use imagegen_core::pixels::{encode_png, Rgba};
    use imagegen_core::refs::RefRole;

    // SYNTHETIC fixtures: these bodies were written for the tests, not recorded from OpenAI.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [10, 200, 30, 255])).unwrap()
    }

    fn provider(fixture: Arc<FixtureTransport>, key: Option<&str>) -> OpenAiProvider {
        OpenAiProvider::new(ProviderConfig::default(), key.map(str::to_string), fixture, HostPolicy::api(&["api.openai.com"]), None)
    }

    fn request(model: &str) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "an iron wall".into(),
            negative: vec![],
            width: 100,
            height: 88,
            transparent: true,
            quality: Some("low".into()),
            refs: vec![],
            edit: None,
            idempotency_key: Some("k1".into()),
            seed: None,
        }
    }

    fn ok_body(extra: Value) -> Vec<u8> {
        let mut v = json!({"created": 1, "data": [{"b64_json": b64(&png())}], "output_format": "png"});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn generation_body_matches_the_documented_wire_format() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("sk-test-000000"));
        let (path, body) = p.build_body(&request("gpt-image-2.5-flare")).unwrap();
        assert_eq!(path, "/v1/images/generations");
        assert_eq!(body["model"], "gpt-image-2.5-flare");
        assert_eq!(body["n"], 1);
        assert_eq!(body["background"], "transparent");
        assert_eq!(body["output_format"], "png");
        assert_eq!(body["quality"], "low");
        let size = body["size"].as_str().unwrap();
        let (w, h) = size.split_once('x').unwrap();
        let (w, h): (u32, u32) = (w.parse().unwrap(), h.parse().unwrap());
        assert!(w % 16 == 0 && h % 16 == 0 && u64::from(w) * u64::from(h) >= 655_360, "{size}");
        assert!(body.get("images").is_none() && body.get("safety_identifier").is_none());
    }

    #[test]
    fn edit_body_puts_the_base_first_then_references_and_a_mask() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("sk-test-000000"));
        let mut r = request("gpt-image-2.5-sunburst");
        r.edit = Some(EditSpec { base_png: png(), mask_png: Some(png()) });
        r.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: vec![1, 2, 3], sha256: "x".into() }];
        let (path, body) = p.build_body(&r).unwrap();
        assert_eq!(path, "/v1/images/edits");
        let images = body["images"].as_array().unwrap();
        assert_eq!(images.len(), 2);
        assert!(images[0]["image_url"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert!(body["mask"]["image_url"].as_str().unwrap().starts_with("data:image/png;base64,"));
    }

    #[test]
    fn no_background_parameter_for_models_without_documented_alpha() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("sk-test-000000"));
        let (_, body) = p.build_body(&request("gpt-image-1.5")).unwrap();
        assert!(body.get("background").is_none(), "alpha is proven from pixels, not requested where undocumented");
        assert_eq!(body["size"], "1024x1024");
    }

    #[test]
    fn successful_generation_parses_image_usage_and_request_id() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(
            Method::Post,
            "/v1/images/generations",
            200,
            &[("x-request-id", "req_synthetic_1")],
            ok_body(json!({"usage": {"input_tokens": 100, "input_tokens_details": {"text_tokens": 100, "image_tokens": 0}, "output_tokens": 1056}})),
        );
        let p = provider(fixture.clone(), Some("sk-test-000000"));
        let out = p.generate(&request("gpt-image-2.5-flare"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!(out.provider_request_id.as_deref(), Some("req_synthetic_1"));
        assert!(!out.usage_unknown);
        let usd = out.actual_usd.unwrap();
        assert!((usd - (100.0 * 5.0 + 1056.0 * 30.0) / 1e6).abs() < 1e-12, "{usd}");
        let sent = &fixture.requests()[0];
        assert!(sent.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer sk-test-000000"));
        assert!(sent.url.starts_with("https://api.openai.com/"));
    }

    #[test]
    fn missing_usage_is_unknown_never_zero_and_per_image_models_use_the_table() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/generations", 200, &[], ok_body(json!({})));
        let p = provider(fixture, Some("sk-test-000000"));
        let out = p.generate(&request("gpt-image-2.5-flare"), &CallContext::new()).unwrap();
        assert!(out.usage_unknown && out.actual_usd.is_none());
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/generations", 200, &[], ok_body(json!({})));
        let p = provider(fixture, Some("sk-test-000000"));
        let out = p.generate(&request("gpt-image-2"), &CallContext::new()).unwrap();
        assert!(!out.usage_unknown);
        assert_eq!(out.actual_usd, Some(0.006));
    }

    #[test]
    fn errors_map_and_never_leak_the_key() {
        let key = "sk-test-secret-123456";
        let cases: [(u16, &str, ErrorKind, RetryClass); 5] = [
            (401, r#"{"error":{"message":"Incorrect API key provided: sk-test-secret-123456"}}"#, ErrorKind::Auth, RetryClass::RequiresAuthorization),
            (429, r#"{"error":{"message":"rate limit"}}"#, ErrorKind::RateLimited, RetryClass::SafeSameInputs),
            (400, r#"{"error":{"code":"moderation_blocked","message":"blocked"}}"#, ErrorKind::ModerationBlocked, RetryClass::Terminal),
            (400, r#"{"error":{"message":"bad size"}}"#, ErrorKind::BadRequest, RetryClass::Terminal),
            (500, "boom", ErrorKind::ServerError, RetryClass::ReconcileFirst),
        ];
        for (status, body, kind, retry) in cases {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/generations", status, &[], body);
            let p = provider(fixture, Some(key));
            let err = p.generate(&request("gpt-image-2.5-flare"), &CallContext::new()).unwrap_err();
            assert_eq!((err.kind, err.retry), (kind, retry), "status {status}");
            assert!(!err.message.contains(key), "key leaked: {}", err.message);
            assert_eq!(err.status, Some(status));
        }
    }

    #[test]
    fn malformed_and_truncated_success_responses_demand_reconciliation() {
        for body in [b"not json".to_vec(), br#"{"data":[]}"#.to_vec(), br#"{"data":[{"b64_json":"%%%"}]}"#.to_vec()] {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/generations", 200, &[], body);
            let p = provider(fixture, Some("sk-test-000000"));
            let err = p.generate(&request("gpt-image-2.5-flare"), &CallContext::new()).unwrap_err();
            assert_eq!(err.retry, RetryClass::ReconcileFirst, "{err}");
        }
    }

    #[test]
    fn missing_key_retired_models_and_wrong_call_kind_fail_before_any_request() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider(fixture.clone(), None);
        assert!(!p.is_configured());
        assert_eq!(p.generate(&request("gpt-image-2.5-flare"), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);
        let p = provider(fixture.clone(), Some("sk-test-000000"));
        assert_eq!(p.generate(&request("gpt-image-1"), &CallContext::new()).unwrap_err().kind, ErrorKind::ModelRetired);
        assert!(p.edit(&request("gpt-image-2.5-flare"), &CallContext::new()).is_err());
        assert!(fixture.requests().is_empty(), "nothing may be sent for a request that cannot succeed");
    }

    #[test]
    fn estimates_use_the_dated_table_and_flag_the_proxy() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("sk-test-000000"));
        let e = p.estimate(&request("gpt-image-2.5-flare")).unwrap();
        assert!(e.usage_unknown, "2.5 has no published per-image price");
        assert!(e.basis.contains("proxy") && e.basis.contains("2026-10-31"), "{}", e.basis);
        assert!((e.usd - 0.006).abs() < 1e-9);
        let mut r = request("gpt-image-2");
        r.quality = Some("high".into());
        let e = p.estimate(&r).unwrap();
        assert!(!e.usage_unknown);
        assert!((e.usd - 0.211).abs() < 1e-9);
        r.quality = Some("ultra".into());
        assert!(p.estimate(&r).is_err());
    }

    #[test]
    fn default_models_follow_the_research() {
        let p = provider(Arc::new(FixtureTransport::new()), None);
        assert_eq!(p.default_model(false), "gpt-image-2.5-flare");
        assert_eq!(p.default_model(true), "gpt-image-2.5-sunburst");
        let cfg = ProviderConfig { model: Some("gpt-image-2".into()), ..Default::default() };
        let p = OpenAiProvider::new(cfg, None, Arc::new(FixtureTransport::new()), HostPolicy::api(&["api.openai.com"]), None);
        assert_eq!(p.default_model(true), "gpt-image-2");
    }

    #[test]
    fn a_base_url_outside_the_policy_is_blocked_at_send_time() {
        let cfg = ProviderConfig { base_url: Some("https://evil.example".into()), ..Default::default() };
        let fixture = Arc::new(FixtureTransport::new());
        let p = OpenAiProvider::new(cfg, Some("sk-test-000000".into()), fixture.clone(), HostPolicy::api(&["api.openai.com"]), None);
        let err = p.generate(&request("gpt-image-2.5-flare"), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Blocked);
        assert!(fixture.requests().is_empty(), "the key must not be sent to a host outside the allowlist");
    }
}
