//! Recraft API (`recraft`): synchronous raster generation, image-to-image and style references.
//!
//! Built from the vendor documentation read on 2026-11-02 and **not live-verified**: no key was available and no
//! call was made. Base `https://external.api.recraft.ai/v1`, `Authorization: Bearer`. Generation is OpenAI-like
//! JSON (`POST /images/generations`) asking for `response_format: b64_json` and `image_format: png`, with a
//! `data[0].url` download as the documented fallback. Style anchors go as multipart `style_references`
//! (`recraftv4_styles` only). Edits use `POST /images/imageToImage`. Only raster models are offered because
//! this tool needs PNG: any `*_vector` id is refused. Transparent output is not documented for generation.

use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource, ProviderConfig};
use imagegen_core::error::RetryClass;
use imagegen_core::refs::RefRole;
use serde_json::{json, Value};

use crate::common::{b64_decode, check_model, descriptor_from, per_image_price, sniff_mime, status_error, ApiKey};
use crate::registry::{policy_for, read_key, ProviderRegistry};
use crate::sizing::choose_size;
use crate::stability::Multipart;
use crate::transport::{redact, send, HostPolicy, HttpRequest, HttpTransport};
use crate::types::{CallContext, CostEstimate, ErrorKind, Funding, ImageProvider, ImageRequest, ProviderDescriptor, ProviderError, ProviderOutput};

/// Default API base.
pub const DEFAULT_BASE: &str = "https://external.api.recraft.ai/v1";

/// Image-to-image strength for edits (an adapter default: 0 is nearly identical, 1 least similar).
pub const EDIT_STRENGTH: &str = "0.5";

/// Models that take `negative_prompt` (the docs say V2 and V3 only).
const NEGATIVE_MODELS: [&str; 2] = ["recraftv3", "recraftv2"];

/// USD per credit: the pricing page lists 35 credits as $0.035.
const USD_PER_CREDIT: f64 = 0.001;

/// What goes on the wire.
#[derive(Debug, Clone)]
pub(crate) enum Wire {
    Json(Value),
    Form(Multipart),
}

/// Adapter state.
pub struct RecraftProvider {
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    base: String,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
}

impl RecraftProvider {
    /// Build the adapter.
    pub fn new(cfg: ProviderConfig, key: Option<String>, transport: Arc<dyn HttpTransport>, policy: HostPolicy) -> Self {
        let base = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_BASE.to_string()).trim_end_matches('/').to_string();
        RecraftProvider { desc: descriptor_from(include_str!("../data/recraft.json")), cfg, key: key.map(ApiKey::new), base, transport, policy }
    }

    fn secrets(&self) -> Vec<String> {
        self.key.iter().map(|k| k.expose().to_string()).collect()
    }

    fn check_inputs(&self, req: &ImageRequest) -> Result<(), ProviderError> {
        if req.model.ends_with("_vector") || req.model.contains("vector") {
            return Err(ProviderError::unsupported(format!(
                "{} is a vector model and returns SVG. This tool needs a PNG, so only Recraft's raster models are offered",
                req.model
            )));
        }
        check_model(&self.desc, req)?;
        let styles = req.model == "recraftv4_styles";
        if styles {
            if req.edit.is_some() {
                return Err(ProviderError::unsupported(
                    "recraftv4_styles generates from style references and cannot edit: image-to-image does not support V4 styles",
                ));
            }
            if req.refs.is_empty() {
                return Err(ProviderError::unsupported("recraftv4_styles always needs a style: give it 1 to 10 style anchor references"));
            }
            if req.refs.iter().any(|r| r.role != RefRole::StyleAnchor) {
                return Err(ProviderError::unsupported("recraftv4_styles takes style anchor references only"));
            }
        } else if req.edit.is_none() && !req.refs.is_empty() {
            return Err(ProviderError::unsupported(format!(
                "{} takes no reference images on a generation. Use recraftv4_styles for style anchors",
                req.model
            )));
        }
        Ok(())
    }

    /// Route and body for a request. Used by contract tests to check the wire format.
    pub(crate) fn build(&self, req: &ImageRequest) -> Result<(String, Wire), ProviderError> {
        self.check_inputs(req)?;
        let (w, h) = self.prepare(req)?;
        let seed = req.seed.map(|s| s % 2_147_483_648);
        let negative = (!req.negative.is_empty() && NEGATIVE_MODELS.contains(&req.model.as_str())).then(|| req.negative.join(", "));
        if let Some(edit) = &req.edit {
            let mut f = Multipart::new();
            f.file("image", "image.png", "image/png", &edit.base_png);
            f.text("prompt", req.prompt.clone());
            f.text("strength", EDIT_STRENGTH);
            f.text("model", req.model.clone());
            f.text("n", "1");
            f.text("response_format", "b64_json");
            f.text("image_format", "png");
            if let Some(s) = seed {
                f.text("random_seed", s.to_string());
            }
            // negative_prompt is V3 only on this route
            if let (Some(n), "recraftv3") = (&negative, req.model.as_str()) {
                f.text("negative_prompt", n.clone());
            }
            return Ok(("/images/imageToImage".into(), Wire::Form(f)));
        }
        if !req.refs.is_empty() {
            let mut f = Multipart::new();
            f.text("model", req.model.clone());
            f.text("prompt", req.prompt.clone());
            f.text("n", "1");
            f.text("size", format!("{w}x{h}"));
            f.text("response_format", "b64_json");
            f.text("image_format", "png");
            if let Some(s) = seed {
                f.text("random_seed", s.to_string());
            }
            for (i, r) in req.refs.iter().enumerate() {
                f.file("style_references", &format!("style{i}.png"), "image/png", &r.png);
            }
            return Ok(("/images/generations".into(), Wire::Form(f)));
        }
        let mut body = json!({
            "model": req.model,
            "prompt": req.prompt,
            "n": 1,
            "size": format!("{w}x{h}"),
            "response_format": "b64_json",
            "image_format": "png",
        });
        if let Some(s) = seed {
            body["random_seed"] = json!(s);
        }
        if let Some(n) = negative {
            body["negative_prompt"] = json!(n);
        }
        Ok(("/images/generations".into(), Wire::Json(body)))
    }

    fn call(&self, req: &ImageRequest) -> Result<ProviderOutput, ProviderError> {
        let key = self.key.as_ref().ok_or_else(|| ProviderError::not_configured("RECRAFT_API_KEY is not set"))?;
        let (path, wire) = self.build(req)?;
        let url = format!("{}{path}", self.base);
        let http = match wire {
            Wire::Json(body) => HttpRequest::post_json(url, &body),
            Wire::Form(form) => form.into_request(url),
        }
        .header("Authorization", &format!("Bearer {}", key.expose()));
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
        let item = v.pointer("/data/0").ok_or_else(|| malformed("the response has no data[0]"))?;
        let (bytes, mime) = if let Some(b) = item.get("b64_json").and_then(Value::as_str) {
            let bytes = b64_decode(b)?;
            let mime = sniff_mime(&bytes).to_string();
            (bytes, mime)
        } else if let Some(url) = item.get("url").and_then(Value::as_str) {
            // fallback: a public link that expires after about 24 hours, so the key is not sent to it
            let dl = send(&*self.transport, &self.policy, &HttpRequest::get(url)).map_err(|mut e| {
                e.message = format!("{} (the image was generated and billed: fetch the link again, or reconcile)", e.message);
                e.retry = RetryClass::ReconcileFirst;
                e
            })?;
            if dl.status != 200 {
                let mut e = status_error(dl.status, &dl.body, &secrets);
                e.retry = RetryClass::ReconcileFirst;
                return Err(e);
            }
            let mime = sniff_mime(&dl.body).to_string();
            (dl.body, mime)
        } else {
            return Err(malformed("data[0] has neither b64_json nor url"));
        };
        if mime == "application/octet-stream" {
            return Err(malformed("the image is not a PNG, JPEG or WebP file (a vector model returns SVG)"));
        }
        let credits = v.get("credits").and_then(Value::as_f64);
        let table = self.desc.model(&req.model).and_then(|m| per_image_price(&m.cost, None)).map(|p| p.0);
        let actual_usd = credits.map(|c| (c * USD_PER_CREDIT * 1e6).round() / 1e6).or(table);
        let mut notes = Vec::new();
        if let Some(id) = item.get("image_id").and_then(Value::as_str) {
            notes.push(format!("image id: {}", redact(id, &secrets)));
        }
        if !req.negative.is_empty() && !NEGATIVE_MODELS.contains(&req.model.as_str()) {
            notes.push("negative prompt not sent: Recraft documents negative_prompt for V2 and V3 only".to_string());
        }
        Ok(ProviderOutput {
            image: bytes,
            mime,
            provider_request_id: item.get("image_id").and_then(Value::as_str).map(str::to_string),
            usage: credits.map(|c| json!({"credits": c})),
            usage_unknown: actual_usd.is_none(),
            actual_usd,
            model_echo: Some(req.model.clone()),
            notes,
            job: None,
        })
    }
}

impl ImageProvider for RecraftProvider {
    fn id(&self) -> &str {
        "recraft"
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
        self.desc.recommended().map(|m| m.id.clone()).unwrap_or_else(|| "recraftv4_1".into())
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        self.check_inputs(req)?;
        let model = check_model(&self.desc, req)?;
        choose_size(&model.sizes, [req.width, req.height])
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        self.check_inputs(req)?;
        let model = check_model(&self.desc, req)?;
        let (base, _) =
            per_image_price(&model.cost, None).ok_or_else(|| ProviderError::unsupported(format!("{} has no price in the table", model.id)))?;
        let style_refs = req.model == "recraftv4_styles" && !req.refs.is_empty();
        let usd = if style_refs { base + 0.005 } else { base };
        Ok(CostEstimate {
            usd: (usd * 1e6).round() / 1e6,
            basis: format!(
                "Recraft price table retrieved {}: {} raster at {base} USD per image{}",
                self.desc.retrieved,
                model.id,
                if style_refs { " plus 0.005 USD per request for style references" } else { "" }
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
    if config.provider("recraft").enabled == Some(false) {
        return;
    }
    let cfg = config.provider("recraft");
    match policy_for(&["external.api.recraft.ai", ".recraft.ai"], cfg.base_url.as_deref(), env) {
        Ok(policy) => {
            let key = read_key("recraft", &cfg, env);
            reg.insert(Arc::new(RecraftProvider::new(cfg, key, transport, policy)));
        }
        Err(why) => reg.problem("recraft", why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::b64;
    use crate::transport::{FixtureTransport, Method};
    use crate::types::{EditSpec, ProviderRef};
    use imagegen_core::config::MapEnv;
    use imagegen_core::pixels::{encode_png, Rgba};

    // DOCUMENTED SHAPE, NOT RECORDED: bodies below follow the request tables and the image-to-image response
    // example on recraft.ai/docs (read 2026-11-02). Nothing was returned by Recraft. The b64_json response
    // field name is inferred from the documented request value, which the pages do not show in a response.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [10, 200, 30, 255])).unwrap()
    }

    fn provider(fixture: Arc<FixtureTransport>, key: Option<&str>) -> RecraftProvider {
        RecraftProvider::new(
            ProviderConfig::default(),
            key.map(str::to_string),
            fixture,
            HostPolicy::api(&["external.api.recraft.ai", ".recraft.ai"]),
        )
    }

    fn request(model: &str) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "an iron wall".into(),
            negative: vec!["text".into()],
            width: 100,
            height: 88,
            transparent: true,
            quality: None,
            refs: vec![],
            edit: None,
            idempotency_key: None,
            seed: Some(3),
        }
    }

    fn style(n: usize) -> Vec<ProviderRef> {
        (0..n).map(|_| ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }).collect()
    }

    fn ok_b64(extra: Value) -> Vec<u8> {
        let mut v =
            json!({"created": 1759140000, "credits": 35, "data": [{"image_id": "11111111-2222-3333-4444-555555555555", "b64_json": b64(&png())}]});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn documented_shape_generation_json_asks_for_png_and_a_table_size() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("rc-test-000000"));
        let (path, Wire::Json(b)) = p.build(&request("recraftv4_1")).unwrap() else { panic!("json expected") };
        assert_eq!(path, "/images/generations");
        assert_eq!(
            (b["model"].as_str(), b["n"].as_u64(), b["response_format"].as_str(), b["image_format"].as_str()),
            (Some("recraftv4_1"), Some(1), Some("b64_json"), Some("png"))
        );
        assert_eq!(b["size"], "1152x896");
        assert_eq!(b["random_seed"], 3);
        assert!(b.get("negative_prompt").is_none(), "negative_prompt is V2 and V3 only");
        assert!(b.get("style").is_none() && b.get("background").is_none(), "no vector style and no undocumented transparency flag");
        let (_, Wire::Json(b)) = p.build(&request("recraftv3")).unwrap() else { panic!("json expected") };
        assert_eq!(b["negative_prompt"], "text");
        assert_eq!(b["size"], "1280x1024");
    }

    #[test]
    fn documented_shape_style_references_go_as_multipart_to_the_styles_model() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("rc-test-000000"));
        let mut r = request("recraftv4_styles");
        r.refs = style(3);
        let (path, Wire::Form(f)) = p.build(&r).unwrap() else { panic!("form expected") };
        assert_eq!(path, "/images/generations");
        assert_eq!(f.file_names(), vec!["style_references"; 3]);
        assert_eq!(f.get("model").as_deref(), Some("recraftv4_styles"));
        assert_eq!(f.get("size").as_deref(), Some("1152x896"));
        let e = p.estimate(&r).unwrap();
        assert!((e.usd - 0.04).abs() < 1e-9 && e.basis.contains("style references"), "{e:?}");
    }

    #[test]
    fn documented_shape_edit_is_image_to_image_with_strength() {
        let p = provider(Arc::new(FixtureTransport::new()), Some("rc-test-000000"));
        let mut r = request("recraftv3");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let (path, Wire::Form(f)) = p.build(&r).unwrap() else { panic!("form expected") };
        assert_eq!(path, "/images/imageToImage");
        assert_eq!(f.file_names(), vec!["image"]);
        assert_eq!((f.get("strength").as_deref(), f.get("model").as_deref()), (Some(EDIT_STRENGTH), Some("recraftv3")));
        assert_eq!(f.get("negative_prompt").as_deref(), Some("text"));
        r.model = "recraftv4_1".into();
        let Wire::Form(f) = p.build(&r).unwrap().1 else { panic!() };
        assert!(f.get("negative_prompt").is_none());
    }

    #[test]
    fn vector_models_and_unsupported_combinations_are_refused_before_any_request() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider(fixture.clone(), Some("rc-test-000000"));
        let ctx = CallContext::new();
        for m in ["recraftv3_vector", "recraftv4_1_vector"] {
            let e = p.generate(&request(m), &ctx).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Unsupported, "{m}");
            assert!(e.message.contains("vector") && e.message.contains("PNG"), "{}", e.message);
        }
        let mut r = request("recraftv4_1_flash");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        assert_eq!(p.edit(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        let mut r = request("recraftv2");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        assert_eq!(p.edit(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        let mut r = request("recraftv4_1");
        r.refs = style(1);
        assert!(p.generate(&r, &ctx).unwrap_err().message.contains("recraftv4_styles"));
        assert!(p.generate(&request("recraftv4_styles"), &ctx).unwrap_err().message.contains("needs a style"));
        let mut r = request("recraftv4_styles");
        r.refs = style(11);
        assert!(p.generate(&r, &ctx).unwrap_err().message.contains("at most 10"));
        let mut r = request("recraftv4_styles");
        r.refs = style(1);
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        assert_eq!(p.edit(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        let mut r = request("recraftv3");
        r.edit = Some(EditSpec { base_png: png(), mask_png: Some(png()) });
        assert!(p.edit(&r, &ctx).unwrap_err().message.contains("mask"));
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn documented_shape_success_decodes_b64_and_charges_the_reported_credits() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/images/generations", 200, &[], ok_b64(json!({})));
        let p = provider(fixture.clone(), Some("rc-test-000000"));
        let out = p.generate(&request("recraftv4_1"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!(out.actual_usd, Some(0.035));
        assert!(!out.usage_unknown && out.usage == Some(json!({"credits": 35.0})));
        assert_eq!(out.provider_request_id.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
        assert!(out.notes.iter().any(|n| n.contains("negative prompt not sent")));
        let sent = &fixture.requests()[0];
        assert_eq!(sent.url, "https://external.api.recraft.ai/v1/images/generations");
        assert!(sent.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer rc-test-000000"));
        assert!(sent.headers.iter().any(|(k, v)| k == "Content-Type" && v == "application/json"));
    }

    #[test]
    fn documented_shape_missing_credits_fall_back_to_the_table_price() {
        let fixture = Arc::new(FixtureTransport::new());
        let body = serde_json::to_vec(&json!({"data": [{"b64_json": b64(&png())}]})).unwrap();
        fixture.respond(Method::Post, "/images/generations", 200, &[], body);
        let p = provider(fixture, Some("rc-test-000000"));
        let out = p.generate(&request("recraftv2"), &CallContext::new()).unwrap();
        assert_eq!(out.actual_usd, Some(0.022));
    }

    #[test]
    fn documented_shape_url_results_are_downloaded_from_allowed_hosts_without_the_key() {
        let fixture = Arc::new(FixtureTransport::new());
        let body = serde_json::to_vec(&json!({"created": 1, "credits": 40, "data": [{"image_id": "i", "url": "https://img.recraft.ai/abc/x.png"}]}))
            .unwrap();
        fixture.respond(Method::Post, "/images/imageToImage", 200, &[], body);
        fixture.respond(Method::Get, "img.recraft.ai/abc/x.png", 200, &[], png());
        let p = provider(fixture.clone(), Some("rc-test-000000"));
        let mut r = request("recraftv3");
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let out = p.edit(&r, &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.actual_usd, Some(0.04));
        assert!(fixture.requests()[1].headers.is_empty());
        // a foreign host is blocked and the error says the call was billed
        let fixture = Arc::new(FixtureTransport::new());
        let body = serde_json::to_vec(&json!({"data": [{"url": "https://evil.example/x.png"}]})).unwrap();
        fixture.respond(Method::Post, "/images/generations", 200, &[], body);
        let p = provider(fixture, Some("rc-test-000000"));
        let err = p.generate(&request("recraftv4_1"), &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.retry), (ErrorKind::Blocked, RetryClass::ReconcileFirst));
    }

    #[test]
    fn documented_shape_errors_map_and_never_leak_the_key() {
        let key = "rc-test-secret-123456";
        let cases: [(u16, String, ErrorKind, RetryClass); 5] = [
            (401, "request unauthorized".into(), ErrorKind::Auth, RetryClass::RequiresAuthorization),
            (400, format!("unsupported size for key {key}"), ErrorKind::BadRequest, RetryClass::Terminal),
            (404, "404 page not found".into(), ErrorKind::BadRequest, RetryClass::Terminal),
            (429, "rate limit".into(), ErrorKind::RateLimited, RetryClass::SafeSameInputs),
            (503, "later".into(), ErrorKind::ServerError, RetryClass::ReconcileFirst),
        ];
        for (status, body, kind, retry) in cases {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/images/generations", status, &[("content-type", "text/plain")], body);
            let p = provider(fixture, Some(key));
            let err = p.generate(&request("recraftv4_1"), &CallContext::new()).unwrap_err();
            assert_eq!((err.kind, err.retry), (kind, retry), "status {status}");
            assert!(!err.message.contains(key), "key leaked: {}", err.message);
        }
    }

    #[test]
    fn documented_shape_malformed_success_demands_reconciliation() {
        for body in
            [b"nope".to_vec(), br#"{"data":[]}"#.to_vec(), br#"{"data":[{"image_id":"x"}]}"#.to_vec(), br#"{"data":[{"b64_json":"%%%"}]}"#.to_vec()]
        {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/images/generations", 200, &[], body);
            let p = provider(fixture, Some("rc-test-000000"));
            let err = p.generate(&request("recraftv4_1"), &CallContext::new()).unwrap_err();
            assert_eq!(err.retry, RetryClass::ReconcileFirst, "{err}");
        }
        // SVG bytes (a vector result) are not an image this tool can use
        let fixture = Arc::new(FixtureTransport::new());
        let body = serde_json::to_vec(&json!({"data": [{"b64_json": b64(b"<svg xmlns='http://www.w3.org/2000/svg'/>")}]})).unwrap();
        fixture.respond(Method::Post, "/images/generations", 200, &[], body);
        let p = provider(fixture, Some("rc-test-000000"));
        assert_eq!(p.generate(&request("recraftv4_1"), &CallContext::new()).unwrap_err().kind, ErrorKind::MalformedResponse);
    }

    #[test]
    fn missing_key_fails_first_and_a_foreign_base_is_blocked() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider(fixture.clone(), None);
        assert!(!p.is_configured());
        assert_eq!(p.generate(&request("recraftv4_1"), &CallContext::new()).unwrap_err().kind, ErrorKind::NotConfigured);
        let cfg = ProviderConfig { base_url: Some("https://evil.example/v1".into()), ..Default::default() };
        let p = RecraftProvider::new(cfg, Some("rc-test-000000".into()), fixture.clone(), HostPolicy::api(&["external.api.recraft.ai"]));
        assert_eq!(p.generate(&request("recraftv4_1"), &CallContext::new()).unwrap_err().kind, ErrorKind::Blocked);
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn estimates_come_from_the_table() {
        let p = provider(Arc::new(FixtureTransport::new()), None);
        for (m, usd) in [("recraftv4_1", 0.035), ("recraftv4_1_flash", 0.007), ("recraftv4", 0.04), ("recraftv3", 0.04), ("recraftv2", 0.022)] {
            let e = p.estimate(&request(m)).unwrap();
            assert!((e.usd - usd).abs() < 1e-9 && !e.usage_unknown, "{m}: {}", e.usd);
            assert!(e.basis.contains("2026-11-02"), "{}", e.basis);
        }
        assert!(p.estimate(&request("recraftv3_vector")).is_err());
    }

    #[test]
    fn table_is_marked_not_live_verified_and_holds_raster_models_only() {
        let raw: Value = serde_json::from_str(include_str!("../data/recraft.json")).unwrap();
        assert_eq!(raw["live_verified"], json!(false));
        assert!(raw["doc_urls"].as_array().map(|a| a.len() > 3).unwrap_or(false) && raw["checked"] == json!("2026-11-02"));
        let d = descriptor_from(include_str!("../data/recraft.json"));
        assert_eq!(d.models.iter().filter(|m| m.recommended).count(), 1);
        assert!(d.models.iter().all(|m| !m.id.contains("vector") && m.notes.contains("Not live-verified")));
        assert!(d.terms_note.contains("Not live-verified") && d.terms_note.contains("transparent"));
        assert_eq!(provider(Arc::new(FixtureTransport::new()), None).default_model(false), "recraftv4_1");
    }

    #[test]
    fn registry_builds_it_with_its_image_hosts() {
        let fixture: Arc<dyn HttpTransport> = Arc::new(FixtureTransport::new());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::from_pairs(&[("RECRAFT_API_KEY", "rc-test-1")]), fixture.clone());
        assert!(reg.get("recraft").unwrap().is_configured());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::default(), fixture.clone());
        assert!(!reg.get("recraft").unwrap().is_configured());
        let cfg = Config::parse("[providers.recraft]\nenabled = false\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &cfg, &MapEnv::default(), fixture);
        assert!(reg.get("recraft").is_none());
    }
}
