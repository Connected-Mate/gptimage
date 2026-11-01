//! ComfyUI adapter: a plain HTTP client to an endpoint the maintainer configured.
//!
//! Decision D3 allows agents to call a ComfyUI endpoint the maintainer set up (a local GPU or a RunPod URL) for
//! any asset. This adapter only talks to that URL. It never starts, configures or stops pods or machines.
//!
//! A workflow is a project file in ComfyUI's API format plus a small slot map saying which node input receives
//! the prompt, size, seed and uploaded images. The adapter checks the server has the nodes the workflow needs
//! (`GET /object_info/<node>`) before it uploads anything, because ComfyUI workflows are brittle across node
//! versions.
//!
//! Routes used: `POST /upload/image`, `POST /prompt`, `GET /history/{id}`, `GET /view`, `GET /object_info/{node}`,
//! `POST /interrupt`, `POST /queue`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use imagegen_core::config::ProviderConfig;
use imagegen_core::error::RetryClass;
use imagegen_core::hashing::sha256_hex;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::common::{sniff_mime, status_error, ApiKey};
use crate::sizing::choose_size;
use crate::transport::{is_loopback_host, parse_url, send, HostPolicy, HttpRequest, HttpTransport, Method};
use crate::types::{
    AlphaSupport, CallContext, CostEstimate, CostModel, ErrorKind, Funding, ImageProvider, ImageRequest, JobRef, ModelInfo, ModelStatus,
    ProviderDescriptor, ProviderError, ProviderOutput, Reconciled, SizeRule, TABLES_RETRIEVED,
};

/// Where an input value goes in the workflow graph.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Slot {
    /// Node id in the graph.
    pub node: String,
    /// Input name on that node.
    pub input: String,
}

/// What the workflow can do.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Capabilities {
    /// The workflow produces real alpha (for example a matte node).
    pub alpha: bool,
    /// The workflow has an `edit_image` slot.
    pub edit: bool,
    /// Number of `ref_N` slots.
    pub max_refs: u8,
}

/// A workflow template file.
#[derive(Debug, Clone, Deserialize)]
pub struct ComfyTemplate {
    /// Workflow name; the model id is `workflow:<name>`.
    pub name: String,
    /// Template format version.
    #[serde(default = "one")]
    pub version: u32,
    /// Node classes the server must have.
    #[serde(default)]
    pub required_nodes: Vec<String>,
    /// Slots: `prompt`, `negative`, `width`, `height`, `seed`, `ref_0`.., `edit_image`.
    pub slots: BTreeMap<String, Slot>,
    /// Node whose output images are the result.
    pub output_node: String,
    /// Capabilities.
    #[serde(default)]
    pub capabilities: Capabilities,
    /// The graph in ComfyUI API format.
    pub graph: Value,
}

fn one() -> u32 {
    1
}

impl ComfyTemplate {
    /// Parse and check a template.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let t: ComfyTemplate = serde_json::from_str(text).map_err(|e| format!("workflow template is not valid: {e}"))?;
        t.check()?;
        Ok(t)
    }

    fn check(&self) -> Result<(), String> {
        if self.name.is_empty() || !self.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err("workflow name must be letters, digits, '_' or '-'".into());
        }
        let graph = self.graph.as_object().ok_or("graph must be an object in ComfyUI API format")?;
        if !graph.contains_key(&self.output_node) {
            return Err(format!("output_node '{}' is not in the graph", self.output_node));
        }
        for required in ["prompt"] {
            if !self.slots.contains_key(required) {
                return Err(format!("the template needs a '{required}' slot"));
            }
        }
        for (name, slot) in &self.slots {
            let node = graph.get(&slot.node).ok_or_else(|| format!("slot '{name}' points at node '{}', which is not in the graph", slot.node))?;
            if node.pointer(&format!("/inputs/{}", slot.input)).is_none() {
                return Err(format!("slot '{name}': node '{}' has no input '{}'", slot.node, slot.input));
            }
        }
        for i in 0..self.capabilities.max_refs {
            if !self.slots.contains_key(&format!("ref_{i}")) {
                return Err(format!("capabilities.max_refs is {} but slot 'ref_{i}' is missing", self.capabilities.max_refs));
            }
        }
        if self.capabilities.edit && !self.slots.contains_key("edit_image") {
            return Err("capabilities.edit is set but slot 'edit_image' is missing".into());
        }
        Ok(())
    }

    /// Model id.
    pub fn model_id(&self) -> String {
        format!("workflow:{}", self.name)
    }
}

/// Adapter state.
pub struct ComfyProvider {
    desc: ProviderDescriptor,
    template: ComfyTemplate,
    endpoint: String,
    key: Option<ApiKey>,
    transport: Arc<dyn HttpTransport>,
    policy: HostPolicy,
    poll_interval: Duration,
    max_polls: u32,
}

impl ComfyProvider {
    /// Build the adapter. `endpoint` is the server's base URL, for example `http://127.0.0.1:8188`.
    pub fn new(cfg: &ProviderConfig, template: ComfyTemplate, key: Option<String>, transport: Arc<dyn HttpTransport>) -> Result<Self, String> {
        let endpoint = cfg.endpoint.clone().ok_or("providers.comfyui.endpoint is not set")?;
        let parsed = parse_url(&endpoint)?;
        let endpoint = format!("{}://{}{}", parsed.scheme, authority(&parsed), parsed.path_and_query.trim_end_matches('/'));
        let model = ModelInfo {
            id: template.model_id(),
            status: ModelStatus::Current,
            recommended: true,
            max_refs: template.capabilities.max_refs.saturating_add(u8::from(template.capabilities.edit)),
            mask: false,
            alpha: if template.capabilities.alpha { AlphaSupport::Ga } else { AlphaSupport::None },
            sizes: SizeRule::Arbitrary { multiple: 8, min_pixels: 589_824, max_pixels: 2_359_296, max_edge: 2048, max_ratio: 4.0 },
            cost: CostModel::Free,
            async_job: true,
            supports_edit: template.capabilities.edit || template.capabilities.max_refs > 0,
            edit_model: None,
            notes: "Cost is the user's own compute; RunPod time is billed by RunPod and is not estimated here.".into(),
        };
        let desc = ProviderDescriptor {
            id: "comfyui".into(),
            display_name: "ComfyUI endpoint".into(),
            doc_url: "https://docs.comfy.org/development/comfyui-server/comms_routes".into(),
            retrieved: TABLES_RETRIEVED.into(),
            env_vars: vec!["COMFYUI_API_KEY".into()],
            hosts: vec![],
            models: vec![model],
            terms_note: "Output rights follow the models in the workflow (FLUX.2 Klein 9B weights are non-commercial). This adapter never starts or stops pods.".into(),
        };
        Ok(ComfyProvider {
            desc,
            template,
            endpoint,
            key: key.map(ApiKey::new),
            transport,
            policy: HostPolicy::user_endpoint(),
            poll_interval: Duration::from_secs(2),
            max_polls: 600,
        })
    }

    /// Override polling (tests use a tiny budget).
    pub fn with_polling(mut self, interval: Duration, max_polls: u32) -> Self {
        self.poll_interval = interval;
        self.max_polls = max_polls;
        self
    }

    fn secrets(&self) -> Vec<String> {
        self.key.iter().map(|k| k.expose().to_string()).collect()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.endpoint)
    }

    /// The key travels only over https or to a loopback host, never in cleartext over a network.
    fn with_key(&self, req: HttpRequest) -> HttpRequest {
        let safe = parse_url(&req.url).map(|u| u.scheme == "https" || is_loopback_host(&u.host)).unwrap_or(false);
        match (&self.key, safe) {
            (Some(k), true) => req.header("X-API-Key", k.expose()),
            _ => req,
        }
    }

    fn get_json(&self, path: &str) -> Result<Value, ProviderError> {
        let resp = send(&*self.transport, &self.policy, &self.with_key(HttpRequest::get(self.url(path))))?;
        if resp.status != 200 {
            return Err(status_error(resp.status, &resp.body, &self.secrets()));
        }
        serde_json::from_slice(&resp.body)
            .map_err(|e| ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, format!("{path} did not return JSON: {e}")))
    }

    fn check_nodes(&self) -> Result<(), ProviderError> {
        let mut missing = Vec::new();
        for node in &self.template.required_nodes {
            let v = self.get_json(&format!("/object_info/{}", pct(node)))?;
            if v.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                missing.push(node.clone());
            }
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ProviderError::unsupported(format!(
                "the ComfyUI server at {} lacks node(s) {} that workflow '{}' requires. Install the custom nodes or choose another workflow",
                self.endpoint,
                missing.join(", "),
                self.template.name
            )))
        }
    }

    fn upload(&self, png: &[u8]) -> Result<String, ProviderError> {
        let name = format!("hoi4_{}.png", &sha256_hex(png)[..16]);
        let (boundary, body) = multipart(&[
            Part::File { name: "image", filename: &name, content_type: "image/png", bytes: png },
            Part::Text { name: "type", value: "input" },
            Part::Text { name: "overwrite", value: "true" },
        ]);
        let mut req = self.with_key(HttpRequest { method: Method::Post, url: self.url("/upload/image"), headers: vec![], body });
        req.headers.push(("Content-Type".into(), format!("multipart/form-data; boundary={boundary}")));
        let resp = send(&*self.transport, &self.policy, &req)?;
        if resp.status != 200 {
            return Err(status_error(resp.status, &resp.body, &self.secrets()));
        }
        let v: Value = serde_json::from_slice(&resp.body)
            .map_err(|e| ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, format!("upload response is not JSON: {e}")))?;
        let stored = v
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, "upload response has no name"))?;
        Ok(match v.get("subfolder").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => format!("{s}/{stored}"),
            _ => stored.to_string(),
        })
    }

    fn set_slot(&self, graph: &mut Value, slot: &str, value: Value) -> Result<(), ProviderError> {
        let s = self
            .template
            .slots
            .get(slot)
            .ok_or_else(|| ProviderError::unsupported(format!("workflow '{}' has no '{slot}' slot", self.template.name)))?;
        graph[&s.node]["inputs"][&s.input] = value;
        Ok(())
    }

    /// The graph with every slot filled, public for tests.
    pub fn fill_graph(&self, req: &ImageRequest, uploads: &[String], edit_upload: Option<&str>, size: (u32, u32)) -> Result<Value, ProviderError> {
        let mut graph = self.template.graph.clone();
        self.set_slot(&mut graph, "prompt", json!(req.prompt))?;
        if self.template.slots.contains_key("negative") {
            self.set_slot(&mut graph, "negative", json!(req.negative.join(", ")))?;
        }
        for (slot, v) in [("width", size.0), ("height", size.1)] {
            if self.template.slots.contains_key(slot) {
                self.set_slot(&mut graph, slot, json!(v))?;
            }
        }
        if self.template.slots.contains_key("seed") {
            let seed = req.seed.unwrap_or_else(|| {
                let h = sha256_hex(req.idempotency_key.as_deref().unwrap_or(&req.prompt).as_bytes());
                u64::from_str_radix(&h[..12], 16).unwrap_or(0)
            });
            self.set_slot(&mut graph, "seed", json!(seed))?;
        }
        for (i, name) in uploads.iter().enumerate() {
            self.set_slot(&mut graph, &format!("ref_{i}"), json!(name))?;
        }
        if let Some(name) = edit_upload {
            self.set_slot(&mut graph, "edit_image", json!(name))?;
        }
        Ok(graph)
    }

    fn validate(&self, req: &ImageRequest) -> Result<(), ProviderError> {
        if req.model != self.template.model_id() {
            return Err(ProviderError::unsupported(format!("this endpoint serves '{}', not '{}'", self.template.model_id(), req.model)));
        }
        if req.refs.len() > usize::from(self.template.capabilities.max_refs) {
            return Err(ProviderError::unsupported(format!(
                "workflow '{}' takes {} reference image(s), this request has {}. Extra references are never dropped silently",
                self.template.name,
                self.template.capabilities.max_refs,
                req.refs.len()
            )));
        }
        if req.edit.is_some() && !self.template.capabilities.edit {
            return Err(ProviderError::unsupported(format!("workflow '{}' has no edit_image slot", self.template.name)));
        }
        if req.edit.as_ref().map(|e| e.mask_png.is_some()).unwrap_or(false) {
            return Err(ProviderError::unsupported("masks are not supported by the ComfyUI adapter"));
        }
        Ok(())
    }

    fn job_for(&self, id: String) -> JobRef {
        JobRef {
            provider: "comfyui".into(),
            id: id.clone(),
            status_url: Some(self.url(&format!("/history/{}", pct(&id)))),
            response_url: Some(self.url(&format!("/history/{}", pct(&id)))),
            cancel_url: Some(self.url("/interrupt")),
            model: self.template.model_id(),
        }
    }

    /// Look in `/history/{id}`. `None` while the job is still pending.
    fn history(&self, id: &str) -> Result<Option<Value>, ProviderError> {
        let v = self.get_json(&format!("/history/{}", pct(id)))?;
        Ok(v.get(id).cloned())
    }

    fn collect(&self, job: &JobRef, entry: &Value) -> Result<ProviderOutput, ProviderError> {
        if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
            let detail = entry.pointer("/status/messages").map(|m| m.to_string()).unwrap_or_default();
            return Err(ProviderError::new(
                ErrorKind::ServerError,
                RetryClass::Terminal,
                format!("the workflow failed: {}", crate::transport::redact(&detail.chars().take(400).collect::<String>(), &self.secrets())),
            )
            .with_job(job.clone()));
        }
        let image = entry.pointer(&format!("/outputs/{}/images/0", self.template.output_node)).ok_or_else(|| {
            ProviderError::new(
                ErrorKind::MalformedResponse,
                RetryClass::SafeSameInputs,
                format!("the job finished but node '{}' produced no image", self.template.output_node),
            )
            .with_job(job.clone())
        })?;
        let filename = image.get("filename").and_then(Value::as_str).unwrap_or("");
        let subfolder = image.get("subfolder").and_then(Value::as_str).unwrap_or("");
        let kind = image.get("type").and_then(Value::as_str).unwrap_or("output");
        if filename.is_empty() {
            return Err(ProviderError::new(ErrorKind::MalformedResponse, RetryClass::SafeSameInputs, "the output image has no filename")
                .with_job(job.clone()));
        }
        let path = format!("/view?filename={}&subfolder={}&type={}", pct(filename), pct(subfolder), pct(kind));
        let resp = send(&*self.transport, &self.policy, &self.with_key(HttpRequest::get(self.url(&path)))).map_err(|e| e.with_job(job.clone()))?;
        if resp.status != 200 {
            return Err(status_error(resp.status, &resp.body, &self.secrets()).with_job(job.clone()));
        }
        Ok(ProviderOutput {
            mime: sniff_mime(&resp.body).to_string(),
            image: resp.body,
            provider_request_id: Some(job.id.clone()),
            usage: None,
            actual_usd: Some(0.0),
            usage_unknown: false,
            model_echo: Some(self.template.model_id()),
            notes: vec![],
            job: Some(job.clone()),
        })
    }

    fn run(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        self.validate(req)?;
        let size = self.prepare(req)?;
        self.check_nodes()?;
        let mut uploads = Vec::new();
        for r in &req.refs {
            uploads.push(self.upload(&r.png)?);
        }
        let edit_upload = match &req.edit {
            Some(e) => Some(self.upload(&e.base_png)?),
            None => None,
        };
        let graph = self.fill_graph(req, &uploads, edit_upload.as_deref(), size)?;
        let client_id = format!("hoi4-imagegen-{}", &sha256_hex(req.idempotency_key.as_deref().unwrap_or(&req.prompt).as_bytes())[..16]);
        let body = json!({"prompt": graph, "client_id": client_id});
        let resp = send(&*self.transport, &self.policy, &self.with_key(HttpRequest::post_json(self.url("/prompt"), &body)))?;
        if resp.status != 200 {
            return Err(status_error(resp.status, &resp.body, &self.secrets()));
        }
        let v: Value = serde_json::from_slice(&resp.body).map_err(|e| {
            ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, format!("/prompt did not return JSON: {e}; a job may exist"))
        })?;
        if let Some(errs) = v.get("node_errors").and_then(Value::as_object) {
            if !errs.is_empty() {
                return Err(ProviderError::new(
                    ErrorKind::BadRequest,
                    RetryClass::Terminal,
                    format!("ComfyUI rejected the workflow: {}", Value::Object(errs.clone()).to_string().chars().take(500).collect::<String>()),
                ));
            }
        }
        let id = v
            .get("prompt_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::new(ErrorKind::MalformedResponse, RetryClass::ReconcileFirst, "/prompt returned no prompt_id; a job may exist")
            })?
            .to_string();
        let job = self.job_for(id.clone());
        ctx.notify(&job);
        for _ in 0..self.max_polls {
            if ctx.cancel.is_cancelled() {
                let _ = self.cancel(&job, ctx);
                return Err(ProviderError::new(ErrorKind::Cancelled, RetryClass::Terminal, "cancelled").with_job(job));
            }
            // /history only lists a prompt once it has finished (or failed), so presence means done
            if let Some(entry) = self.history(&id).map_err(|e| e.with_job(job.clone()))? {
                return self.collect(&job, &entry);
            }
            ctx.sleeper.sleep(self.poll_interval);
        }
        Err(ProviderError::new(
            ErrorKind::Transport,
            RetryClass::ReconcileFirst,
            "the workflow is still running after the polling budget; reconcile it by id",
        )
        .with_job(job))
    }
}

fn authority(u: &crate::transport::ParsedUrl) -> String {
    let host = if u.host.contains(':') { format!("[{}]", u.host) } else { u.host.clone() };
    match u.port {
        Some(p) => format!("{host}:{p}"),
        None => host,
    }
}

/// Percent-encode everything except unreserved characters.
pub fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A multipart part.
pub enum Part<'a> {
    /// A text field.
    Text {
        /// Field name.
        name: &'a str,
        /// Value.
        value: &'a str,
    },
    /// A file field.
    File {
        /// Field name.
        name: &'a str,
        /// File name.
        filename: &'a str,
        /// Content type.
        content_type: &'a str,
        /// Bytes.
        bytes: &'a [u8],
    },
}

/// Build a multipart/form-data body. The boundary derives from the content hash so it cannot appear in it.
pub fn multipart(parts: &[Part<'_>]) -> (String, Vec<u8>) {
    let mut material = Vec::new();
    for p in parts {
        match p {
            Part::Text { name, value } => material.extend_from_slice(format!("{name}={value};").as_bytes()),
            Part::File { bytes, .. } => material.extend_from_slice(bytes),
        }
    }
    let boundary = format!("hoi4imagegen{}", &sha256_hex(&material)[..24]);
    let mut body = Vec::new();
    for p in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match p {
            Part::Text { name, value } => {
                body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
            }
            Part::File { name, filename, content_type, bytes } => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n")
                        .as_bytes(),
                );
                body.extend_from_slice(bytes);
                body.extend_from_slice(b"\r\n");
            }
        }
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

impl ImageProvider for ComfyProvider {
    fn id(&self) -> &str {
        "comfyui"
    }

    fn descriptor(&self) -> &ProviderDescriptor {
        &self.desc
    }

    fn is_configured(&self) -> bool {
        true
    }

    fn default_model(&self, _editing: bool) -> String {
        self.template.model_id()
    }

    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError> {
        self.validate(req)?;
        choose_size(&self.desc.models[0].sizes, [req.width, req.height])
    }

    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError> {
        self.validate(req)?;
        Ok(CostEstimate {
            usd: 0.0,
            basis: "no provider fee: local or self-hosted compute. RunPod GPU time is billed by RunPod and is not estimated here".into(),
            usage_unknown: false,
            funding: Funding::Local,
        })
    }

    fn generate(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.edit.is_some() {
            return Err(ProviderError::unsupported("generate was called with an edit request"));
        }
        self.run(req, ctx)
    }

    fn edit(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError> {
        if req.edit.is_none() {
            return Err(ProviderError::unsupported("edit was called without a base image"));
        }
        self.run(req, ctx)
    }

    fn reconcile(&self, job: &JobRef, _ctx: &CallContext) -> Result<Reconciled, ProviderError> {
        match self.history(&job.id)? {
            None => Ok(Reconciled::Pending),
            Some(entry) => {
                if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
                    return Ok(Reconciled::Failed("the workflow reported an error".into()));
                }
                Ok(Reconciled::Completed(Box::new(self.collect(job, &entry)?)))
            }
        }
    }

    fn cancel(&self, job: &JobRef, _ctx: &CallContext) -> Result<(), ProviderError> {
        let interrupt = self.with_key(HttpRequest { method: Method::Post, url: self.url("/interrupt"), headers: vec![], body: vec![] });
        send(&*self.transport, &self.policy, &interrupt)?;
        let delete = self.with_key(HttpRequest::post_json(self.url("/queue"), &json!({"delete": [job.id]})));
        send(&*self.transport, &self.policy, &delete)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FixtureTransport;
    use crate::types::{CancelToken, EditSpec, ProviderRef, Sleeper};
    use imagegen_core::pixels::{encode_png, Rgba};
    use imagegen_core::refs::RefRole;

    // SYNTHETIC fixtures: the ComfyUI replies are written from the documented routes, not recorded from a server.

    fn png() -> Vec<u8> {
        encode_png(&Rgba::filled(4, 4, [9, 9, 9, 255])).unwrap()
    }

    fn template_json(extra_slots: &str, caps: &str) -> String {
        format!(
            r#"{{
              "name": "t2i", "version": 1,
              "required_nodes": ["KSampler", "SaveImage"],
              "slots": {{
                "prompt": {{"node": "6", "input": "text"}},
                "negative": {{"node": "7", "input": "text"}},
                "width": {{"node": "5", "input": "width"}},
                "height": {{"node": "5", "input": "height"}},
                "seed": {{"node": "3", "input": "seed"}}{extra_slots}
              }},
              "output_node": "9",
              "capabilities": {caps},
              "graph": {{
                "3": {{"class_type": "KSampler", "inputs": {{"seed": 0, "steps": 24}}}},
                "5": {{"class_type": "EmptyLatentImage", "inputs": {{"width": 512, "height": 512}}}},
                "6": {{"class_type": "CLIPTextEncode", "inputs": {{"text": ""}}}},
                "7": {{"class_type": "CLIPTextEncode", "inputs": {{"text": ""}}}},
                "9": {{"class_type": "SaveImage", "inputs": {{"filename_prefix": "hoi4"}}}},
                "10": {{"class_type": "LoadImage", "inputs": {{"image": "x.png"}}}},
                "11": {{"class_type": "LoadImage", "inputs": {{"image": "y.png"}}}}
              }}
            }}"#
        )
    }

    fn plain_template() -> ComfyTemplate {
        ComfyTemplate::from_json(&template_json("", "{}")).unwrap()
    }

    fn ref_template() -> ComfyTemplate {
        ComfyTemplate::from_json(&template_json(
            r#", "ref_0": {"node": "10", "input": "image"}, "edit_image": {"node": "11", "input": "image"}"#,
            r#"{"max_refs": 1, "edit": true}"#,
        ))
        .unwrap()
    }

    fn provider(t: ComfyTemplate, fixture: Arc<FixtureTransport>) -> ComfyProvider {
        let cfg = ProviderConfig { endpoint: Some("http://127.0.0.1:8188".into()), ..Default::default() };
        ComfyProvider::new(&cfg, t, None, fixture).unwrap().with_polling(Duration::from_millis(1), 4)
    }

    fn request() -> ImageRequest {
        ImageRequest {
            model: "workflow:t2i".into(),
            prompt: "an iron wall".into(),
            negative: vec!["text".into(), "watermark".into()],
            width: 156,
            height: 210,
            transparent: false,
            quality: None,
            refs: vec![],
            edit: None,
            idempotency_key: Some("k-1".into()),
            seed: Some(42),
        }
    }

    fn server(fixture: &FixtureTransport) {
        fixture.respond(Method::Get, "/object_info/KSampler", 200, &[], r#"{"KSampler":{"input":{}}}"#);
        fixture.respond(Method::Get, "/object_info/SaveImage", 200, &[], r#"{"SaveImage":{"input":{}}}"#);
        fixture.respond(Method::Post, "/prompt", 200, &[], r#"{"prompt_id":"pid-synthetic-1","number":1,"node_errors":{}}"#);
        fixture.respond(Method::Get, "$/history/pid-synthetic-1", 200, &[], "{}");
        let done = json!({"pid-synthetic-1": {"status": {"status_str": "success", "completed": true},
            "outputs": {"9": {"images": [{"filename": "hoi4_00001_.png", "subfolder": "", "type": "output"}]}}}});
        fixture.respond(Method::Get, "$/history/pid-synthetic-1", 200, &[], serde_json::to_vec(&done).unwrap());
        fixture.respond(Method::Get, "/view?filename=hoi4_00001_.png", 200, &[("content-type", "image/png")], png());
        fixture.respond(Method::Post, "/upload/image", 200, &[], r#"{"name":"uploaded.png","subfolder":"","type":"input"}"#);
    }

    #[test]
    fn template_validation_catches_broken_files() {
        assert!(ComfyTemplate::from_json("not json").is_err());
        let bad_node = template_json("", "{}").replace(r#""output_node": "9""#, r#""output_node": "99""#);
        assert!(ComfyTemplate::from_json(&bad_node).unwrap_err().contains("output_node"));
        let bad_input = template_json("", "{}").replace(r#""input": "text"}"#, r#""input": "nonsense"}"#);
        assert!(ComfyTemplate::from_json(&bad_input).unwrap_err().contains("no input"));
        let missing_ref = template_json("", r#"{"max_refs": 1}"#);
        assert!(ComfyTemplate::from_json(&missing_ref).unwrap_err().contains("ref_0"));
        assert_eq!(plain_template().model_id(), "workflow:t2i");
    }

    #[test]
    fn graph_slots_are_filled_without_touching_other_inputs() {
        let p = provider(ref_template(), Arc::new(FixtureTransport::new()));
        let g = p.fill_graph(&request(), &["ref.png".into()], Some("base.png"), (640, 896)).unwrap();
        assert_eq!(g["6"]["inputs"]["text"], "an iron wall");
        assert_eq!(g["7"]["inputs"]["text"], "text, watermark");
        assert_eq!(g["5"]["inputs"]["width"], 640);
        assert_eq!(g["5"]["inputs"]["height"], 896);
        assert_eq!(g["3"]["inputs"]["seed"], 42);
        assert_eq!(g["3"]["inputs"]["steps"], 24, "inputs outside the slots stay as the template had them");
        assert_eq!(g["10"]["inputs"]["image"], "ref.png");
        assert_eq!(g["11"]["inputs"]["image"], "base.png");
    }

    #[test]
    fn full_flow_checks_nodes_submits_polls_and_downloads() {
        let fixture = Arc::new(FixtureTransport::new());
        server(&fixture);
        let p = provider(plain_template(), fixture.clone());
        let out = p.generate(&request(), &CallContext::new()).unwrap();
        assert_eq!(out.image, png());
        assert_eq!(out.mime, "image/png");
        assert_eq!(out.job.as_ref().unwrap().id, "pid-synthetic-1");
        assert_eq!(out.actual_usd, Some(0.0));
        let reqs = fixture.requests();
        let order: Vec<&str> = reqs.iter().map(|r| r.url.as_str()).collect();
        let first_prompt = order.iter().position(|u| u.ends_with("/prompt")).unwrap();
        assert!(order[..first_prompt].iter().all(|u| u.contains("/object_info/")), "nodes are checked before anything is submitted: {order:?}");
        let sent: Value = serde_json::from_slice(&reqs[first_prompt].body).unwrap();
        assert_eq!(sent["prompt"]["6"]["inputs"]["text"], "an iron wall");
        assert!(sent["client_id"].as_str().unwrap().starts_with("hoi4-imagegen-"));
        assert!(reqs.iter().all(|r| r.url.starts_with("http://127.0.0.1:8188/")));
    }

    #[test]
    fn references_are_uploaded_by_hash_name_and_extra_ones_are_refused() {
        let fixture = Arc::new(FixtureTransport::new());
        server(&fixture);
        let p = provider(ref_template(), fixture.clone());
        let mut r = request();
        r.refs = vec![ProviderRef { role: RefRole::StyleAnchor, png: png(), sha256: "x".into() }];
        p.generate(&r, &CallContext::new()).unwrap();
        let reqs = fixture.requests();
        let upload = reqs.iter().find(|q| q.url.ends_with("/upload/image")).unwrap();
        let body = String::from_utf8_lossy(&upload.body);
        assert!(body.contains(&format!("filename=\"hoi4_{}.png\"", &sha256_hex(&png())[..16])));
        assert!(body.contains("name=\"overwrite\"\r\n\r\ntrue"));
        let ct = upload.headers.iter().find(|(k, _)| k == "Content-Type").unwrap();
        assert!(ct.1.starts_with("multipart/form-data; boundary=hoi4imagegen"));
        let prompt_req = reqs.iter().find(|q| q.url.ends_with("/prompt")).unwrap();
        let sent: Value = serde_json::from_slice(&prompt_req.body).unwrap();
        assert_eq!(sent["prompt"]["10"]["inputs"]["image"], "uploaded.png");

        // a workflow with no reference slots refuses references instead of dropping them
        let fixture = Arc::new(FixtureTransport::new());
        server(&fixture);
        let p = provider(plain_template(), fixture.clone());
        let err = p.generate(&r, &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported);
        assert!(err.message.contains("never dropped silently"));
        assert!(fixture.requests().is_empty());
    }

    #[test]
    fn edits_upload_the_base_image() {
        let fixture = Arc::new(FixtureTransport::new());
        server(&fixture);
        let p = provider(ref_template(), fixture.clone());
        let mut r = request();
        r.edit = Some(EditSpec { base_png: png(), mask_png: None });
        p.edit(&r, &CallContext::new()).unwrap();
        let prompt_req = fixture.requests().into_iter().find(|q| q.url.ends_with("/prompt")).unwrap();
        let sent: Value = serde_json::from_slice(&prompt_req.body).unwrap();
        assert_eq!(sent["prompt"]["11"]["inputs"]["image"], "uploaded.png");
        // masks are refused
        r.edit = Some(EditSpec { base_png: png(), mask_png: Some(png()) });
        assert!(p.edit(&r, &CallContext::new()).unwrap_err().message.contains("masks"));
    }

    #[test]
    fn a_missing_custom_node_is_reported_before_uploading_anything() {
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Get, "/object_info/KSampler", 200, &[], r#"{"KSampler":{}}"#);
        fixture.respond(Method::Get, "/object_info/SaveImage", 200, &[], "{}");
        let p = provider(plain_template(), fixture.clone());
        let err = p.generate(&request(), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported);
        assert!(err.message.contains("SaveImage") && !err.message.contains("KSampler,"), "{}", err.message);
        assert!(!fixture.requests().iter().any(|r| r.url.ends_with("/prompt") || r.url.ends_with("/upload/image")));
    }

    #[test]
    fn server_side_validation_errors_and_workflow_failures_are_reported() {
        let fixture2 = Arc::new(FixtureTransport::new());
        fixture2.respond(Method::Get, "/object_info/KSampler", 200, &[], r#"{"KSampler":{}}"#);
        fixture2.respond(Method::Get, "/object_info/SaveImage", 200, &[], r#"{"SaveImage":{}}"#);
        fixture2.respond(
            Method::Post,
            "/prompt",
            200,
            &[],
            r#"{"prompt_id":"pid-synthetic-1","number":1,"node_errors":{"3":{"errors":[{"message":"bad seed"}]}}}"#,
        );
        let p = provider(plain_template(), fixture2);
        let err = p.generate(&request(), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::BadRequest);
        assert!(err.message.contains("bad seed"));

        let fixture3 = Arc::new(FixtureTransport::new());
        server(&fixture3);
        fixture3.clear_route(Method::Get, "$/history/pid-synthetic-1");
        fixture3.respond(Method::Get, "$/history/pid-synthetic-1", 200, &[], r#"{"pid-synthetic-1":{"status":{"status_str":"error","completed":false,"messages":[["execution_error",{"exception_message":"CUDA out of memory"}]]}}}"#);
        let p = provider(plain_template(), fixture3);
        let err = p.generate(&request(), &CallContext::new()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::ServerError);
        assert!(err.message.contains("CUDA") || err.message.contains("workflow failed"));
    }

    #[test]
    fn cancel_interrupts_and_removes_the_queued_job() {
        let fixture = Arc::new(FixtureTransport::new());
        server(&fixture);
        fixture.respond(Method::Post, "/interrupt", 200, &[], "");
        fixture.respond(Method::Post, "/queue", 200, &[], "");
        let p = provider(plain_template(), fixture.clone());
        let ctx = CallContext::new();
        ctx.cancel.cancel();
        let err = p.generate(&request(), &ctx).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Cancelled);
        let reqs = fixture.requests();
        assert!(reqs.iter().any(|r| r.url.ends_with("/interrupt")));
        let q = reqs.iter().find(|r| r.url.ends_with("/queue")).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&q.body).unwrap(), json!({"delete": ["pid-synthetic-1"]}));
    }

    #[test]
    fn polling_budget_and_reconcile() {
        #[derive(Default)]
        struct NoSleep;
        impl Sleeper for NoSleep {
            fn sleep(&self, _d: Duration) {}
        }
        let fixture = Arc::new(FixtureTransport::new());
        fixture.respond(Method::Get, "/object_info/KSampler", 200, &[], r#"{"KSampler":{}}"#);
        fixture.respond(Method::Get, "/object_info/SaveImage", 200, &[], r#"{"SaveImage":{}}"#);
        fixture.respond(Method::Post, "/prompt", 200, &[], r#"{"prompt_id":"pid-synthetic-1","node_errors":{}}"#);
        fixture.respond(Method::Get, "$/history/pid-synthetic-1", 200, &[], "{}");
        let p = provider(plain_template(), fixture.clone());
        let ctx = CallContext { cancel: CancelToken::new(), sleeper: Arc::new(NoSleep), jobs: None };
        let err = p.generate(&request(), &ctx).unwrap_err();
        assert_eq!(err.retry, RetryClass::ReconcileFirst);
        let job = err.job.unwrap();
        assert!(matches!(p.reconcile(&job, &ctx).unwrap(), Reconciled::Pending));
        // the server finishes later
        fixture.clear_route(Method::Get, "$/history/pid-synthetic-1");
        let done = json!({"pid-synthetic-1": {"status": {"status_str": "success", "completed": true},
            "outputs": {"9": {"images": [{"filename": "hoi4_00001_.png", "subfolder": "", "type": "output"}]}}}});
        fixture.respond(Method::Get, "$/history/pid-synthetic-1", 200, &[], serde_json::to_vec(&done).unwrap());
        fixture.respond(Method::Get, "/view?filename=hoi4_00001_.png", 200, &[], png());
        match p.reconcile(&job, &ctx).unwrap() {
            Reconciled::Completed(out) => assert_eq!(out.image, png()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn api_keys_are_sent_only_over_https_or_loopback() {
        let fixture = Arc::new(FixtureTransport::new());
        server(&fixture);
        let cfg = ProviderConfig { endpoint: Some("http://192.168.1.20:8188".into()), ..Default::default() };
        let p = ComfyProvider::new(&cfg, plain_template(), Some("comfy-secret-key".into()), fixture.clone())
            .unwrap()
            .with_polling(Duration::from_millis(1), 3);
        p.generate(&request(), &CallContext::new()).unwrap();
        assert!(
            fixture.requests().iter().all(|r| !r.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("x-api-key"))),
            "no key over cleartext LAN http"
        );

        let fixture = Arc::new(FixtureTransport::new());
        server(&fixture);
        let cfg = ProviderConfig { endpoint: Some("https://pod-8188.proxy.example".into()), ..Default::default() };
        let p = ComfyProvider::new(&cfg, plain_template(), Some("comfy-secret-key".into()), fixture.clone())
            .unwrap()
            .with_polling(Duration::from_millis(1), 3);
        p.generate(&request(), &CallContext::new()).unwrap();
        assert!(fixture.requests().iter().all(|r| r.headers.iter().any(|(k, v)| k == "X-API-Key" && v == "comfy-secret-key")));
    }

    #[test]
    fn estimate_is_free_and_local() {
        let p = provider(plain_template(), Arc::new(FixtureTransport::new()));
        let e = p.estimate(&request()).unwrap();
        assert_eq!((e.usd, e.funding), (0.0, Funding::Local));
        assert!(p.estimate(&ImageRequest { model: "workflow:other".into(), ..request() }).is_err());
        let (w, h) = p.prepare(&request()).unwrap();
        assert!(w % 8 == 0 && h % 8 == 0 && h > w);
    }

    #[test]
    fn helpers() {
        assert_eq!(pct("a b/c.png"), "a%20b%2Fc.png");
        let (b, body) = multipart(&[Part::Text { name: "a", value: "b" }]);
        assert!(String::from_utf8(body).unwrap().contains(&format!("--{b}--")));
        let cfg = ProviderConfig::default();
        assert!(ComfyProvider::new(&cfg, plain_template(), None, Arc::new(FixtureTransport::new())).is_err());
        let cfg = ProviderConfig { endpoint: Some("ftp://x".into()), ..Default::default() };
        assert!(ComfyProvider::new(&cfg, plain_template(), None, Arc::new(FixtureTransport::new())).is_err());
    }
}
