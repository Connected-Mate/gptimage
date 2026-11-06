//! The operations behind the tools. The MCP server and the CLI both call this, so a tool and its
//! command-line twin can never drift apart.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use imagegen_core::config::{Config, EnvSource};
use imagegen_core::error::{Error as CoreError, RetryClass};
use imagegen_core::family::{Automation, Family};
use imagegen_core::fsio::ProjectFs;
use imagegen_core::hashing::{sha256_hex, Clock};
use imagegen_core::ledger::Ledger;
use imagegen_core::profile::{Profile, ProfileSet};
use imagegen_core::records::{kind, CandidateRecord, CompanionRecord, ProcessedRecord, ProfileRef};
use imagegen_providers::types::{CancelToken, JobRef, ProviderError, Sleeper};
use imagegen_providers::{ErrorKind, HttpTransport, ProviderRegistry};
use serde_json::Value;

pub mod audit;
pub mod autostyle;
pub mod derive;
pub mod diversity;
pub mod finish;
pub mod generate;
pub mod install;
pub mod misc;
pub mod pack;
pub mod plan;
pub mod preview;
pub mod process;
pub mod review;
pub mod validate;
pub mod vanilla;

/// How the process was started. The ChatGPT-login plugin refuses everything but `LocalStdio` and `Cli`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// An MCP server on stdio, started by a local client.
    LocalStdio,
    /// The command-line tool on the user's machine.
    Cli,
    /// A hosted or networked service. Never used by this repository's binaries; exists so the gates can refuse it.
    Hosted,
}

/// Any failure an operation reports. `code` is stable and `retry` says what the caller may do next.
#[derive(Debug, Clone)]
pub struct EngineError {
    /// Stable machine code.
    pub code: String,
    /// Explanation (no secrets).
    pub message: String,
    /// Retry class.
    pub retry: RetryClass,
    /// Extra structured detail.
    pub details: Option<Value>,
}

impl EngineError {
    /// Build an error.
    pub fn new(code: &str, message: impl Into<String>, retry: RetryClass) -> Self {
        EngineError { code: code.to_string(), message: message.into(), retry, details: None }
    }

    /// Attach details.
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Invalid arguments.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_argument", message, RetryClass::Terminal)
    }

    /// Refusal by policy or gate.
    pub fn refused(code: &str, message: impl Into<String>, retry: RetryClass) -> Self {
        Self::new(code, message, retry)
    }

    /// JSON form for tool results.
    pub fn to_json(&self) -> Value {
        let mut v = serde_json::json!({"error": {"code": self.code, "message": self.message, "retry_class": self.retry}});
        if let Some(d) = &self.details {
            v["error"]["details"] = d.clone();
        }
        v
    }
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for EngineError {}

impl From<CoreError> for EngineError {
    fn from(e: CoreError) -> Self {
        EngineError::new(e.code(), e.to_string(), e.retry_class())
    }
}

impl From<ProviderError> for EngineError {
    fn from(e: ProviderError) -> Self {
        let code = match e.kind {
            ErrorKind::NotConfigured => "provider_not_configured",
            ErrorKind::Auth => "provider_auth",
            ErrorKind::RateLimited => "provider_rate_limited",
            ErrorKind::ModerationBlocked => "provider_moderation_blocked",
            ErrorKind::BadRequest => "provider_bad_request",
            ErrorKind::Unsupported => "provider_unsupported",
            ErrorKind::ModelRetired => "provider_model_retired",
            ErrorKind::Transport => "provider_transport",
            ErrorKind::MalformedResponse => "provider_malformed_response",
            ErrorKind::BadImage => "provider_bad_image",
            ErrorKind::ServerError => "provider_server_error",
            ErrorKind::Cancelled => "provider_cancelled",
            ErrorKind::Blocked => "provider_blocked",
        };
        let mut err = EngineError::new(code, e.message.clone(), e.retry);
        if let Some(job) = &e.job {
            err.details = Some(serde_json::json!({"job": job}));
        }
        err
    }
}

/// Result alias.
pub type EResult<T> = Result<T, EngineError>;

/// A generate or edit that is running now.
#[derive(Clone)]
pub struct ActiveJob {
    /// Cancellation handle.
    pub cancel: CancelToken,
    /// Provider id.
    pub provider: String,
    /// Idempotency key of the request.
    pub idempotency_key: String,
    /// Provider-side job, once known.
    pub remote: Option<JobRef>,
}

/// Running jobs, so `hoi4.image_cancel` can reach them.
#[derive(Default)]
pub struct JobTable {
    inner: Mutex<HashMap<String, ActiveJob>>,
}

impl JobTable {
    pub(crate) fn insert(&self, id: &str, job: ActiveJob) {
        self.inner.lock().expect("jobs lock").insert(id.to_string(), job);
    }

    pub(crate) fn remove(&self, id: &str) {
        self.inner.lock().expect("jobs lock").remove(id);
    }

    pub(crate) fn set_remote(&self, id: &str, remote: JobRef) {
        if let Some(j) = self.inner.lock().expect("jobs lock").get_mut(id) {
            j.remote = Some(remote);
        }
    }

    pub(crate) fn find(&self, job_id: Option<&str>, key: Option<&str>) -> Option<(String, ActiveJob)> {
        let map = self.inner.lock().expect("jobs lock");
        map.iter()
            .find(|(id, j)| job_id.map(|w| w == id.as_str()).unwrap_or(false) || key.map(|k| k == j.idempotency_key).unwrap_or(false))
            .map(|(id, j)| (id.clone(), j.clone()))
    }

    /// Number of running jobs.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("jobs lock").len()
    }

    /// True when nothing is running.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Everything an operation needs.
pub struct Engine {
    /// Project root with containment.
    pub fs: ProjectFs,
    /// Project config.
    pub config: Config,
    /// Environment (keys, acknowledgements).
    pub env: Arc<dyn EnvSource>,
    /// Effective profiles.
    pub profiles: ProfileSet,
    /// Providers.
    pub registry: ProviderRegistry,
    /// Provenance ledger.
    pub ledger: Ledger,
    /// Time source.
    pub clock: Arc<dyn Clock>,
    /// Running jobs.
    pub jobs: Arc<JobTable>,
    /// How this process was started.
    pub mode: RunMode,
    /// Sleeper handed to polling providers.
    pub sleeper: Arc<dyn Sleeper>,
    /// What the ChatGPT-login plugin gates said at start-up.
    pub plugin: crate::plugin::PluginReport,
    /// Measured vanilla art per family and game folder, for the review's vanilla-match check and the preview.
    pub(crate) vanilla_cache: vanilla::Cache,
}

impl Engine {
    /// Open a project.
    pub fn open(root: &Path, env: Arc<dyn EnvSource>, transport: Arc<dyn HttpTransport>, clock: Arc<dyn Clock>, mode: RunMode) -> EResult<Engine> {
        let fs = ProjectFs::new(root)?;
        let config = Config::load(fs.root())?;
        let kit = config.kit_profiles_file(fs.root());
        let profiles = ProfileSet::load(Some(&kit), &config.profiles)?;
        let mut registry = ProviderRegistry::build(&fs, &config, &*env, transport.clone());
        let plugin = crate::plugin::register(&mut registry, &config, &*env, mode, transport);
        let ledger = Ledger::new(fs.clone(), clock.clone());
        Ok(Engine {
            fs,
            config,
            env,
            profiles,
            registry,
            ledger,
            clock,
            jobs: Arc::new(JobTable::default()),
            mode,
            sleeper: Arc::new(imagegen_providers::types::RealSleeper),
            plugin,
            vanilla_cache: vanilla::Cache::default(),
        })
    }

    /// The effective profile of a family.
    pub(crate) fn family_profile(&self, family: Family) -> EResult<&Profile> {
        self.profiles.for_family(family).ok_or_else(|| EngineError::new("not_found", format!("no profile for family {family}"), RetryClass::Terminal))
    }

    /// The profile of an artifact: always its own family's, never a caller's choice, with the style pack it was
    /// planned with applied again (its canvas, processing switches and checks).
    pub(crate) fn profile_of(&self, a: &Artifact) -> EResult<Profile> {
        self.profile_with_pack(a.family, a.profile.style_pack.as_deref())
    }

    /// What the install gate expects of the records of an artifact with this effective profile.
    pub(crate) fn gate_expect(&self, profile: &Profile) -> imagegen_core::gate::GateExpect {
        imagegen_core::gate::GateExpect::for_profile(profile)
    }

    pub(crate) fn profile_ref(&self, p: &Profile) -> ProfileRef {
        ProfileRef {
            id: p.id.clone(),
            hash: ProfileSet::profile_hash(p),
            kit_sha256: self.profiles.kit_sha256.clone(),
            style_pack: p.pack.as_ref().map(|x| x.name.clone()),
        }
    }

    pub(crate) fn game_dir(&self) -> Option<std::path::PathBuf> {
        imagegen_core::config::find_game_dir(&self.config, &*self.env)
    }

    /// Refuse families the project or the profile does not allow, for a given provider (decision D3).
    pub(crate) fn check_family_policy(&self, profile: &Profile, provider: &str) -> EResult<()> {
        if self.config.policy.forbidden_families.contains(&profile.family) {
            return Err(EngineError::refused(
                "family_forbidden_by_project",
                format!("this project's policy.forbidden_families lists {}; no provider may generate it", profile.family),
                RetryClass::Terminal,
            ));
        }
        match profile.imagegen.automation {
            Automation::Generate => Ok(()),
            Automation::ImportOnly => Err(EngineError::refused(
                "family_import_only",
                format!(
                    "{} is built by another kit tool. Bring its output under the ledger with hoi4.image_process (import), then validate, review and install it",
                    profile.family
                ),
                RetryClass::Terminal,
            )),
            Automation::UserRunOnly if provider == "comfyui" => Ok(()),
            Automation::UserRunOnly => Err(EngineError::refused(
                "family_user_run_only",
                format!(
                    "{} is user-run: only a ComfyUI endpoint you configured may generate it (decision D3). Import the result with hoi4.image_process instead",
                    profile.family
                ),
                RetryClass::RequiresAuthorization,
            )),
        }
    }

    /// Current time.
    pub(crate) fn now(&self) -> String {
        self.clock.now_rfc3339()
    }
}

/// True when a target string is an artifact id (`feature.asset-v3`), not a path.
pub(crate) fn looks_like_artifact(s: &str) -> bool {
    !s.contains('/') && !s.contains('\\') && imagegen_core::naming::ArtifactId::parse(s).is_ok()
}

/// Raw candidate or processed artifact, resolved from the ledger.
#[derive(Debug, Clone)]
pub struct Artifact {
    /// Id.
    pub id: String,
    /// True for a processed artifact.
    pub processed: bool,
    /// `feature.asset`
    pub asset_id: String,
    /// Project-relative path.
    pub path: String,
    /// Hash recorded when it was written.
    pub recorded_sha256: String,
    /// Family.
    pub family: Family,
    /// Feature folder.
    pub feature_slug: String,
    /// Asset name.
    pub asset: String,
    /// Profile in force.
    pub profile: ProfileRef,
    /// Effective canvas per frame.
    pub canvas: [u32; 2],
    /// Id of the raw candidate this derives from (itself for a raw candidate).
    pub root_id: String,
    /// Provider that made the root candidate (`import` for imports).
    pub provider: String,
    /// The record of a processed artifact (its recipe and companions).
    pub record: Option<ProcessedRecord>,
    /// The sub-family the root candidate was planned in, when it had one.
    pub subfamily: Option<String>,
}

impl Artifact {
    /// Companion images installed with the artifact.
    pub fn companions(&self) -> &[CompanionRecord] {
        self.record.as_ref().map(|r| r.companions.as_slice()).unwrap_or(&[])
    }
}

impl Engine {
    /// Look an artifact up in the ledger.
    pub(crate) fn artifact(&self, id: &str) -> EResult<Artifact> {
        imagegen_core::naming::ArtifactId::parse(id)?;
        let entries = self.ledger.entries()?;
        let find_candidate = |cid: &str| -> Option<CandidateRecord> {
            entries
                .iter()
                .filter(|e| matches!(e.kind.as_str(), kind::GENERATED | kind::EDITED | kind::IMPORTED))
                .find(|e| e.artifact_id.as_deref() == Some(cid))
                .and_then(|e| serde_json::from_value(e.data.clone()).ok())
        };
        if let Some(p) = entries
            .iter()
            .find(|e| e.kind == kind::PROCESSED && e.artifact_id.as_deref() == Some(id))
            .and_then(|e| serde_json::from_value::<ProcessedRecord>(e.data.clone()).ok())
        {
            let root_id = imagegen_core::naming::ArtifactId::parse(&p.source_id)?.raw_id();
            let root = find_candidate(&root_id).ok_or_else(|| {
                EngineError::new("ledger_error", format!("the root candidate {root_id} of {id} is missing from the ledger"), RetryClass::Terminal)
            })?;
            if root.family != p.family || p.profile.id != p.family.as_str() {
                return Err(EngineError::refused(
                    "GFX-FAMILY-MISMATCH",
                    format!(
                        "the records of {id} disagree about its family ({} against {}); an asset keeps the family it was planned as",
                        root.family, p.family
                    ),
                    RetryClass::Terminal,
                ));
            }
            let recorded = imagegen_core::chain::bundle_sha256(&p.sha256, &p.companions);
            return Ok(Artifact {
                id: id.to_string(),
                processed: true,
                asset_id: p.asset_id.clone(),
                path: p.path.clone(),
                recorded_sha256: recorded,
                family: p.family,
                feature_slug: root.feature_slug,
                asset: root.asset,
                profile: p.profile.clone(),
                canvas: p.canvas,
                root_id,
                provider: root.provider,
                record: Some(p),
                subfamily: root.subfamily,
            });
        }
        if let Some(c) = find_candidate(id) {
            if c.profile.id != c.family.as_str() {
                return Err(EngineError::refused(
                    "GFX-FAMILY-MISMATCH",
                    format!("candidate {id} is a {} but was recorded against profile {}", c.family, c.profile.id),
                    RetryClass::Terminal,
                ));
            }
            return Ok(Artifact {
                id: id.to_string(),
                processed: false,
                asset_id: c.asset_id,
                path: c.path,
                recorded_sha256: c.sha256,
                family: c.family,
                feature_slug: c.feature_slug,
                asset: c.asset,
                profile: c.profile,
                canvas: c.canvas,
                root_id: id.to_string(),
                provider: c.provider,
                record: None,
                subfamily: c.subfamily,
            });
        }
        Err(EngineError::new("not_found", format!("no candidate or processed artifact '{id}' in the ledger"), RetryClass::Terminal))
    }

    /// Read an artifact's file and hash it. With companions the hash covers the main file and every companion
    /// ([`imagegen_core::chain::bundle_sha256`]), so a validation, a review or an approval of "the file" is of all
    /// the files an install will write.
    pub(crate) fn read_artifact(&self, a: &Artifact) -> EResult<(Vec<u8>, String)> {
        let bytes = self.fs.read(&a.path, 64 * 1024 * 1024)?;
        if a.companions().is_empty() {
            let sha = sha256_hex(&bytes);
            return Ok((bytes, sha));
        }
        let mut found: Vec<CompanionRecord> = Vec::new();
        for c in a.companions() {
            let data = self.fs.read(&c.path, 64 * 1024 * 1024).map_err(|e| match e {
                CoreError::NotFound(_) => EngineError::refused(
                    "companion_missing",
                    format!("{} is missing: it belongs to {} ({}). Restore it or process the source candidate again", c.path, a.id, c.key),
                    RetryClass::RefreshPreconditions,
                ),
                other => other.into(),
            })?;
            found.push(CompanionRecord { sha256: sha256_hex(&data), ..c.clone() });
        }
        let sha = imagegen_core::chain::bundle_sha256(&sha256_hex(&bytes), &found);
        Ok((bytes, sha))
    }

    /// The companion images of an artifact with their bytes.
    pub(crate) fn read_companions(&self, a: &Artifact) -> EResult<Vec<(CompanionRecord, Vec<u8>)>> {
        a.companions().iter().map(|c| Ok((c.clone(), self.fs.read(&c.path, 64 * 1024 * 1024)?))).collect()
    }
}
