//! Local Stable Diffusion WebUI servers (`automatic1111`, `forge`): the `/sdapi/v1/txt2img` and `img2img` API.
//!
//! Built from the WebUI wiki and source read on 2026-11-02 and **not live-verified**: no server was started and
//! no call was made. The API exists only when the WebUI starts with `--api`. The server listens on
//! `http://127.0.0.1:7860` by default (documented `--port 7860`, `--listen` off), which is the endpoint used when
//! none is configured, but the provider only reports itself configured when `endpoint` is set in the project
//! config, and nothing is probed at build time. The endpoint must be a loopback host unless the user's own
//! environment allows another one (`policy_for`), and then it must be https on port 443. No key is needed. An
//! `api_key_env` holding `user:password` is sent as HTTP Basic auth for servers started with `--api-auth`.
//!
//! Forge is a fork that keeps the same API, shared here on the documented assumption that its README lists the
//! API endpoints as working. Both ids use one implementation and each reads its own config section.

use std::sync::Arc;
use std::time::Duration;

use imagegen_core::config::{Config, EnvSource, ProviderConfig};
use imagegen_core::error::RetryClass;
use serde_json::{json, Value};

use crate::common::{b64, b64_decode, check_model, descriptor_from, parse_data_url, sniff_mime, status_error, ApiKey};
use crate::registry::{policy_for, read_key, ProviderRegistry};
use crate::sizing::choose_size;
use crate::transport::{redact, send, HostPolicy, HttpRequest, HttpTransport};
use crate::types::{
    CallContext, CostEstimate, ErrorKind, Funding, ImageProvider, ImageRequest, ModelInfo, ProviderDescriptor, ProviderError, ProviderOutput,
    SizeRule,
};

/// Endpoint used when none is configured: the WebUI's documented default.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:7860";

/// The one table entry: whatever checkpoint the server has loaded.
pub const LOADED_CHECKPOINT: &str = "loaded-checkpoint";

/// img2img denoising strength for edits (an adapter default: the server's own default is 0.75).
pub const EDIT_DENOISING: f64 = 0.6;

/// Pixel floor for SDXL-class checkpoints, used when the quality is `high`.
const SDXL_FLOOR: u64 = 1_048_576;

/// Adapter state.
pub struct SdWebUiProvider {
    id: &'static str,
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    auth: Option<ApiKey>,
    endpoint: String,
    endpoint_set: bool,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
}

impl SdWebUiProvider {
    /// Build the adapter for `automatic1111` or `forge`. `auth` is an optional `user:password` for `--api-auth`.
    pub fn new(id: &'static str, cfg: ProviderConfig, auth: Option<String>, transport: Arc<dyn HttpTransport>, mut policy: HostPolicy) -> Self {
        let mut desc = descriptor_from(include_str!("../data/sdwebui.json"));
        desc.id = id.to_string();
        if id == "forge" {
            desc.display_name = "Stable Diffusion WebUI Forge (same API as AUTOMATIC1111)".to_string();
            desc.doc_url = "https://github.com/lllyasviel/stable-diffusion-webui-forge".to_string();
        }
        // a local render can take minutes
        policy.timeout = Duration::from_secs(900);
        let endpoint_set = cfg.endpoint.is_some();
        let endpoint = cfg.endpoint.clone().unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()).trim_end_matches('/').to_string();
        SdWebUiProvider { id, desc, cfg, auth: auth.map(ApiKey::new), endpoint, endpoint_set, transport, policy }
    }

    fn secrets(&self) -> Vec<String> {
        self.auth.iter().flat_map(|k| [k.expose().to_string(), b64(k.expose().as_bytes())]).collect()
    }

    /// The model entry for a request. Any model id other than the loaded checkpoint is a checkpoint name.
    fn table_request(req: &ImageRequest) -> ImageRequest {
        ImageRequest { model: LOADED_CHECKPOINT.to_string(), ..req.clone() }
    }

    fn checked<'a>(&'a self, req: &ImageRequest) -> Result<&'a ModelInfo, ProviderError> {
        let model = check_model(&self.desc, &Self::table_request(req))?;
        if req.edit.is_none() && !req.refs.is_empty() {
            return Err(ProviderError::unsupported(format!(
                "{} takes no references on a plain generation: an image sent to img2img is the picture to change, not a style anchor",
                self.id
            )));
        }
        Ok(model)
    }

    fn rule(&self, model: &ModelInfo, req: &ImageRequest) -> SizeRule {
        match (&model.sizes, self.quality(req).as_str()) {
            (SizeRule::Arbitrary { multiple, max_pixels, max_edge, max_ratio, .. }, "high") => SizeRule::Arbitrary {
                multiple: *multiple,
                min_pixels: SDXL_FLOOR,
                max_pixels: *max_pixels,
                max_edge: *max_edge,
                max_ratio: *max_ratio,
            },
            (rule, _) => rule.clone(),
        }
    }

    fn quality(&self, req: &ImageRequest) -> String {
        req.quality.clone().or_else(|| self.cfg.quality.clone()).unwrap_or_else(|| "medium".to_string())
    }

    /// Route and JSON body. Used by contract tests to check the wire format.
    pub(crate) fn build(&self, req: &ImageRequest) -> Result<(String, Value), ProviderError> {
        let model = self.checked(req)?;
        let (w, h) = choose_size(&self.rule(model, req), [req.width, req.height])?;
        let mut body = json!({
            "prompt": req.prompt,
            "width": w,
            "height": h,
            "batch_size": 1,
            "n_iter": 1,
            "send_images": true,
            "save_images": false,
        });
        if !req.negative.is_empty() {
            body["negative_prompt"] = json!(req.negative.join(", "));
        }
        if let Some(seed) = req.seed {
            body["seed"] = json!(seed % 4_294_967_296);
        }
        if req.model != LOADED_CHECKPOINT {
            body["override_settings"] = json!({"sd_model_checkpoint": req.model});
        }
        match &req.edit {
            None => Ok(("/sdapi/v1/txt2img".into(), body)),
            Some(edit) => {
                body["init_images"] = json!([b64(&edit.base_png)]);
                body["denoising_strength"] = json!(EDIT_DENOISING);
                Ok(("/sdapi/v1/img2img".into(), body))
            }
        }
    }

    fn call(&self, req: &ImageRequest) -> Result<ProviderOutput, ProviderError> {
        if !self.endpoint_set {
            return Err(ProviderError::not_configured(format!(
                "no endpoint is set. Add endpoint = \"{DEFAULT_ENDPOINT}\" (or your server's address) under [providers.{}] in the project config",
                self.id
            )));
        }
        let (path, body) = self.build(req)?;
        let mut http = HttpRequest::post_json(format!("{}{path}", self.endpoint), &body);
        if let Some(a) = &self.auth {
            http = http.header("Authorization", &format!("Basic {}", b64(a.expose().as_bytes())));
        }
        let secrets = self.secrets();
        let resp = send(&*self.transport, &self.policy, &http).map_err(|mut e| {
            if e.kind == ErrorKind::Transport {
                e.message = format!("{} (is the WebUI running with --api at {}?)", redact(&e.message, &secrets), self.endpoint);
            }
            e
        })?;
        if resp.status != 200 {
            let mut e = status_error(resp.status, &resp.body, &secrets);
            let hint = match resp.status {
                404 => Some("the server has no /sdapi routes: start the WebUI with --api"),
                401 => Some("the server wants credentials: it was started with --api-auth, so set api_key_env to a variable holding user:password"),
                _ => None,
            };
            if let Some(h) = hint {
                e.message = format!("{} ({h})", e.message);
            }
            return Err(e);
        }
        let malformed = |why: &str| {
            ProviderError::new(
                ErrorKind::MalformedResponse,
                RetryClass::ReconcileFirst,
                format!("{why}. The server accepted the request, so a render may have run"),
            )
            .with_status(200)
        };
        let v: Value = serde_json::from_slice(&resp.body).map_err(|e| malformed(&format!("the response is not valid JSON ({e})")))?;
        let first = v.pointer("/images/0").and_then(Value::as_str).ok_or_else(|| malformed("the response has no images[0]"))?;
        let bytes = match parse_data_url(first) {
            Some((_, b)) => b,
            None => b64_decode(first)?,
        };
        let mime = sniff_mime(&bytes);
        if mime == "application/octet-stream" {
            return Err(malformed("images[0] is not a PNG, JPEG or WebP image"));
        }
        let mut notes = Vec::new();
        if let Some(seed) = v
            .get("info")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|i| i.get("seed").and_then(Value::as_i64))
        {
            notes.push(format!("seed: {seed}"));
        }
        Ok(ProviderOutput {
            image: bytes,
            mime: mime.to_string(),
            provider_request_id: None,
            usage: None,
            actual_usd: Some(0.0),
            usage_unknown: false,
            model_echo: Some(req.model.clone()),
            notes,
            job: None,
        })
    }
}

impl ImageProvider for SdWebUiProvider {
    fn id(&self) -> &str {
        self.id
    }

    fn descriptor(&self) -> &ProviderDescriptor {
        &self.desc
    }

    /// True when an endpoint is set in the config. The server is never contacted to find out.
    fn is_configured(&self) -> bool {
        self.endpoint_set
    }

    fn default_model(&self, _editing: bool) -> String {
        self.cfg.model.clone().unwrap_or_else(|| LOADED_CHECKPOINT.to_string())
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        let model = self.checked(req)?;
        choose_size(&self.rule(model, req), [req.width, req.height])
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        self.checked(req)?;
        Ok(CostEstimate {
            usd: 0.0,
            basis: format!("{} runs on the local server: no provider fee (the cost is your own GPU time)", self.id),
            usage_unknown: false,
            funding: Funding::Local,
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
    for id in ["automatic1111", "forge"] {
        if config.provider(id).enabled == Some(false) {
            continue;
        }
        let cfg = config.provider(id);
        let endpoint = cfg.endpoint.clone().unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        match policy_for(&[], Some(&endpoint), env) {
            Ok(policy) => {
                let auth = read_key(id, &cfg, env);
                reg.insert(Arc::new(SdWebUiProvider::new(id, cfg, auth, transport.clone(), policy)));
            }
            Err(why) => reg.problem(id, why),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{FixtureTransport, Method, TransportError};
    use crate::types::{EditSpec, ProviderRef};
    use imagegen_core::config::MapEnv;
    use imagegen_core::pixels::{encode_png, Rgba};
    use imagegen_core::refs::RefRole;

    // DOCUMENTED SHAPE, NOT RECORDED: bodies below follow modules/api/models.py and the wiki page (read
    // 2026-11-02): a JSON request and a response of `images` (base64 list), `parameters` and `info`.
    // No WebUI server produced any of it.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [10, 200, 30, 255])).unwrap()
    }

    fn cfg(endpoint: Option<&str>) -> ProviderConfig {
        ProviderConfig { endpoint: endpoint.map(str::to_string), ..Default::default() }
    }

    fn provider(id: &'static str, fixture: Arc<FixtureTransport>, endpoint: Option<&str>) -> SdWebUiProvider {
        let c = cfg(endpoint);
        let policy = policy_for(&[], Some(c.endpoint.as_deref().unwrap_or(DEFAULT_ENDPOINT)), &MapEnv::default()).unwrap();
        SdWebUiProvider::new(id, c, None, fixture, policy)
    }

    fn request() -> ImageRequest {
        ImageRequest {
            model: LOADED_CHECKPOINT.into(),
            prompt: "an iron wall".into(),
            negative: vec!["text".into(), "blur".into()],
            width: 100,
            height: 88,
            transparent: false,
            quality: None,
            refs: vec![],
            edit: None,
            idempotency_key: None,
            seed: Some(5),
        }
    }

    fn ok_body() -> Vec<u8> {
        serde_json::to_vec(&json!({"images": [b64(&png())], "parameters": {"prompt": "an iron wall"}, "info": "{\"seed\": 5, \"steps\": 20}"}))
            .unwrap()
    }

    #[test]
    fn documented_shape_txt2img_body_uses_a_multiple_of_8_size_and_the_negative_prompt() {
        let p = provider("automatic1111", Arc::new(FixtureTransport::new()), Some("http://127.0.0.1:7860"));
        let (path, b) = p.build(&request()).unwrap();
        assert_eq!(path, "/sdapi/v1/txt2img");
        let (w, h) = (b["width"].as_u64().unwrap(), b["height"].as_u64().unwrap());
        assert!(w % 8 == 0 && h % 8 == 0 && w * h >= 262_144 && w > h, "{w}x{h}");
        assert_eq!(b["negative_prompt"], "text, blur");
        assert_eq!((b["batch_size"].as_u64(), b["n_iter"].as_u64(), b["seed"].as_u64()), (Some(1), Some(1), Some(5)));
        assert_eq!((b["send_images"].as_bool(), b["save_images"].as_bool()), (Some(true), Some(false)));
        assert!(b.get("override_settings").is_none() && b.get("init_images").is_none());
        assert!(b.get("steps").is_none() && b.get("sampler_name").is_none(), "the server's own defaults apply");
    }

    #[test]
    fn quality_high_raises_the_pixel_floor_for_sdxl_class_models() {
        let p = provider("forge", Arc::new(FixtureTransport::new()), Some("http://127.0.0.1:7860"));
        let mut r = request();
        let (w, h) = p.prepare(&r).unwrap();
        assert!(u64::from(w) * u64::from(h) < SDXL_FLOOR, "{w}x{h}");
        r.quality = Some("high".into());
        let (w, h) = p.prepare(&r).unwrap();
        assert!(u64::from(w) * u64::from(h) >= SDXL_FLOOR && w % 8 == 0 && h % 8 == 0, "{w}x{h}");
    }

    #[test]
    fn documented_shape_img2img_carries_the_base_and_a_named_checkpoint_is_an_override() {
        let p = provider("automatic1111", Arc::new(FixtureTransport::new()), Some("http://127.0.0.1:7860"));
        let mut r = request();
        r.model = "dreamshaper_8.safetensors".into();
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let (path, b) = p.build(&r).unwrap();
        assert_eq!(path, "/sdapi/v1/img2img");
        assert_eq!(b["init_images"], json!([b64(&png())]));
        assert_eq!(b["denoising_strength"], EDIT_DENOISING);
        assert_eq!(b["override_settings"], json!({"sd_model_checkpoint": "dreamshaper_8.safetensors"}));
    }

    #[test]
    fn unsupported_combinations_fail_before_any_request() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider("automatic1111", fixture.clone(), Some("http://127.0.0.1:7860"));
        let ctx = CallContext::new();
        let mut r = request();
        r.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }];
        assert!(p.generate(&r, &ctx).unwrap_err().message.contains("style anchor"));
        let mut r = request();
        r.edit = Some(EditSpec { base_png: png(), mask_png: Some(png()) });
        assert!(p.edit(&r, &ctx).unwrap_err().message.contains("mask"));
        let mut r = request();
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        r.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }];
        assert!(p.edit(&r, &ctx).unwrap_err().message.contains("at most 1"));
        let mut r = request();
        r.width = 1000;
        r.height = 10;
        assert_eq!(p.generate(&r, &ctx).unwrap_err().kind, ErrorKind::Unsupported);
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn documented_shape_success_decodes_the_first_image_and_reads_the_seed() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/sdapi/v1/txt2img", 200, &[], ok_body());
        let p = provider("automatic1111", fixture.clone(), Some("http://127.0.0.1:7860"));
        let out = p.generate(&request(), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!((out.actual_usd, out.usage_unknown), (Some(0.0), false));
        assert_eq!(out.notes, vec!["seed: 5".to_string()]);
        let sent = &fixture.requests()[0];
        assert_eq!(sent.url, "http://127.0.0.1:7860/sdapi/v1/txt2img");
        assert!(sent.headers.iter().all(|(k, _)| k != "Authorization"), "no key is needed");
    }

    #[test]
    fn documented_shape_data_url_images_and_the_edit_route_work() {
        let fixture = Arc::new(FixtureTransport::new());
        let body =
            serde_json::to_vec(&json!({"images": [crate::common::data_url("image/png", &png())], "parameters": {}, "info": "not json"})).unwrap();
        fixture.respond(Method::Post, "/sdapi/v1/img2img", 200, &[], body);
        let p = provider("forge", fixture, Some("http://127.0.0.1:7860/"));
        let mut r = request();
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        let out = p.edit(&r, &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert!(out.notes.is_empty());
    }

    #[test]
    fn basic_auth_is_sent_only_when_configured_and_never_echoed() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/sdapi/v1/txt2img", 401, &[], r#"{"detail":"Not authenticated for user:secretpass"}"#);
        let c = cfg(Some("http://127.0.0.1:7860"));
        let policy = policy_for(&[], c.endpoint.as_deref(), &MapEnv::default()).unwrap();
        let p = SdWebUiProvider::new("automatic1111", c, Some("user:secretpass".into()), fixture.clone(), policy);
        let err = p.generate(&request(), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Auth);
        assert!(!err.message.contains("secretpass") && err.message.contains("--api-auth"), "{}", err.message);
        let sent = &fixture.requests()[0];
        assert!(sent.headers.iter().any(|(k, v)| k == "Authorization" && *v == format!("Basic {}", b64(b"user:secretpass"))));
    }

    #[test]
    fn errors_map_with_helpful_hints() {
        let cases: [(u16, &str, ErrorKind); 4] = [
            (404, r#"{"detail":"Not Found"}"#, ErrorKind::BadRequest),
            (422, r#"{"detail":[{"loc":["body","width"],"msg":"bad"}]}"#, ErrorKind::BadRequest),
            (500, r#"{"error":"OutOfMemoryError","detail":"","body":"","errors":"CUDA out of memory"}"#, ErrorKind::ServerError),
            (429, "slow", ErrorKind::RateLimited),
        ];
        for (status, body, kind) in cases {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/sdapi/v1/txt2img", status, &[], body);
            let p = provider("automatic1111", fixture, Some("http://127.0.0.1:7860"));
            let err = p.generate(&request(), &CallContext::new()).unwrap_err();
            assert_eq!(err.kind, kind, "status {status}");
            if status == 404 {
                assert!(err.message.contains("--api"), "{}", err.message);
            }
        }
        // a server that is not running
        let fixture = Arc::new(FixtureTransport::new());
        fixture.fail(Method::Post, "/sdapi/v1/txt2img", TransportError::NotSent("connection refused".into()));
        let p = provider("automatic1111", fixture, Some("http://127.0.0.1:7860"));
        let err = p.generate(&request(), &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.retry), (ErrorKind::Transport, RetryClass::SafeSameInputs));
        assert!(err.message.contains("--api"));
    }

    #[test]
    fn malformed_success_demands_reconciliation() {
        for body in [
            b"nope".to_vec(),
            br#"{"images":[]}"#.to_vec(),
            br#"{"images":["%%%"]}"#.to_vec(),
            serde_json::to_vec(&json!({"images": [b64(b"<html>")]})).unwrap(),
        ] {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/sdapi/v1/txt2img", 200, &[], body);
            let p = provider("automatic1111", fixture, Some("http://127.0.0.1:7860"));
            let err = p.generate(&request(), &CallContext::new()).unwrap_err();
            assert_eq!(err.retry, RetryClass::ReconcileFirst, "{err}");
        }
    }

    #[test]
    fn configured_means_an_endpoint_is_set_and_nothing_is_probed() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = provider("automatic1111", fixture.clone(), None);
        assert!(!p.is_configured());
        let err = p.generate(&request(), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotConfigured);
        assert!(err.message.contains("127.0.0.1:7860") && err.message.contains("[providers.automatic1111]"));
        assert!(fixture.requests().is_empty(), "no network call without an endpoint, and none at build time");
        assert!(provider("forge", fixture.clone(), Some("http://127.0.0.1:7860")).is_configured());
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn estimates_are_local_and_free() {
        let p = provider("forge", Arc::new(FixtureTransport::new()), None);
        let e = p.estimate(&request()).unwrap();
        assert_eq!((e.usd, e.funding, e.usage_unknown), (0.0, Funding::Local, false));
        assert!(e.basis.contains("local"));
        assert_eq!(p.default_model(false), LOADED_CHECKPOINT);
    }

    #[test]
    fn registry_registers_both_ids_with_their_own_config_and_loopback_only() {
        let fixture: Arc<dyn HttpTransport> = Arc::new(FixtureTransport::new());
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &Config::default(), &MapEnv::default(), fixture.clone());
        assert_eq!(reg.ids(), vec!["automatic1111", "forge"]);
        assert!(reg.configured_ids().is_empty(), "no endpoint set, so neither counts as configured");
        let cfg = Config::parse("[providers.forge]\nendpoint = \"http://localhost:7861\"\n[providers.automatic1111]\nenabled = false\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &cfg, &MapEnv::default(), fixture.clone());
        assert_eq!(reg.ids(), vec!["forge"]);
        assert_eq!(reg.configured_ids(), vec!["forge"]);
        assert_eq!(reg.get("forge").unwrap().descriptor().id, "forge");
        // a LAN or remote host is refused unless the user's environment names it
        let cfg = Config::parse("[providers.automatic1111]\nendpoint = \"http://192.168.1.20:7860\"\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &cfg, &MapEnv::default(), fixture.clone());
        assert!(reg.get("automatic1111").is_none() && reg.problems()["automatic1111"].contains("HOI4_IMAGEGEN_ALLOWED_HOSTS"));
        let env = MapEnv::from_pairs(&[("HOI4_IMAGEGEN_ALLOWED_HOSTS", "192.168.1.20")]);
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &cfg, &env, fixture.clone());
        assert!(reg.get("automatic1111").is_some());
        let cfg = Config::parse("[providers.forge]\nendpoint = \"not a url\"\n").unwrap();
        let mut reg = ProviderRegistry::new();
        register(&mut reg, &cfg, &MapEnv::default(), fixture);
        assert!(reg.problems().contains_key("forge"));
    }

    #[test]
    fn a_remote_endpoint_never_receives_requests_over_plain_http() {
        // even when allowed by the environment, a non-loopback host must be https on 443
        let fixture = Arc::new(FixtureTransport::new());
        let c = cfg(Some("http://192.168.1.20:7860"));
        let env = MapEnv::from_pairs(&[("HOI4_IMAGEGEN_ALLOWED_HOSTS", "192.168.1.20")]);
        let policy = policy_for(&[], c.endpoint.as_deref(), &env).unwrap();
        let p = SdWebUiProvider::new("automatic1111", c, None, fixture.clone(), policy);
        assert_eq!(p.generate(&request(), &CallContext::new()).unwrap_err().kind, ErrorKind::Blocked);
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn table_is_marked_not_live_verified_and_names_the_default_endpoint() {
        let raw: Value = serde_json::from_str(include_str!("../data/sdwebui.json")).unwrap();
        assert_eq!(raw["live_verified"], json!(false));
        assert_eq!(raw["default_endpoint"], json!(DEFAULT_ENDPOINT));
        assert!(raw["doc_urls"].as_array().map(|a| a.len() > 3).unwrap_or(false) && raw["checked"] == json!("2026-11-02"));
        let d = descriptor_from(include_str!("../data/sdwebui.json"));
        assert_eq!(d.models.iter().filter(|m| m.recommended).count(), 1);
        assert!(d.terms_note.contains("Not live-verified") && d.terms_note.contains("--api") && d.models[0].notes.contains("Not live-verified"));
    }
}
