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
    /// A paid call is about to be made. Written before the call, so a crash leaves evidence.
    pub const JOB_INTENT: &str = "job_intent";
    /// An asynchronous provider accepted a job and returned its reference.
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
    /// Family.
    pub family: crate::family::Family,
    /// Feature folder name.
    pub feature_slug: String,
    /// Asset name.
    pub asset: String,
    /// Profile in force when the candidate was made.
    pub profile: ProfileRef,
    /// Effective canvas per frame (the profile's, or the explicit one for custom categories).
    pub canvas: [u32; 2],
    /// Version number.
    pub version: u32,
    /// Project-relative path of the raw image.
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

/// An engine-level finishing stage that ran (a template composite, a face crop, the flag sizes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FinisherRecord {
    /// Stage name (`advisor_card`, `achievement_triplet`, `extract_portrait`, `flag_ladder`).
    pub name: String,
    /// SHA-256 of the pixels the stage received.
    pub input_sha256: String,
    /// SHA-256 of the main image it produced.
    pub output_sha256: String,
    /// What the stage used and decided: template hashes, the detected face box, the audit result.
    pub detail: Value,
}

/// The recipe a processed artifact went through. Validation and the install gate refuse an artifact whose
/// recipe is not its family's, or whose chain lacks one of the family's required stages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RecipeRecord {
    /// Recipe id (`icon`, `report_card`, `achievement`...).
    pub id: String,
    /// Recipe version.
    pub version: u32,
    /// Engine-level stages that ran after the steps, in order.
    pub finishers: Vec<FinisherRecord>,
}

/// An image that belongs to a processed artifact and is installed with it: the achievement variants, the smaller
/// flag sizes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompanionRecord {
    /// Which companion (`_grey`, `_not_eligible`, `medium`, `small`).
    pub key: String,
    /// Project-relative path of the companion PNG.
    pub path: String,
    /// SHA-256 of the file.
    pub sha256: String,
    /// SHA-256 of the decoded pixels.
    pub pixels_sha256: String,
    /// Pixel size.
    pub size: [u32; 2],
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
    /// Family of the source candidate (the lock: it can never change).
    pub family: crate::family::Family,
    /// Run number for this source.
    pub run: u32,
    /// Effective canvas per frame.
    pub canvas: [u32; 2],
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
    /// The recipe and its engine-level stages. Missing in records made before recipes were recorded.
    #[serde(default)]
    pub recipe: Option<RecipeRecord>,
    /// Companion images installed with this artifact.
    #[serde(default)]
    pub companions: Vec<CompanionRecord>,
}

/// A validation run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ValidatedRecord {
    /// Artifact id (or the path for files outside the ledger).
    pub target: String,
    /// Family the validation ran as: the artifact's own, or the one named for a plain file.
    pub family: crate::family::Family,
    /// Hash of the check set that ran (`validate::check_set_hash`). The gate compares it with the family's
    /// full set, so a validation that skipped a check cannot open it.
    pub check_set_hash: String,
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
    /// Family of the artifact when it was reviewed.
    pub family: crate::family::Family,
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
    /// The measured vanilla-match check the review ran (absent in reviews recorded before 0.3.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vanilla_check: Option<Value>,
    /// The reviewer's override of the vanilla-match check, with its reason, when one was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vanilla_override: Option<Value>,
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
    /// Family the files were installed as. The reuse guard reads this.
    pub family: crate::family::Family,
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
    /// Division template symbol number of a division emblem (`GFX_div_templ_<n>_large`), chosen at install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub division_template_number: Option<u32>,
    /// What the mod's script writes so the game finds the art, with the names filled in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
}

/// A file that an approved replace overwrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReplacedFile {
    /// Project-relative path.
    pub path: String,
    /// SHA-256 of the previous bytes.
    pub previous_sha256: String,
}
