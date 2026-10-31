//! Typed ledger events. Each variant is one append to the provenance ledger.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::process::StepRecord;
use crate::refs::{RefOrigin, RefRole};
use crate::validate::{Finding, Status};

/// Ledger event kinds.
pub mod kind {
    /// A plan was created.
    pub const PLAN: &str = "plan_created";
    /// A provider call produced a candidate.
    pub const GENERATED: &str = "generated";
    /// A provider edit produced a candidate.
    pub const EDITED: &str = "edited";
    /// An existing image was imported as a candidate.
    pub const IMPORTED: &str = "imported";
    /// A provider job was submitted (asynchronous providers) or is about to be.
    pub const JOB_SUBMITTED: &str = "job_submitted";
    /// A provider job ended.
    pub const JOB_ENDED: &str = "job_ended";
    /// The deterministic chain produced a processed artifact.
    pub const PROCESSED: &str = "processed";
    /// Deterministic validation ran.
    pub const VALIDATED: &str = "validated";
    /// Previews were written.
    pub const PREVIEWED: &str = "previewed";
    /// A visual review or approval was recorded.
    pub const REVIEWED: &str = "reviewed";
    /// Final files were written.
    pub const INSTALLED: &str = "installed";
    /// A reference was added or approved.
    pub const REFERENCE: &str = "reference";
    /// A style profile was saved.
    pub const STYLE: &str = "style_saved";
}

/// Which profile and file a record was made against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileRef {
    /// Profile id.
    pub id: String,
    /// Hash of the effective profile.
    pub hash: String,
    /// SHA-256 of the kit `profiles.json` that fed it, when one did.
    pub kit_sha256: Option<String>,
}

/// A reference as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RefSummary {
    /// Display path.
    pub path: String,
    /// Role.
    pub role: RefRole,
    /// Origin.
    pub origin: RefOrigin,
    /// SHA-256 of the clean PNG that was sent.
    pub sha256: String,
}

/// Cost as recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CostRecord {
    /// Estimate made at plan time, in USD.
    pub estimated_usd: f64,
    /// Amount reserved before the call.
    pub reserved_usd: f64,
    /// Actual cost when the provider reported usage.
    pub actual_usd: Option<f64>,
    /// True when the provider returned no usable usage; the reserve then stands, never zero.
    pub usage_unknown: bool,
    /// Who pays: `user_key`, `local` or `plan_quota`.
    pub funding: String,
    /// Basis of the estimate.
    pub basis: String,
}

/// A candidate produced by a provider or imported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CandidateRecord {
    /// `feature.asset-vN`
    pub candidate_id: String,
    /// `feature.asset`
    pub asset_id: String,
    /// Version number.
    pub version: u32,
    /// Project-relative path of the raw PNG.
    pub path: String,
    /// SHA-256 of the file.
    pub sha256: String,
    /// SHA-256 of the decoded RGBA pixels.
    pub pixels_sha256: String,
    /// Pixel size.
    pub size: [u32; 2],
    /// True when any pixel is translucent or transparent.
    pub has_transparency: bool,
    /// Provider id (`import` for imported images).
    pub provider: String,
    /// Exact model id.
    pub model: String,
    /// Plan this came from.
    pub plan_id: Option<String>,
    /// Hash of the plan.
    pub plan_hash: Option<String>,
    /// SHA-256 of the provider request.
    pub request_hash: String,
    /// Caller's idempotency key.
    pub idempotency_key: Option<String>,
    /// SHA-256 of the prompt text.
    pub prompt_sha256: String,
    /// References that were sent.
    pub references: Vec<RefSummary>,
    /// Cost.
    pub cost: CostRecord,
    /// Parent candidate for edits.
    pub parent: Option<String>,
    /// Provider's request id.
    pub provider_request_id: Option<String>,
    /// Usage as the provider reported it.
    pub usage: Option<Value>,
    /// Note the provider attached (for example a safety message).
    pub note: Option<String>,
}

/// A processed artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessedRecord {
    /// `feature.asset-vN-pM`
    pub artifact_id: String,
    /// Candidate it was made from.
    pub source_id: String,
    /// `feature.asset`
    pub asset_id: String,
    /// Run number for this source.
    pub run: u32,
    /// Project-relative path of the processed PNG.
    pub path: String,
    /// SHA-256 of the file.
    pub sha256: String,
    /// SHA-256 of the decoded RGBA pixels.
    pub pixels_sha256: String,
    /// Pixel size.
    pub size: [u32; 2],
    /// Steps that ran.
    pub steps: Vec<StepRecord>,
    /// Profile used.
    pub profile: ProfileRef,
}

/// A validation run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ValidatedRecord {
    /// Artifact id (or the path for files outside the ledger).
    pub target: String,
    /// SHA-256 of the validated file.
    pub sha256: String,
    /// Result.
    pub status: Status,
    /// Error count.
    pub errors: u32,
    /// Warning count.
    pub warnings: u32,
    /// Findings.
    pub findings: Vec<Finding>,
    /// Checks that ran.
    pub checks: Vec<String>,
    /// Profile used.
    pub profile: ProfileRef,
}

/// Review verdicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The art is approved for install.
    Approved,
    /// The art is rejected.
    Rejected,
    /// The art needs another round.
    ChangesRequested,
}

/// A visual review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReviewedRecord {
    /// Artifact reviewed.
    pub target: String,
    /// SHA-256 of the file the reviewer saw.
    pub sha256: String,
    /// Verdict.
    pub verdict: Verdict,
    /// Who reviewed (a person, or an agent name).
    pub reviewer: String,
    /// Rubric id.
    pub rubric: String,
    /// Answers to the rubric questions.
    pub answers: Value,
    /// Free notes.
    pub notes: String,
    /// Approval id, present for approvals. Install needs it.
    pub approval_id: Option<String>,
}

/// One installed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledFile {
    /// Project-relative path.
    pub path: String,
    /// SHA-256 of the file bytes.
    pub sha256: String,
    /// SHA-256 of the decoded pixels (matches `pixels_sha256` of the processed art).
    pub pixels_sha256: String,
    /// `dds` or `tga`.
    pub container: String,
}

/// Install result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledRecord {
    /// Artifact installed.
    pub target: String,
    /// `handoff` or `register`.
    pub mode: String,
    /// Files written.
    pub files: Vec<InstalledFile>,
    /// Sprite names produced.
    pub sprites: Vec<String>,
    /// `.gfx` file written or suggested.
    pub gfx_file: Option<String>,
    /// Hashes of files that were replaced.
    pub replaced: Vec<ReplacedFile>,
    /// Approval used.
    pub approval_id: String,
}

/// A file that an approved replace overwrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReplacedFile {
    /// Project-relative path.
    pub path: String,
    /// SHA-256 of the previous bytes.
    pub previous_sha256: String,
}
