//! Helpers shared by the provider adapters.

use std::fmt;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use imagegen_core::error::RetryClass;

use crate::transport::error_message;
use crate::types::{CostModel, ErrorKind, ImageRequest, ModelInfo, ModelStatus, ProviderDescriptor, ProviderError, TABLES_RETRIEVED};

/// A secret that never prints.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wrap a key.
    pub fn new(value: String) -> Self {
        ApiKey(value)
    }

    /// The key text, for building headers only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey([redacted])")
    }
}

/// Parse an embedded descriptor table. The tables are compiled in and covered by tests, so a failure here is a
/// build-time mistake, not a runtime condition.
pub fn descriptor_from(json: &str) -> ProviderDescriptor {
    serde_json::from_str(json).expect("embedded provider table is valid")
}

/// Standard base64.
pub fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// Decode standard base64, tolerating whitespace.
pub fn b64_decode(text: &str) -> Result<Vec<u8>, ProviderError> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD
        .decode(cleaned.as_bytes())
        .map_err(|e| ProviderError::new(ErrorKind::BadImage, RetryClass::ReconcileFirst, format!("the image data is not valid base64: {e}")))
}

/// `data:` URL.
pub fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", b64(bytes))
}

/// Decode a `data:` URL into its MIME type and bytes.
pub fn parse_data_url(url: &str) -> Option<(String, Vec<u8>)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    Some((mime.to_string(), b64_decode(data).ok()?))
}

/// MIME type from magic bytes.
pub fn sniff_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

/// Megapixels of a size.
pub fn megapixels(w: u32, h: u32) -> f64 {
    f64::from(w) * f64::from(h) / 1_048_576.0
}

/// Turn an HTTP error response into a provider error with an honest retry class.
pub fn status_error(status: u16, body: &[u8], secrets: &[String]) -> ProviderError {
    let msg = error_message(body, secrets);
    let lower = String::from_utf8_lossy(body).to_ascii_lowercase();
    let moderated = lower.contains("moderation_blocked")
        || lower.contains("content_policy")
        || lower.contains("safety_system")
        || lower.contains("blockreason")
        || lower.contains("blocked_by_safety");
    let (kind, retry) = match status {
        401 | 403 => (ErrorKind::Auth, RetryClass::RequiresAuthorization),
        // out of credits or payment required: the user has to top up, so never retry on our own
        402 => (ErrorKind::RateLimited, RetryClass::RequiresAuthorization),
        429 => (ErrorKind::RateLimited, RetryClass::SafeSameInputs),
        400 | 422 if moderated => (ErrorKind::ModerationBlocked, RetryClass::Terminal),
        400 | 404 | 405 | 409 | 413 | 415 | 422 => (ErrorKind::BadRequest, RetryClass::Terminal),
        500..=599 => (ErrorKind::ServerError, RetryClass::ReconcileFirst),
        300..=399 => (ErrorKind::BadRequest, RetryClass::Terminal),
        _ => (ErrorKind::BadRequest, RetryClass::Terminal),
    };
    let msg = if (300..400).contains(&status) { "the provider answered with a redirect, which is not followed".to_string() } else { msg };
    ProviderError::new(kind, retry, format!("HTTP {status}: {msg}")).with_status(status)
}

/// Check a request against a descriptor's model table before anything is sent.
pub fn check_model<'a>(desc: &'a ProviderDescriptor, req: &ImageRequest) -> Result<&'a ModelInfo, ProviderError> {
    let known =
        || desc.models.iter().filter(|m| !matches!(m.status, ModelStatus::Retired { .. })).map(|m| m.id.as_str()).collect::<Vec<_>>().join(", ");
    let model = desc.model(&req.model).ok_or_else(|| {
        ProviderError::unsupported(format!(
            "'{}' is not in the {} model table (checked {}). Known models: {}",
            req.model,
            desc.display_name,
            desc.retrieved,
            known()
        ))
    })?;
    if let ModelStatus::Retired { on } = &model.status {
        let hint = desc.recommended().map(|m| format!(" Use {} instead.", m.id)).unwrap_or_default();
        return Err(ProviderError::new(ErrorKind::ModelRetired, RetryClass::Terminal, format!("{} was shut down on {on}.{hint}", model.id)));
    }
    let editing = req.edit.is_some();
    if editing && !model.supports_edit {
        return Err(ProviderError::unsupported(format!("{} cannot edit images or take references", model.id)));
    }
    if !editing && !req.refs.is_empty() && !model.supports_edit {
        return Err(ProviderError::unsupported(format!("{} does not accept reference images. Choose a model that supports editing", model.id)));
    }
    let images = req.refs.len() + usize::from(editing);
    if images > usize::from(model.max_refs) {
        return Err(ProviderError::unsupported(format!(
            "{} accepts at most {} input images in one request, this one has {images}",
            model.id, model.max_refs
        )));
    }
    if req.edit.as_ref().map(|e| e.mask_png.is_some()).unwrap_or(false) && !model.mask {
        return Err(ProviderError::unsupported(format!("{} does not accept an edit mask", model.id)));
    }
    Ok(model)
}

/// Look up a per-image price for a quality tier. The tier `default` is used for single-price models.
pub fn per_image_price(cost: &CostModel, quality: Option<&str>) -> Option<(f64, String)> {
    let table = match cost {
        CostModel::PerImage { usd_by_quality } => usd_by_quality,
        CostModel::UnpublishedProxy { proxy_usd_by_quality, .. } => proxy_usd_by_quality,
        _ => return None,
    };
    let want = quality.unwrap_or("default");
    table.iter().find(|(q, _)| q == want).or_else(|| table.iter().find(|(q, _)| q == "default")).map(|(q, p)| (*p, q.clone()))
}

/// Basis text with the table date.
pub fn basis(provider: &str, what: &str) -> String {
    format!("{provider} price table retrieved {TABLES_RETRIEVED}: {what}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EditSpec;

    #[test]
    fn api_key_never_prints() {
        let k = ApiKey::new("sk-secret-value".into());
        assert!(!format!("{k:?}").contains("secret"));
        assert_eq!(k.expose(), "sk-secret-value");
    }

    #[test]
    fn data_urls_round_trip() {
        let url = data_url("image/png", b"hello");
        assert_eq!(parse_data_url(&url), Some(("image/png".to_string(), b"hello".to_vec())));
        assert!(parse_data_url("https://x").is_none());
        assert!(b64_decode("!!!").is_err());
        assert_eq!(b64_decode("aGVs\nbG8=").unwrap(), b"hello");
    }

    #[test]
    fn status_mapping() {
        let s: Vec<String> = vec![];
        assert_eq!(status_error(401, b"{}", &s).kind, ErrorKind::Auth);
        assert_eq!(status_error(429, b"{}", &s).retry, RetryClass::SafeSameInputs);
        assert_eq!(
            (status_error(402, b"{}", &s).kind, status_error(402, b"{}", &s).retry),
            (ErrorKind::RateLimited, RetryClass::RequiresAuthorization)
        );
        let m = status_error(400, br#"{"error":{"code":"moderation_blocked","message":"blocked"}}"#, &s);
        assert_eq!((m.kind, m.retry), (ErrorKind::ModerationBlocked, RetryClass::Terminal));
        assert_eq!(status_error(400, b"{}", &s).kind, ErrorKind::BadRequest);
        let e = status_error(503, b"oops", &s);
        assert_eq!((e.kind, e.retry), (ErrorKind::ServerError, RetryClass::ReconcileFirst));
        assert_eq!(status_error(302, b"", &s).kind, ErrorKind::BadRequest);
    }

    fn req(model: &str) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "p".into(),
            negative: vec![],
            width: 1024,
            height: 1024,
            transparent: false,
            quality: None,
            refs: vec![],
            edit: None,
            idempotency_key: None,
            seed: None,
        }
    }

    #[test]
    fn retired_and_unknown_models_are_refused_before_sending() {
        let d = descriptor_from(include_str!("../data/openai.json"));
        let e = check_model(&d, &req("gpt-image-1")).unwrap_err();
        assert_eq!(e.kind, ErrorKind::ModelRetired);
        assert!(e.message.contains("2026-10-23") && e.message.contains("gpt-image-2.5-flare"), "{}", e.message);
        let e = check_model(&d, &req("dall-e-3")).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unsupported);
        assert!(!e.message.contains("gpt-image-1,"), "retired models are not suggested: {}", e.message);
        assert!(check_model(&d, &req("gpt-image-2.5-flare")).is_ok());
    }

    #[test]
    fn capability_limits_are_enforced() {
        let d = descriptor_from(include_str!("../data/gemini.json"));
        let mut r = req("gemini-3-pro-image");
        r.edit = Some(EditSpec { base_png: vec![1], mask_png: Some(vec![2]) });
        assert!(check_model(&d, &r).unwrap_err().message.contains("mask"));
        r.edit = Some(EditSpec { base_png: vec![1], mask_png: None });
        assert!(check_model(&d, &r).is_ok());
        let fal = descriptor_from(include_str!("../data/fal.json"));
        let mut r = req("fal-ai/flux/schnell");
        r.edit = Some(EditSpec { base_png: vec![1], mask_png: None });
        assert!(check_model(&fal, &r).is_err());
        let mut r = req("fal-ai/flux-2");
        r.refs = (0..5)
            .map(|_| crate::types::ProviderRef { role: imagegen_core::refs::RefRole::StyleAnchor, png: vec![], sha256: String::new() })
            .collect();
        assert!(check_model(&fal, &r).unwrap_err().message.contains("at most 4"));
    }

    #[test]
    fn price_lookup_falls_back_to_default() {
        let c = CostModel::PerImage { usd_by_quality: vec![("default".into(), 0.05)] };
        assert_eq!(per_image_price(&c, Some("high")).unwrap().0, 0.05);
        let c = CostModel::PerImage { usd_by_quality: vec![("low".into(), 0.01), ("high".into(), 0.2)] };
        assert_eq!(per_image_price(&c, Some("high")).unwrap().0, 0.2);
        assert!(per_image_price(&c, None).is_none());
    }

    #[test]
    fn embedded_tables_parse_and_have_one_recommended_model() {
        for json in [include_str!("../data/openai.json"), include_str!("../data/gemini.json"), include_str!("../data/fal.json")] {
            let d = descriptor_from(json);
            assert_eq!(d.retrieved, TABLES_RETRIEVED, "{}", d.id);
            assert_eq!(d.models.iter().filter(|m| m.recommended).count(), 1, "{}", d.id);
            assert!(!d.hosts.is_empty() && !d.env_vars.is_empty());
        }
    }
}
