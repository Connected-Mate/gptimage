//! The provider trait and the data it moves.
//!
//! Rules the trait encodes (research report 12, section 7.2): the provider and model come from the plan,
//! never from prompt text; unsupported options fail before any request is sent; one image per call; there is
//! no silent fallback to another provider; model ids, limits and prices live in dated data tables, not in
//! code paths or skills.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use imagegen_core::error::RetryClass;
use imagegen_core::refs::RefRole;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Date the model tables were last checked against vendor documentation.
pub const TABLES_RETRIEVED: &str = "2026-10-31";

/// Lifecycle of a model id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelStatus {
    /// Usable.
    Current,
    /// Usable but scheduled for shutdown.
    Deprecated {
        /// Shutdown date.
        shutdown: String,
    },
    /// Shut down: requests are refused before they are sent.
    Retired {
        /// Shutdown date.
        on: String,
    },
}

/// Native transparency support as documented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlphaSupport {
    /// Generally available.
    Ga,
    /// Documented as a preview.
    Preview,
    /// Not documented: the pipeline must prove transparency from pixels and never rely on the prompt.
    None,
}

/// How a model chooses output sizes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SizeRule {
    /// Any width and height meeting the constraints.
    Arbitrary {
        /// Both edges are multiples of this.
        multiple: u32,
        /// Smallest total pixel count.
        min_pixels: u64,
        /// Largest total pixel count.
        max_pixels: u64,
        /// Longest edge.
        max_edge: u32,
        /// Largest long:short ratio.
        max_ratio: f64,
    },
    /// A fixed list of sizes.
    Fixed {
        /// Allowed `[w, h]` pairs.
        sizes: Vec<[u32; 2]>,
    },
    /// Aspect-ratio choices and size tiers (Gemini).
    AspectTiers {
        /// Allowed aspect ratios as `w:h`.
        ratios: Vec<String>,
        /// Tier used by this tool.
        tier: String,
    },
}

/// How a call is priced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CostModel {
    /// A price per image by quality tier, at about one megapixel.
    PerImage {
        /// `(quality, usd)` pairs.
        usd_by_quality: Vec<(String, f64)>,
    },
    /// A price for the first megapixel and for each further megapixel (inputs and outputs).
    PerMegapixel {
        /// USD for the first megapixel.
        first: f64,
        /// USD for each additional megapixel.
        additional: f64,
    },
    /// Per-image price not published; `proxy_usd` is the stand-in used for planning.
    UnpublishedProxy {
        /// Stand-in price per call by quality.
        proxy_usd_by_quality: Vec<(String, f64)>,
        /// Which published row the stand-in comes from.
        proxy_of: String,
    },
    /// No provider fee (local or self-hosted).
    Free,
}

/// One model's reviewed facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelInfo {
    /// Model id as sent to the provider.
    pub id: String,
    /// Lifecycle.
    pub status: ModelStatus,
    /// True for the model used when none is named.
    pub recommended: bool,
    /// Most reference images one request may carry.
    pub max_refs: u8,
    /// Whether a mask is accepted.
    pub mask: bool,
    /// Native alpha.
    pub alpha: AlphaSupport,
    /// Output sizes.
    pub sizes: SizeRule,
    /// Pricing.
    pub cost: CostModel,
    /// True when the provider runs jobs asynchronously (reconcilable by job id).
    pub async_job: bool,
    /// False when the model cannot edit or take references at all.
    #[serde(default = "yes")]
    pub supports_edit: bool,
    /// Model or endpoint id to call for edits when it differs from `id` (fal: `<id>/edit`).
    #[serde(default)]
    pub edit_model: Option<String>,
    /// Extra notes.
    pub notes: String,
}

fn yes() -> bool {
    true
}

/// A provider's reviewed description. Model tables carry a retrieval date because ids retire quickly
/// (three retirements in eight months at the time of writing).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderDescriptor {
    /// Provider id.
    pub id: String,
    /// Display name.
    pub display_name: String,
    /// Vendor documentation.
    pub doc_url: String,
    /// When the tables below were checked.
    pub retrieved: String,
    /// Environment variables read for credentials.
    pub env_vars: Vec<String>,
    /// Hosts the adapter may contact.
    pub hosts: Vec<String>,
    /// Models.
    pub models: Vec<ModelInfo>,
    /// Terms and rights note shown to users.
    pub terms_note: String,
}

impl ProviderDescriptor {
    /// Find a model.
    pub fn model(&self, id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.id == id)
    }

    /// The model used when none is named.
    pub fn recommended(&self) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.recommended)
    }
}

/// Who pays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Funding {
    /// The user's own API key.
    UserKey,
    /// Local or self-hosted compute.
    Local,
    /// A subscription's plan quota.
    PlanQuota,
}

impl Funding {
    /// Stable id.
    pub fn as_str(self) -> &'static str {
        match self {
            Funding::UserKey => "user_key",
            Funding::Local => "local",
            Funding::PlanQuota => "plan_quota",
        }
    }
}

/// An estimate made without any network call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CostEstimate {
    /// USD for one call (an upper bound where the price depends on tokens).
    pub usd: f64,
    /// How the figure was derived, with the table date.
    pub basis: String,
    /// True when the provider will not report usage; the reserve then stands and is never recorded as zero.
    pub usage_unknown: bool,
    /// Funding source.
    pub funding: Funding,
}

/// A reference image ready to send.
#[derive(Debug, Clone)]
pub struct ProviderRef {
    /// Role in the request.
    pub role: RefRole,
    /// Clean PNG bytes.
    pub png: Vec<u8>,
    /// SHA-256 of `png`.
    pub sha256: String,
}

/// The image an edit changes.
#[derive(Debug, Clone)]
pub struct EditSpec {
    /// PNG bytes of the image to edit.
    pub base_png: Vec<u8>,
    /// Optional PNG mask (alpha channel marks the area to change).
    pub mask_png: Option<Vec<u8>>,
}

/// One provider request (generate when `edit` is `None`, edit otherwise).
#[derive(Debug, Clone)]
pub struct ImageRequest {
    /// Model id.
    pub model: String,
    /// Prompt, or the edit instruction.
    pub prompt: String,
    /// Things to avoid, for providers with a negative prompt.
    pub negative: Vec<String>,
    /// Requested output width.
    pub width: u32,
    /// Requested output height.
    pub height: u32,
    /// Ask for real transparency.
    pub transparent: bool,
    /// Quality tier.
    pub quality: Option<String>,
    /// References, in prompt order.
    pub refs: Vec<ProviderRef>,
    /// Edit details.
    pub edit: Option<EditSpec>,
    /// Caller's idempotency key (also used as the job client id where one exists).
    pub idempotency_key: Option<String>,
    /// Seed, where supported.
    pub seed: Option<u64>,
}

/// Reference to a provider-side job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobRef {
    /// Provider id.
    pub provider: String,
    /// Job id (fal request id, ComfyUI prompt id).
    pub id: String,
    /// Status URL, where the provider returned one.
    pub status_url: Option<String>,
    /// Result URL.
    pub response_url: Option<String>,
    /// Cancel URL.
    pub cancel_url: Option<String>,
    /// Model or endpoint the job runs.
    pub model: String,
}

/// A successful result.
#[derive(Debug, Clone)]
pub struct ProviderOutput {
    /// Raw image bytes exactly as received.
    pub image: Vec<u8>,
    /// MIME type the provider declared (or `application/octet-stream`).
    pub mime: String,
    /// Provider's request id.
    pub provider_request_id: Option<String>,
    /// Usage block as received.
    pub usage: Option<Value>,
    /// Actual cost when it can be computed from usage.
    pub actual_usd: Option<f64>,
    /// True when no usable usage came back.
    pub usage_unknown: bool,
    /// Model the provider says it ran.
    pub model_echo: Option<String>,
    /// Notes (safety messages, revised prompts).
    pub notes: Vec<String>,
    /// Job, for asynchronous providers.
    pub job: Option<JobRef>,
}

/// What a reconcile found.
#[derive(Debug, Clone)]
pub enum Reconciled {
    /// The job finished; here is the output.
    Completed(Box<ProviderOutput>),
    /// Still queued or running.
    Pending,
    /// The provider says it failed or was cancelled.
    Failed(String),
    /// The provider cannot tell (synchronous APIs).
    Unknown,
}

/// Error classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Key or endpoint missing.
    NotConfigured,
    /// The provider rejected the credentials.
    Auth,
    /// Rate limited or out of quota.
    RateLimited,
    /// Content moderation refused the request.
    ModerationBlocked,
    /// The provider says the request is malformed.
    BadRequest,
    /// This provider or model cannot do what was asked; refused before sending.
    Unsupported,
    /// The model is retired.
    ModelRetired,
    /// Network failure.
    Transport,
    /// The provider answered with something unusable.
    MalformedResponse,
    /// The image is cut off or too large.
    BadImage,
    /// The provider had an internal error.
    ServerError,
    /// The caller cancelled.
    Cancelled,
    /// A destination was blocked by policy.
    Blocked,
}

/// Provider failure.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct ProviderError {
    /// Class.
    pub kind: ErrorKind,
    /// Message with secrets removed.
    pub message: String,
    /// What the caller may do next.
    pub retry: RetryClass,
    /// HTTP status, if any.
    pub status: Option<u16>,
    /// Job, when one was created before the failure.
    pub job: Option<JobRef>,
}

impl ProviderError {
    /// Build an error.
    pub fn new(kind: ErrorKind, retry: RetryClass, message: impl Into<String>) -> Self {
        ProviderError { kind, message: message.into(), retry, status: None, job: None }
    }

    /// Attach an HTTP status.
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// Attach a job.
    pub fn with_job(mut self, job: JobRef) -> Self {
        self.job = Some(job);
        self
    }

    /// Unsupported option, refused before any request.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, RetryClass::Terminal, message)
    }

    /// Credentials or endpoint missing.
    pub fn not_configured(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotConfigured, RetryClass::RequiresAuthorization, message)
    }
}

/// Cooperative cancellation.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A fresh, un-cancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// True once cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Sleeps between polls; replaceable so tests never wait.
pub trait Sleeper: Send + Sync {
    /// Sleep for `d`.
    fn sleep(&self, d: Duration);
}

/// Real sleeping.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealSleeper;

impl Sleeper for RealSleeper {
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Gets told when an asynchronous job exists, so the ledger records it before the wait begins.
pub trait JobSink: Send + Sync {
    /// A job was created.
    fn submitted(&self, job: &JobRef);
}

/// Per-call context.
#[derive(Clone)]
pub struct CallContext {
    /// Cancellation.
    pub cancel: CancelToken,
    /// Sleeper for polling.
    pub sleeper: Arc<dyn Sleeper>,
    /// Job notifications.
    pub jobs: Option<Arc<dyn JobSink>>,
}

impl CallContext {
    /// Context with real sleeping and no sink.
    pub fn new() -> Self {
        CallContext { cancel: CancelToken::new(), sleeper: Arc::new(RealSleeper), jobs: None }
    }

    /// Tell the sink about a job.
    pub fn notify(&self, job: &JobRef) {
        if let Some(sink) = &self.jobs {
            sink.submitted(job);
        }
    }
}

impl Default for CallContext {
    fn default() -> Self {
        Self::new()
    }
}

/// An image provider.
pub trait ImageProvider: Send + Sync {
    /// Provider id.
    fn id(&self) -> &str;

    /// Reviewed description with model tables.
    fn descriptor(&self) -> &ProviderDescriptor;

    /// True when the credentials or endpoint it needs are present. Never reveals them.
    fn is_configured(&self) -> bool;

    /// Model used when the plan names none, for generation or editing.
    fn default_model(&self, editing: bool) -> String;

    /// Check a request against the model table and choose the concrete output size. No network.
    /// Returns the size to ask for. Refuses retired models and unsupported options.
    fn prepare(&self, req: &ImageRequest) -> Result<(u32, u32), ProviderError>;

    /// Estimate the cost of one call. No network.
    fn estimate(&self, req: &ImageRequest) -> Result<CostEstimate, ProviderError>;

    /// Run a generation (`req.edit` is `None`).
    fn generate(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError>;

    /// Run an edit (`req.edit` is `Some`).
    fn edit(&self, req: &ImageRequest, ctx: &CallContext) -> Result<ProviderOutput, ProviderError>;

    /// Ask the provider what became of a job. Synchronous providers return [`Reconciled::Unknown`].
    fn reconcile(&self, _job: &JobRef, _ctx: &CallContext) -> Result<Reconciled, ProviderError> {
        Ok(Reconciled::Unknown)
    }

    /// Cancel a job.
    fn cancel(&self, _job: &JobRef, _ctx: &CallContext) -> Result<(), ProviderError> {
        Err(ProviderError::unsupported("this provider has no cancellable jobs"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_token_is_shared_between_clones() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }

    #[test]
    fn funding_ids_are_stable() {
        assert_eq!(Funding::UserKey.as_str(), "user_key");
        assert_eq!(Funding::Local.as_str(), "local");
        assert_eq!(Funding::PlanQuota.as_str(), "plan_quota");
    }
}
