//! OpenAI-compatible image APIs: a generic `openai_compatible` provider (any `/images/generations` server) and
//! presets for xAI (`xai`), Together AI (`together`), OpenRouter (`openrouter`) and Hugging Face Inference
//! Providers (`huggingface`). Microsoft Foundry is deferred and is not registered here.
//!
//! One adapter core ([`CompatProvider`]) serves all of them. A [`Protocol`] value picks the request and response
//! dialect, and each provider keeps its model table in `data/<id>.json`.
//!
//! **Evidence status: documented shape, not recorded.** No API key was available and no network call was made.
//! Every request and response shape below comes from the vendor documentation named in the data files
//! (`doc_urls`, `checked`), and every test fixture is built from that documented schema. Nothing here is
//! live-verified.

use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource, ProviderConfig};
use imagegen_core::error::RetryClass;
use serde_json::{json, Value};

use crate::common::{
    b64_decode, check_model, data_url, descriptor_from, megapixels, parse_data_url, per_image_price, sniff_mime, status_error, ApiKey,
};
use crate::registry::{policy_for, read_key, ProviderRegistry};
use crate::sizing::{choose_size, nearest_ratio, tier_size};
use crate::transport::{is_loopback_host, parse_url, redact, send, HostPolicy, HttpRequest, HttpTransport};
use crate::types::{
    AlphaSupport, CallContext, CostEstimate, CostModel, ErrorKind, Funding, ImageProvider, ImageRequest, ModelInfo, ModelStatus, ProviderDescriptor,
    ProviderError, ProviderOutput, SizeRule,
};

/// Which dialect a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Any server with `POST <base>/images/generations`.
    Generic,
    /// xAI: `/images/generations` and `/images/edits`, `aspect_ratio` plus `resolution`.
    Xai,
    /// Together AI: `width` and `height` (or `aspect_ratio` for Kontext), `response_format: base64`.
    Together,
    /// OpenRouter: `POST /api/v1/images`.
    OpenRouter,
    /// Hugging Face router, hf-inference text-to-image: raw image bytes back.
    HuggingFace,
}

struct Spec {
    id: &'static str,
    json: &'static str,
    default_base: Option<&'static str>,
    hosts: &'static [&'static str],
    key_var: &'static str,
}

const ALL: [Protocol; 5] = [Protocol::Generic, Protocol::Xai, Protocol::Together, Protocol::OpenRouter, Protocol::HuggingFace];

/// Default model-id style check for models that become part of a URL path (Hugging Face).
fn valid_hf_model(id: &str) -> bool {
    let part = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) && p != "." && p != "..";
    let mut it = id.split('/');
    match (it.next(), it.next(), it.next()) {
        (Some(a), None, None) => part(a),
        (Some(a), Some(b), None) => part(a) && part(b),
        _ => false,
    }
}

impl Protocol {
    fn spec(self) -> Spec {
        match self {
            Protocol::Generic => Spec {
                id: "openai_compatible",
                json: include_str!("../data/openai_compatible.json"),
                default_base: None,
                hosts: &[],
                key_var: "OPENAI_COMPATIBLE_API_KEY",
            },
            Protocol::Xai => Spec {
                id: "xai",
                json: include_str!("../data/xai.json"),
                default_base: Some("https://api.x.ai/v1"),
                hosts: &["api.x.ai", ".x.ai"],
                key_var: "XAI_API_KEY",
            },
            Protocol::Together => Spec {
                id: "together",
                json: include_str!("../data/together.json"),
                default_base: Some("https://api.together.ai/v1"),
                hosts: &["api.together.ai", "api.together.xyz"],
                key_var: "TOGETHER_API_KEY",
            },
            Protocol::OpenRouter => Spec {
                id: "openrouter",
                json: include_str!("../data/openrouter.json"),
                default_base: Some("https://openrouter.ai/api/v1"),
                hosts: &["openrouter.ai"],
                key_var: "OPENROUTER_API_KEY",
            },
            Protocol::HuggingFace => Spec {
                id: "huggingface",
                json: include_str!("../data/huggingface.json"),
                default_base: Some("https://router.huggingface.co"),
                hosts: &["router.huggingface.co"],
                key_var: "HF_TOKEN",
            },
        }
    }
}

/// Register this module's providers unless the project config disables them.
pub fn register(reg: &mut ProviderRegistry, config: &Config, env: &dyn EnvSource, transport: Arc<dyn HttpTransport>) {
    for proto in ALL {
        let spec = proto.spec();
        let cfg = config.provider(spec.id);
        if cfg.enabled == Some(false) {
            continue;
        }
        if proto == Protocol::Generic {
            match (&cfg.base_url, &cfg.model) {
                // not configured at all: not offered, and not a problem (the same as comfyui)
                (None, None) => continue,
                (None, Some(_)) => {
                    reg.problem(
                        spec.id,
                        "providers.openai_compatible.base_url is not set (the server's API base, for example https://host/v1)".into(),
                    );
                    continue;
                }
                (Some(_), None) => {
                    reg.problem(spec.id, "providers.openai_compatible.model is not set (the server's model id)".into());
                    continue;
                }
                _ => {}
            }
        }
        match policy_for(spec.hosts, cfg.base_url.as_deref(), env) {
            Ok(policy) => {
                let key = read_key(spec.id, &cfg, env);
                reg.insert(Arc::new(CompatProvider::new(proto, cfg, key, transport.clone(), policy)));
            }
            Err(why) => reg.problem(spec.id, why),
        }
    }
}

/// The adapter.
pub struct CompatProvider {
    proto: Protocol,
    id: &'static str,
    key_var: String,
    desc: ProviderDescriptor,
    cfg: ProviderConfig,
    key: Option<ApiKey>,
    base: String,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
}

/// A request ready to send.
#[derive(Debug, Clone)]
pub struct Built {
    /// Absolute URL.
    pub url: String,
    /// JSON body.
    pub body: Value,
    /// Options the API has no field for, reported with the result.
    pub notes: Vec<String>,
}

fn unknown_cost() -> CostModel {
    CostModel::UnpublishedProxy { proxy_usd_by_quality: vec![], proxy_of: "none: no price is documented".into() }
}

/// A model that is not in the reviewed table but that the project config names explicitly.
fn config_model(proto: Protocol, id: &str, recommended: bool) -> ModelInfo {
    let (sizes, notes) = match proto {
        Protocol::Generic => (
            SizeRule::Fixed { sizes: vec![[1024, 1024], [1536, 1024], [1024, 1536]] },
            "Named in the project config. The server's limits and price are unknown, so the common 1024 and 1536 sizes are asked for and the price estimate says unknown.",
        ),
        Protocol::OpenRouter => (
            SizeRule::AspectTiers {
                ratios: ["1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9"].iter().map(|s| s.to_string()).collect(),
                tier: "1K".into(),
            },
            "Named in the project config and not in the reviewed table: price, edit support and limits are unknown. usage.cost settles the real figure.",
        ),
        _ => (
            SizeRule::Arbitrary { multiple: 64, min_pixels: 262_144, max_pixels: 1_048_576, max_edge: 1536, max_ratio: 2.0 },
            "Named in the project config and not in the reviewed table: price and limits are unknown, and it must be served by hf-inference.",
        ),
    };
    ModelInfo {
        id: id.to_string(),
        status: ModelStatus::Current,
        recommended,
        max_refs: 0,
        mask: false,
        alpha: AlphaSupport::None,
        sizes,
        cost: unknown_cost(),
        async_job: false,
        supports_edit: false,
        edit_model: None,
        notes: notes.to_string(),
    }
}

impl CompatProvider {
    /// Build the adapter. `policy` decides which hosts it may reach.
    pub fn new(proto: Protocol, cfg: ProviderConfig, key: Option<String>, transport: Arc<dyn HttpTransport>, policy: HostPolicy) -> Self {
        let spec = proto.spec();
        let mut desc = descriptor_from(spec.json);
        if let Some(m) = cfg.model.as_deref() {
            let listed = desc.model(m).is_some();
            if !listed && matches!(proto, Protocol::Generic | Protocol::OpenRouter | Protocol::HuggingFace) {
                let recommended = proto == Protocol::Generic;
                desc.models.push(config_model(proto, m, recommended));
            }
        }
        let base = cfg.base_url.clone().or_else(|| spec.default_base.map(str::to_string)).unwrap_or_default().trim_end_matches('/').to_string();
        let key_var = cfg.api_key_env.clone().unwrap_or_else(|| spec.key_var.to_string());
        CompatProvider { proto, id: spec.id, key_var, desc, cfg, key: key.map(ApiKey::new), base, transport, policy }
    }

    fn secrets(&self) -> Vec<String> {
        self.key.iter().map(|k| k.expose().to_string()).collect()
    }

    /// A local OpenAI-compatible server may need no key at all.
    fn keyless_ok(&self) -> bool {
        self.proto == Protocol::Generic && parse_url(&self.base).map(|u| is_loopback_host(&u.host)).unwrap_or(false)
    }

    fn ratio_for(&self, model: &ModelInfo, req: &ImageRequest) -> String {
        match &model.sizes {
            SizeRule::AspectTiers { ratios, .. } => nearest_ratio(ratios, [req.width, req.height]).unwrap_or_else(|| "1:1".into()),
            _ => "1:1".into(),
        }
    }

    fn tier(model: &ModelInfo) -> String {
        match &model.sizes {
            SizeRule::AspectTiers { tier, .. } => tier.clone(),
            _ => "1K".into(),
        }
    }

    /// The request for a call. Public so contract tests can snapshot the wire format.
    pub fn build(&self, req: &ImageRequest) -> Result<Built, ProviderError> {
        let model = check_model(&self.desc, req)?;
        let (w, h) = self.prepare(req)?;
        let mut notes = Vec::new();
        let mut inputs: Vec<String> = Vec::new();
        if let Some(e) = &req.edit {
            inputs.push(data_url("image/png", &e.base_png));
        }
        for r in &req.refs {
            inputs.push(data_url("image/png", &r.png));
        }
        let negative = req.negative.join(", ");
        let mut drop_note = |what: &str| notes.push(format!("{what} was not sent: this API documents no such field"));
        let (url, body) = match self.proto {
            Protocol::Generic => {
                let mut body = json!({"model": req.model, "prompt": req.prompt, "n": 1, "size": format!("{w}x{h}"), "response_format": "b64_json"});
                if let Some(q) = req.quality.clone().or_else(|| self.cfg.quality.clone()) {
                    body["quality"] = json!(q);
                }
                if !negative.is_empty() {
                    drop_note("the negative prompt");
                }
                if req.seed.is_some() {
                    drop_note("the seed");
                }
                (format!("{}/images/generations", self.base), body)
            }
            Protocol::Xai => {
                let mut body = json!({
                    "model": req.model,
                    "prompt": req.prompt,
                    "n": 1,
                    "aspect_ratio": self.ratio_for(model, req),
                    "resolution": Self::tier(model),
                    "response_format": "b64_json",
                });
                if !negative.is_empty() {
                    drop_note("the negative prompt");
                }
                if req.seed.is_some() {
                    drop_note("the seed");
                }
                if inputs.is_empty() {
                    (format!("{}/images/generations", self.base), body)
                } else {
                    let obj = |u: &String| json!({"url": u, "type": "image_url"});
                    if inputs.len() == 1 {
                        body["image"] = obj(&inputs[0]);
                    } else {
                        body["images"] = Value::Array(inputs.iter().map(obj).collect());
                    }
                    (format!("{}/images/edits", self.base), body)
                }
            }
            Protocol::Together => {
                let mut body = json!({"model": req.model, "prompt": req.prompt, "n": 1, "response_format": "base64", "output_format": "png"});
                if matches!(model.sizes, SizeRule::AspectTiers { .. }) {
                    // Kontext models take aspect_ratio instead of width and height, and one image_url
                    body["aspect_ratio"] = json!(self.ratio_for(model, req));
                    if let Some(first) = inputs.first() {
                        body["image_url"] = json!(first);
                    }
                } else {
                    body["width"] = json!(w);
                    body["height"] = json!(h);
                    if !inputs.is_empty() {
                        body["reference_images"] = json!(inputs);
                    }
                }
                if !negative.is_empty() {
                    body["negative_prompt"] = json!(negative);
                }
                if let Some(s) = req.seed {
                    body["seed"] = json!(s);
                }
                (format!("{}/images/generations", self.base), body)
            }
            Protocol::OpenRouter => {
                let mut body = json!({
                    "model": req.model,
                    "prompt": req.prompt,
                    "n": 1,
                    "aspect_ratio": self.ratio_for(model, req),
                    "resolution": Self::tier(model),
                    "output_format": "png",
                });
                if req.transparent && model.alpha != AlphaSupport::None {
                    body["background"] = json!("transparent");
                }
                if let Some(q) = req.quality.clone().or_else(|| self.cfg.quality.clone()) {
                    body["quality"] = json!(q);
                }
                if let Some(s) = req.seed {
                    body["seed"] = json!(s);
                }
                if !negative.is_empty() {
                    drop_note("the negative prompt");
                }
                if !inputs.is_empty() {
                    body["input_references"] = Value::Array(inputs.iter().map(|u| json!({"type": "image_url", "image_url": {"url": u}})).collect());
                }
                (format!("{}/images", self.base), body)
            }
            Protocol::HuggingFace => {
                if !valid_hf_model(&req.model) {
                    return Err(ProviderError::unsupported(format!("'{}' is not a Hugging Face model id (owner/name)", req.model)));
                }
                let mut params = serde_json::Map::new();
                params.insert("width".into(), json!(w));
                params.insert("height".into(), json!(h));
                if !negative.is_empty() {
                    params.insert("negative_prompt".into(), json!(negative));
                }
                if let Some(s) = req.seed {
                    params.insert("seed".into(), json!(s));
                }
                (format!("{}/hf-inference/models/{}", self.base, req.model), json!({"inputs": req.prompt, "parameters": Value::Object(params)}))
            }
        };
        Ok(Built { url, body, notes })
    }

    /// Price of one call from the table, or `None` when no price is documented.
    fn table_price(&self, model: &ModelInfo, w: u32, h: u32) -> Option<f64> {
        match &model.cost {
            CostModel::PerImage { .. } => per_image_price(&model.cost, Some("default")).map(|p| p.0),
            CostModel::PerMegapixel { first, additional } => {
                let mp = megapixels(w, h).ceil().max(1.0);
                Some(first + additional * (mp - 1.0))
            }
            _ => None,
        }
    }

    fn call(&self, req: &ImageRequest) -> Result<ProviderOutput, ProviderError> {
        let secrets = self.secrets();
        let key = match &self.key {
            Some(k) => Some(k),
            None if self.keyless_ok() => None,
            None => return Err(ProviderError::not_configured(format!("{} is not set", self.key_var))),
        };
        let built = self.build(req)?;
        let mut http = HttpRequest::post_json(built.url.clone(), &built.body);
        if let Some(k) = key {
            http = http.header("Authorization", &format!("Bearer {}", k.expose()));
        }
        let resp = send(&*self.transport, &self.policy, &http)?;
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
        let mut notes = built.notes;
        let (image, mime, usage, echo, request_id): (Vec<u8>, String, Option<Value>, Option<String>, Option<String>);
        if self.proto == Protocol::HuggingFace {
            // The documented response is the raw image bytes.
            let declared = resp.header("content-type").map(|c| c.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
            let sniffed = sniff_mime(&resp.body);
            if sniffed == "application/octet-stream" && !declared.as_deref().map(|d| d.starts_with("image/")).unwrap_or(false) {
                let why = match serde_json::from_slice::<Value>(&resp.body) {
                    Ok(v) => {
                        format!("the response is JSON, not image bytes: {}", redact(&v.to_string().chars().take(200).collect::<String>(), &secrets))
                    }
                    Err(_) => "the response is not an image".to_string(),
                };
                return Err(malformed(&why));
            }
            image = resp.body.clone();
            mime = if sniffed != "application/octet-stream" { sniffed.to_string() } else { declared.unwrap_or_else(|| sniffed.to_string()) };
            usage = None;
            echo = Some(req.model.clone());
            request_id = resp.header("x-request-id").map(str::to_string);
        } else {
            let v: Value = serde_json::from_slice(&resp.body).map_err(|e| malformed(&format!("the response is not valid JSON ({e})")))?;
            let item = v.pointer("/data/0").ok_or_else(|| malformed("the response has no data[0]"))?;
            let declared = item.get("mime_type").or_else(|| item.get("media_type")).and_then(Value::as_str).map(str::to_string);
            let bytes = if let Some(b64) = item.get("b64_json").and_then(Value::as_str) {
                match parse_data_url(b64) {
                    Some((_, bytes)) => bytes,
                    None => b64_decode(b64)?,
                }
            } else if let Some(url) = item.get("url").and_then(Value::as_str) {
                // Public media URL: it must pass the same host policy, and the key is not sent to it.
                let dl = send(&*self.transport, &self.policy, &HttpRequest::get(url))?;
                if dl.status != 200 {
                    return Err(status_error(dl.status, &dl.body, &secrets));
                }
                dl.body
            } else {
                return Err(malformed("data[0] has neither b64_json nor url"));
            };
            let sniffed = sniff_mime(&bytes);
            mime = if sniffed != "application/octet-stream" { sniffed.to_string() } else { declared.unwrap_or_else(|| sniffed.to_string()) };
            image = bytes;
            usage = v.get("usage").cloned();
            echo = v.get("model").and_then(Value::as_str).map(str::to_string).or_else(|| Some(req.model.clone()));
            request_id = resp.header("x-request-id").map(str::to_string).or_else(|| v.get("id").and_then(Value::as_str).map(str::to_string));
            if let Some(rp) = item.get("revised_prompt").and_then(Value::as_str) {
                notes.push(format!("revised prompt: {}", redact(rp, &secrets)));
            }
        }
        let (actual_usd, usage_unknown) = self.actual_cost(req, usage.as_ref());
        Ok(ProviderOutput { image, mime, provider_request_id: request_id, usage, actual_usd, usage_unknown, model_echo: echo, notes, job: None })
    }

    /// Actual cost: the figure the provider reports when it reports one (xAI ticks, OpenRouter `usage.cost`),
    /// otherwise the table price. Never zero when unknown.
    fn actual_cost(&self, req: &ImageRequest, usage: Option<&Value>) -> (Option<f64>, bool) {
        match self.proto {
            Protocol::Xai => {
                if let Some(t) = usage.and_then(|u| u.get("cost_in_usd_ticks")).and_then(Value::as_f64) {
                    return (Some(t / 1e10), false);
                }
            }
            Protocol::OpenRouter => {
                if let Some(c) = usage.and_then(|u| u.get("cost")).and_then(Value::as_f64) {
                    return (Some(c), false);
                }
            }
            _ => {}
        }
        // Inputs may be billed separately where the docs say so, so edits are not claimed exact.
        let has_inputs = req.edit.is_some() || !req.refs.is_empty();
        let Some(model) = self.desc.model(&req.model) else { return (None, true) };
        let Ok((w, h)) = self.prepare(req) else { return (None, true) };
        match self.table_price(model, w, h) {
            Some(p) if !(has_inputs && matches!(self.proto, Protocol::Xai | Protocol::Together)) => (Some(p), false),
            _ => (None, true),
        }
    }
}

impl ImageProvider for CompatProvider {
    fn id(&self) -> &str {
        self.id
    }

    fn descriptor(&self) -> &ProviderDescriptor {
        &self.desc
    }

    fn is_configured(&self) -> bool {
        self.key.is_some() || self.keyless_ok()
    }

    fn default_model(&self, _editing: bool) -> String {
        self.cfg.model.clone().or_else(|| self.desc.recommended().map(|m| m.id.clone())).unwrap_or_default()
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        let model = check_model(&self.desc, req)?;
        match &model.sizes {
            SizeRule::AspectTiers { .. } => Ok(tier_size(&self.ratio_for(model, req))),
            rule => choose_size(rule, [req.width, req.height]),
        }
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        let model = check_model(&self.desc, req)?;
        let (w, h) = self.prepare(req)?;
        let name = &self.desc.display_name;
        let when = &self.desc.retrieved;
        let inputs_billed = (req.edit.is_some() || !req.refs.is_empty()) && matches!(self.proto, Protocol::Xai | Protocol::Together);
        match self.table_price(model, w, h) {
            Some(usd) => Ok(CostEstimate {
                usd: (usd * 1e6).round() / 1e6,
                basis: format!(
                    "{name} price table retrieved {when}: {} output only{}",
                    model.id,
                    if inputs_billed { ". Input images are billed too and the docs give no price for them" } else { "" }
                ),
                usage_unknown: inputs_billed,
                funding: Funding::UserKey,
            }),
            None => Ok(CostEstimate {
                usd: 0.0,
                basis: format!(
                    "{name}: price unknown, no price is documented for {} (tables retrieved {when}). The figure 0.0 is a placeholder, not a price: check the account",
                    model.id
                ),
                usage_unknown: true,
                funding: Funding::UserKey,
            }),
        }
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
    // Every fixture below is DOCUMENTED SHAPE, NOT RECORDED: built from the vendor documentation's schema
    // (see doc_urls in data/*.json), never captured from a provider.

    use super::*;
    use crate::common::b64;
    use crate::transport::{FixtureTransport, Method};
    use crate::types::{EditSpec, ProviderRef};
    use imagegen_core::config::MapEnv;
    use imagegen_core::pixels::{encode_png, Rgba};
    use imagegen_core::refs::RefRole;

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [10, 200, 30, 255])).unwrap()
    }

    fn make(proto: Protocol, fixture: &Arc<FixtureTransport>, key: Option<&str>, cfg: ProviderConfig) -> CompatProvider {
        let spec = proto.spec();
        let policy =
            policy_for(spec.hosts, cfg.base_url.as_deref(), &MapEnv::from_pairs(&[("HOI4_IMAGEGEN_ALLOWED_HOSTS", "my.server.example")])).unwrap();
        CompatProvider::new(proto, cfg, key.map(str::to_string), fixture.clone(), policy)
    }

    fn generic_cfg() -> ProviderConfig {
        ProviderConfig { base_url: Some("https://my.server.example/v1".into()), model: Some("sdxl".into()), ..Default::default() }
    }

    fn req(model: &str) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "an iron wall".into(),
            negative: vec![],
            width: 100,
            height: 88,
            transparent: true,
            quality: None,
            refs: vec![],
            edit: None,
            idempotency_key: Some("k1".into()),
            seed: None,
        }
    }

    fn with_edit(mut r: ImageRequest) -> ImageRequest {
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        r
    }

    fn with_ref(mut r: ImageRequest) -> ImageRequest {
        r.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: vec![1, 2, 3], sha256: "x".into() }];
        r
    }

    fn b64_body(extra: Value) -> Vec<u8> {
        let mut v = json!({"data": [{"b64_json": b64(&png())}]});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        serde_json::to_vec(&v).unwrap()
    }

    const GEN: &str = "/generations";

    #[test]
    fn documented_shape_generic_body_and_response() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, GEN, 200, &[], b64_body(json!({})));
        let p = make(Protocol::Generic, &fixture, Some("sk-test-000000"), generic_cfg());
        assert_eq!(p.default_model(false), "sdxl");
        let b = p.build(&req("sdxl")).unwrap();
        assert_eq!(b.url, "https://my.server.example/v1/images/generations");
        assert_eq!(b.body["response_format"], "b64_json");
        assert_eq!(b.body["n"], 1);
        assert_eq!(b.body["size"], "1024x1024");
        assert!(b.body.get("background").is_none(), "alpha is proven from pixels, never requested from an unknown server");
        let out = p.generate(&req("sdxl"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert!(out.usage_unknown && out.actual_usd.is_none());
        let sent = &fixture.requests()[0];
        assert!(sent.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer sk-test-000000"));
        // edits are refused before any request
        assert!(p.edit(&with_edit(req("sdxl")), &CallContext::new()).is_err());
        assert_eq!(fixture.requests().len(), 1);
    }

    #[test]
    fn documented_shape_generic_url_response_downloads_without_the_key() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, GEN, 200, &[], br#"{"data":[{"url":"https://my.server.example/files/a.png"}]}"#.to_vec());
        fixture.respond(Method::Get, "/files/a.png", 200, &[("content-type", "image/png")], png());
        let p = make(Protocol::Generic, &fixture, Some("sk-test-000000"), generic_cfg());
        let out = p.generate(&req("sdxl"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        let dl = fixture.requests().into_iter().find(|r| r.method == Method::Get).unwrap();
        assert!(!dl.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization")));
        // a download host outside the policy is blocked
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, GEN, 200, &[], br#"{"data":[{"url":"https://evil.example/a.png"}]}"#.to_vec());
        let p = make(Protocol::Generic, &fixture, Some("sk-test-000000"), generic_cfg());
        assert_eq!(p.generate(&req("sdxl"), &CallContext::new()).unwrap_err().kind, ErrorKind::Blocked);
    }

    #[test]
    fn documented_shape_unknown_prices_say_unknown() {
        let fixture = Arc::new(FixtureTransport::new());
        let p = make(Protocol::Generic, &fixture, None, generic_cfg());
        let e = p.estimate(&req("sdxl")).unwrap();
        assert!(e.usage_unknown && e.usd == 0.0 && e.basis.contains("price unknown"), "{}", e.basis);
        let p = make(Protocol::OpenRouter, &fixture, None, ProviderConfig::default());
        let e = p.estimate(&req("openai/gpt-image-2")).unwrap();
        assert!(e.usage_unknown && e.basis.contains("price unknown"));
        let e = p.estimate(&req("bytedance-seed/seedream-4.5")).unwrap();
        assert!(!e.usage_unknown && (e.usd - 0.05).abs() < 1e-9);
    }

    #[test]
    fn documented_shape_generic_keyless_loopback_and_missing_key() {
        let fixture = Arc::new(FixtureTransport::new());
        let local = ProviderConfig { base_url: Some("http://127.0.0.1:8080/v1".into()), model: Some("sdxl".into()), ..Default::default() };
        let p = make(Protocol::Generic, &fixture, None, local);
        assert!(p.is_configured(), "a loopback server needs no key");
        let p = make(Protocol::Generic, &fixture, None, generic_cfg());
        assert!(!p.is_configured());
        let err = p.generate(&req("sdxl"), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotConfigured);
        assert!(err.message.contains("OPENAI_COMPATIBLE_API_KEY"));
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn documented_shape_xai_generation_edit_and_usage_ticks() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(
            Method::Post,
            GEN,
            200,
            &[],
            b64_body(json!({"data": [{"b64_json": b64(&png()), "mime_type": "image/png"}], "usage": {"cost_in_usd_ticks": 400_000_000}})),
        );
        let p = make(Protocol::Xai, &fixture, Some("xai-test-000000"), ProviderConfig::default());
        assert_eq!(p.default_model(false), "grok-imagine-image-2.0");
        let mut r = req("grok-imagine-image-2.0");
        r.negative = vec!["blur".into()];
        r.seed = Some(7);
        let b = p.build(&r).unwrap();
        assert_eq!(b.url, "https://api.x.ai/v1/images/generations");
        assert_eq!(b.body["aspect_ratio"], "1:1");
        assert_eq!(b.body["resolution"], "1k");
        assert_eq!(b.body["response_format"], "b64_json");
        assert_eq!(b.notes.len(), 2, "negative prompt and seed have no xAI field: {:?}", b.notes);
        let out = p.generate(&req("grok-imagine-image-2.0"), &CallContext::new()).unwrap();
        assert!((out.actual_usd.unwrap() - 0.04).abs() < 1e-12, "ticks 400_000_000 / 1e10 = $0.04");
        assert!(!out.usage_unknown);
        // one input image uses `image`, several use `images`, both on the edits route
        let b = p.build(&with_edit(req("grok-imagine-image-2.0"))).unwrap();
        assert_eq!(b.url, "https://api.x.ai/v1/images/edits");
        assert_eq!(b.body["image"]["type"], "image_url");
        assert!(b.body["image"]["url"].as_str().unwrap().starts_with("data:image/png;base64,"));
        let b = p.build(&with_ref(with_edit(req("grok-imagine-image-2.0")))).unwrap();
        assert_eq!(b.body["images"].as_array().unwrap().len(), 2);
        assert!(b.body.get("image").is_none());
        // edits bill the input image, so the estimate is not exact
        assert!(p.estimate(&with_edit(req("grok-imagine-image-2.0"))).unwrap().usage_unknown);
        assert!(!p.estimate(&req("grok-imagine-image-2.0")).unwrap().usage_unknown);
        // six inputs exceed the documented limit of five
        let mut many = req("grok-imagine-image-2.0");
        many.refs = (0..6).map(|_| ProviderRef { role: RefRole::StyleAnchor, png: vec![1], sha256: "x".into() }).collect();
        assert!(p.estimate(&many).is_err());
    }

    #[test]
    fn documented_shape_together_width_height_and_kontext_aspect_ratio() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(
            Method::Post,
            GEN,
            200,
            &[],
            serde_json::to_vec(&json!({"id": "oFuwv7Y-2kFHot-99170ebf9e84e0ce-SJC", "model": "black-forest-labs/FLUX.2-dev", "object": "list",
                "data": [{"index": 0, "type": "b64_json", "b64_json": b64(&png())}]}))
            .unwrap(),
        );
        let p = make(Protocol::Together, &fixture, Some("tg-test-000000"), ProviderConfig::default());
        assert_eq!(p.default_model(false), "black-forest-labs/FLUX.2-dev");
        let mut r = req("black-forest-labs/FLUX.2-dev");
        r.negative = vec!["blur".into(), "text".into()];
        r.seed = Some(3);
        let b = p.build(&r).unwrap();
        assert_eq!(b.url, "https://api.together.ai/v1/images/generations");
        assert_eq!(b.body["response_format"], "base64");
        assert_eq!(b.body["negative_prompt"], "blur, text");
        assert_eq!(b.body["seed"], 3);
        let (w, h) = (b.body["width"].as_u64().unwrap(), b.body["height"].as_u64().unwrap());
        assert!(w % 16 == 0 && h % 16 == 0 && w * h >= 262_144);
        assert!(b.body.get("aspect_ratio").is_none() && b.body.get("reference_images").is_none());
        let b = p.build(&with_ref(with_edit(req("black-forest-labs/FLUX.2-pro")))).unwrap();
        assert_eq!(b.body["reference_images"].as_array().unwrap().len(), 2, "the base image comes first, then the references");
        let b = p.build(&with_edit(req("black-forest-labs/FLUX.1-kontext-pro"))).unwrap();
        assert_eq!(b.body["aspect_ratio"], "1:1");
        assert!(b.body["image_url"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert!(b.body.get("width").is_none());
        // FLUX.1.1 pro takes no references, and kontext takes only one image
        assert!(p.build(&with_ref(req("black-forest-labs/FLUX.1.1-pro"))).is_err());
        assert!(p.build(&with_ref(with_edit(req("black-forest-labs/FLUX.1-kontext-pro")))).is_err());
        // price per megapixel and per image
        let e = p.estimate(&req("black-forest-labs/FLUX.2-max")).unwrap();
        assert!((e.usd - 0.07).abs() < 1e-9, "{}", e.usd);
        let e = p.estimate(&req("black-forest-labs/FLUX.2-dev")).unwrap();
        assert!((e.usd - 0.0154).abs() < 1e-9);
        let out = p.generate(&req("black-forest-labs/FLUX.2-dev"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.actual_usd, Some(0.0154));
        assert_eq!(out.provider_request_id.as_deref(), Some("oFuwv7Y-2kFHot-99170ebf9e84e0ce-SJC"));
    }

    #[test]
    fn documented_shape_openrouter_images_endpoint_and_usage_cost() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(
            Method::Post,
            "/api/v1/images",
            200,
            &[],
            serde_json::to_vec(&json!({"created": 1748372400, "data": [{"b64_json": b64(&png()), "media_type": "image/png"}],
                "usage": {"prompt_tokens": 0, "completion_tokens": 4175, "total_tokens": 4175, "cost": 0.04}}))
            .unwrap(),
        );
        let p = make(Protocol::OpenRouter, &fixture, Some("or-test-000000"), ProviderConfig::default());
        assert_eq!(p.default_model(false), "bytedance-seed/seedream-4.5");
        let b = p.build(&with_ref(req("openai/gpt-image-2"))).unwrap();
        assert_eq!(b.url, "https://openrouter.ai/api/v1/images");
        assert_eq!(b.body["aspect_ratio"], "5:4");
        assert_eq!(b.body["resolution"], "1K");
        assert_eq!(b.body["background"], "transparent");
        assert_eq!(b.body["input_references"][0]["type"], "image_url");
        assert!(b.body["input_references"][0]["image_url"]["url"].as_str().unwrap().starts_with("data:image/png;base64,"));
        let b = p.build(&req("bytedance-seed/seedream-4.5")).unwrap();
        assert!(b.body.get("background").is_none() && b.body.get("input_references").is_none());
        let out = p.generate(&req("bytedance-seed/seedream-4.5"), &CallContext::new()).unwrap();
        assert_eq!(out.actual_usd, Some(0.04), "usage.cost is the provider's own figure and wins over the table");
        assert!(!out.usage_unknown);
        // a model named in the config but not in the table is allowed, with no price
        let cfg = ProviderConfig { model: Some("vendor/new-image".into()), ..Default::default() };
        let p = make(Protocol::OpenRouter, &fixture, Some("or-test-000000"), cfg);
        assert_eq!(p.default_model(false), "vendor/new-image");
        assert!(p.estimate(&req("vendor/new-image")).unwrap().usage_unknown);
        assert!(p.build(&with_ref(req("vendor/new-image"))).is_err(), "edit support of an unlisted model is unknown");
    }

    #[test]
    fn documented_shape_huggingface_raw_bytes_route() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/hf-inference/models/", 200, &[("content-type", "image/png")], png());
        let p = make(Protocol::HuggingFace, &fixture, Some("hf_test_000000"), ProviderConfig::default());
        assert_eq!(p.default_model(false), "stabilityai/stable-diffusion-3-medium-diffusers");
        let mut r = req("stabilityai/stable-diffusion-3-medium-diffusers");
        r.negative = vec!["blur".into()];
        r.seed = Some(5);
        let b = p.build(&r).unwrap();
        assert_eq!(b.url, "https://router.huggingface.co/hf-inference/models/stabilityai/stable-diffusion-3-medium-diffusers");
        assert_eq!(b.body["inputs"], "an iron wall");
        assert_eq!(b.body["parameters"]["negative_prompt"], "blur");
        assert_eq!(b.body["parameters"]["seed"], 5);
        let (w, h) = (b.body["parameters"]["width"].as_u64().unwrap(), b.body["parameters"]["height"].as_u64().unwrap());
        assert!(w % 64 == 0 && h % 64 == 0 && w * h >= 262_144, "{w}x{h}");
        assert!(b.body.get("model").is_none(), "the model is in the path, not the body");
        let out = p.generate(&req("stabilityai/stable-diffusion-3-medium-diffusers"), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert!(out.usage_unknown && out.actual_usd.is_none());
        let sent = &fixture.requests()[0];
        assert!(sent.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer hf_test_000000"));
        // no references and no edits on this route
        assert!(p.build(&with_ref(req("stabilityai/stable-diffusion-3-medium-diffusers"))).is_err());
        assert!(p.edit(&with_edit(req("stabilityai/stable-diffusion-3-medium-diffusers")), &CallContext::new()).is_err());
    }

    /// Recorded-real replay: one real call to the Hugging Face router on 2026-11-02 (the maintainer's free account,
    /// no payment method, so it could not be charged) with the request this adapter builds. The router accepted
    /// the token and the route and answered 402 for missing credits. The response is replayed byte for byte from
    /// `tests/fixtures/huggingface/`. The success path is not live-verified.
    #[test]
    fn recorded_real_huggingface_route_and_token_accepted_out_of_credits() {
        let meta: Value = serde_json::from_str(include_str!("../tests/fixtures/huggingface/recorded.json")).unwrap();
        assert_eq!(meta["recorded"], "real");
        let body = include_bytes!("../tests/fixtures/huggingface/response.body").to_vec();
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Post, "/hf-inference/models/", 402, &[("content-type", "application/json; charset=utf-8")], body);
        let p = make(Protocol::HuggingFace, &fixture, Some("hf_test_000000"), ProviderConfig::default());
        let model = meta["request"]["url"].as_str().unwrap().rsplit("/models/").next().unwrap().to_string();
        let mut r = req(&model);
        r.prompt = meta["request"]["body"]["inputs"].as_str().unwrap().to_string();
        r.width = 512;
        r.height = 512;
        // the adapter builds the request that was sent
        let b = p.build(&r).unwrap();
        assert_eq!(b.url, meta["request"]["url"].as_str().unwrap());
        assert_eq!(b.body, meta["request"]["body"]);
        // and turns the real answer into a clear, non-retried out-of-credits error
        let err = p.generate(&r, &CallContext::new()).unwrap_err();
        assert_eq!((err.kind, err.retry), (ErrorKind::RateLimited, RetryClass::RequiresAuthorization));
        assert!(err.message.contains("HTTP 402") && err.message.contains("credits"), "{}", err.message);
    }

    #[test]
    fn documented_shape_huggingface_json_or_text_answer_is_not_an_image_and_ids_are_checked() {
        for body in [br#"{"error":"Model is loading"}"#.to_vec(), b"plain text".to_vec()] {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, "/hf-inference/models/", 200, &[("content-type", "application/json")], body);
            let p = make(Protocol::HuggingFace, &fixture, Some("hf_test_000000"), ProviderConfig::default());
            let err = p.generate(&req("stabilityai/stable-diffusion-3-medium-diffusers"), &CallContext::new()).unwrap_err();
            assert_eq!((err.kind, err.retry), (ErrorKind::MalformedResponse, RetryClass::ReconcileFirst));
        }
        assert!(valid_hf_model("owner/name-1.0") && valid_hf_model("gpt2"));
        for bad in ["", "a/b/c", "../x", "a/..", "a b/c", "a/b?x=1", "a/b#c", "/a"] {
            assert!(!valid_hf_model(bad), "{bad}");
        }
        // a config-named model with a path trick never reaches the URL
        let cfg = ProviderConfig { model: Some("a/b?x=1".into()), ..Default::default() };
        let fixture = Arc::new(FixtureTransport::new());
        let p = make(Protocol::HuggingFace, &fixture, Some("hf_test_000000"), cfg);
        assert!(p.generate(&req("a/b?x=1"), &CallContext::new()).is_err());
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn documented_shape_errors_map_and_never_leak_the_key() {
        let key = "secret-key-123456";
        let protos = [
            (Protocol::Generic, "sdxl", generic_cfg()),
            (Protocol::Xai, "grok-imagine-image-2.0", ProviderConfig::default()),
            (Protocol::Together, "black-forest-labs/FLUX.2-dev", ProviderConfig::default()),
            (Protocol::OpenRouter, "bytedance-seed/seedream-4.5", ProviderConfig::default()),
            (Protocol::HuggingFace, "stabilityai/stable-diffusion-3-medium-diffusers", ProviderConfig::default()),
        ];
        for (proto, model, cfg) in protos {
            let cases: [(u16, String, ErrorKind, RetryClass); 5] = [
                (401, format!(r#"{{"error":{{"message":"bad key {key}"}}}}"#), ErrorKind::Auth, RetryClass::RequiresAuthorization),
                (429, r#"{"error":{"message":"rate limit"}}"#.into(), ErrorKind::RateLimited, RetryClass::SafeSameInputs),
                (400, r#"{"error":{"code":"moderation_blocked","message":"blocked"}}"#.into(), ErrorKind::ModerationBlocked, RetryClass::Terminal),
                (400, r#"{"error":{"message":"bad size"}}"#.into(), ErrorKind::BadRequest, RetryClass::Terminal),
                (500, "boom".into(), ErrorKind::ServerError, RetryClass::ReconcileFirst),
            ];
            for (status, body, kind, retry) in cases {
                let fixture = Arc::new(FixtureTransport::new());
                fixture.respond(Method::Post, "https://", status, &[], body);
                let p = make(proto, &fixture, Some(key), cfg.clone());
                let err = p.generate(&req(model), &CallContext::new()).unwrap_err();
                assert_eq!((err.kind, err.retry), (kind, retry), "{proto:?} status {status}");
                assert!(!err.message.contains(key), "{proto:?} leaked the key: {}", err.message);
                assert_eq!(err.status, Some(status));
            }
        }
    }

    #[test]
    fn documented_shape_malformed_success_responses_demand_reconciliation() {
        for body in [b"not json".to_vec(), br#"{"data":[]}"#.to_vec(), br#"{"data":[{"b64_json":"%%%"}]}"#.to_vec(), br#"{"data":[{}]}"#.to_vec()] {
            let fixture = Arc::new(FixtureTransport::new());
            fixture.respond(Method::Post, GEN, 200, &[], body);
            let p = make(Protocol::Together, &fixture, Some("tg-test-000000"), ProviderConfig::default());
            let err = p.generate(&req("black-forest-labs/FLUX.2-dev"), &CallContext::new()).unwrap_err();
            assert_eq!(err.retry, RetryClass::ReconcileFirst, "{err}");
        }
    }

    #[test]
    fn documented_shape_missing_key_unknown_models_and_hosts_fail_before_any_request() {
        let fixture = Arc::new(FixtureTransport::new());
        for (proto, var, model) in [
            (Protocol::Xai, "XAI_API_KEY", "grok-imagine-image-2.0"),
            (Protocol::Together, "TOGETHER_API_KEY", "black-forest-labs/FLUX.2-dev"),
            (Protocol::OpenRouter, "OPENROUTER_API_KEY", "bytedance-seed/seedream-4.5"),
            (Protocol::HuggingFace, "HF_TOKEN", "stabilityai/stable-diffusion-3-medium-diffusers"),
        ] {
            let p = make(proto, &fixture, None, ProviderConfig::default());
            assert!(!p.is_configured());
            let err = p.generate(&req(model), &CallContext::new()).unwrap_err();
            assert_eq!(err.kind, ErrorKind::NotConfigured);
            assert!(err.message.contains(var), "{}", err.message);
            let p = make(proto, &fixture, Some("key-000000"), ProviderConfig::default());
            assert_eq!(p.generate(&req("no/such-model"), &CallContext::new()).unwrap_err().kind, ErrorKind::Unsupported, "{proto:?}");
        }
        assert!(fixture.requests().is_empty());
        // a base URL outside the policy is blocked at send time, so the key never leaves
        let cfg = ProviderConfig { base_url: Some("https://evil.example/v1".into()), ..Default::default() };
        let p = CompatProvider::new(Protocol::Xai, cfg, Some("key-000000".into()), fixture.clone(), HostPolicy::api(&["api.x.ai"]));
        assert_eq!(p.generate(&req("grok-imagine-image-2.0"), &CallContext::new()).unwrap_err().kind, ErrorKind::Blocked);
        assert!(fixture.requests().is_empty());
    }

    fn registry(cfg: &str, env: &[(&str, &str)]) -> ProviderRegistry {
        let mut reg = ProviderRegistry::new();
        let config = Config::parse(cfg).unwrap();
        register(&mut reg, &config, &MapEnv::from_pairs(env), Arc::new(FixtureTransport::new()));
        reg
    }

    #[test]
    fn documented_shape_register_offers_presets_even_without_keys_and_never_foundry() {
        let reg = registry("", &[]);
        assert_eq!(reg.ids(), vec!["huggingface", "openrouter", "together", "xai"]);
        assert!(reg.configured_ids().is_empty(), "no keys, so none configured, but they are offered so doctor can name the variable");
        assert!(reg.get("foundry").is_none() && reg.problems().is_empty());
        let reg = registry("", &[("XAI_API_KEY", "x-1"), ("HF_TOKEN", "hf_1")]);
        assert_eq!(reg.configured_ids(), vec!["huggingface", "xai"]);
        let reg = registry("[providers.xai]\nenabled = false\n[providers.together]\nenabled = false\n", &[]);
        assert_eq!(reg.ids(), vec!["huggingface", "openrouter"]);
        let reg = registry("[providers.xai]\napi_key_env = \"MY_XAI\"\n", &[("XAI_API_KEY", "ignored"), ("MY_XAI", "mine")]);
        assert!(reg.get("xai").unwrap().is_configured());
    }

    #[test]
    fn documented_shape_register_generic_needs_base_url_and_model_and_obeys_host_rules() {
        let reg = registry("[providers.openai_compatible]\nmodel = \"m\"\n", &[]);
        assert!(reg.get("openai_compatible").is_none());
        assert!(reg.problems()["openai_compatible"].contains("base_url"));
        let reg = registry("[providers.openai_compatible]\nbase_url = \"https://srv.example/v1\"\n", &[]);
        assert!(reg.problems()["openai_compatible"].contains("model"));
        // a new host needs the user's own acknowledgement
        let cfg = "[providers.openai_compatible]\nbase_url = \"https://srv.example/v1\"\nmodel = \"m\"\n";
        let reg = registry(cfg, &[]);
        assert!(reg.get("openai_compatible").is_none() && reg.problems()["openai_compatible"].contains("HOI4_IMAGEGEN_ALLOWED_HOSTS"));
        let reg = registry(cfg, &[("HOI4_IMAGEGEN_ALLOWED_HOSTS", "srv.example"), ("OPENAI_COMPATIBLE_API_KEY", "k-000000")]);
        let p = reg.get("openai_compatible").unwrap();
        assert!(p.is_configured());
        assert_eq!(p.default_model(false), "m");
        // loopback is allowed without acknowledgement
        let reg = registry("[providers.openai_compatible]\nbase_url = \"http://127.0.0.1:9/v1\"\nmodel = \"m\"\n", &[]);
        assert!(reg.get("openai_compatible").is_some());
        // a project file cannot move a preset's key to a new host
        let reg = registry("[providers.together]\nbase_url = \"https://evil.example\"\n", &[("TOGETHER_API_KEY", "k-000000")]);
        assert!(reg.get("together").is_none() && reg.problems().contains_key("together"));
    }

    #[test]
    fn documented_shape_tables_parse_say_not_live_verified_and_have_one_recommended_model() {
        for proto in ALL {
            let spec = proto.spec();
            let raw: Value = serde_json::from_str(spec.json).unwrap();
            assert_eq!(raw["live_verified"], json!(false), "{}", spec.id);
            assert!(raw["doc_urls"].as_array().map(|a| !a.is_empty()).unwrap_or(false), "{}", spec.id);
            assert!(raw["checked"].as_str().is_some(), "{}", spec.id);
            assert!(raw["terms_note"].as_str().unwrap().contains("live-verified"), "{}", spec.id);
            let d = descriptor_from(spec.json);
            assert_eq!(d.id, spec.id);
            assert_eq!(d.env_vars, vec![spec.key_var.to_string()]);
            if proto != Protocol::Generic {
                assert_eq!(d.models.iter().filter(|m| m.recommended).count(), 1, "{}", spec.id);
                assert_eq!(d.hosts, spec.hosts.iter().map(|h| h.to_string()).collect::<Vec<_>>(), "{}", spec.id);
                assert!(!d.models.is_empty());
            } else {
                assert!(d.models.is_empty(), "the generic provider has no table, its model comes from the config");
            }
            // every price in a table is positive and sizes can be chosen for a HOI4 canvas
            for m in &d.models {
                if let CostModel::PerImage { usd_by_quality } = &m.cost {
                    assert!(usd_by_quality.iter().all(|(_, p)| *p > 0.0), "{} {}", spec.id, m.id);
                }
                let size = match &m.sizes {
                    SizeRule::AspectTiers { ratios, .. } => tier_size(&nearest_ratio(ratios, [100, 88]).unwrap()),
                    rule => choose_size(rule, [100, 88]).unwrap(),
                };
                assert!(size.0 > 0 && size.1 > 0);
            }
        }
    }
}
