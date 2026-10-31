//! Parameter types of the thirteen `hoi4.image_*` tools. Unknown fields are rejected everywhere.

use imagegen_core::category::Category;
use imagegen_core::process::Op;
use imagegen_core::records::Verdict;
use imagegen_core::refs::{ReferenceInput, RefRole};
use imagegen_core::style::StyleProfile;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `hoi4.image_profiles`
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfilesParams {
    /// A profile id such as `focus_icon`. Omit to list every profile.
    #[serde(default)]
    pub id: Option<String>,
    /// Compare the profiles with the installed game's file headers and return the proposed diff.
    #[serde(default)]
    pub calibrate: Option<bool>,
}

/// What `hoi4.image_references` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReferencesAction {
    /// List reference images in the project.
    List,
    /// Record a reference image with its role and rights note.
    Add,
    /// Approve a recorded reference as part of the project's style.
    Approve,
    /// List saved style profiles.
    StyleList,
    /// Read one style profile.
    StyleGet,
    /// Save a style profile; references are pinned by SHA-256.
    StyleSave,
}

/// `hoi4.image_references`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferencesParams {
    /// Action.
    pub action: ReferencesAction,
    /// Project-relative image path (`add`, `approve`).
    #[serde(default)]
    pub path: Option<String>,
    /// Role (`add`).
    #[serde(default)]
    pub role: Option<RefRole>,
    /// Rights note: where the image comes from and what you may do with it (`add`, `approve`).
    #[serde(default)]
    pub note: Option<String>,
    /// Who approves (`approve`).
    #[serde(default)]
    pub approver: Option<String>,
    /// Style name (`style_get`).
    #[serde(default)]
    pub name: Option<String>,
    /// Style profile to save (`style_save`). Reference hashes are filled in for you.
    #[serde(default)]
    pub style: Option<StyleProfile>,
    /// Replace an existing style; the old file is backed up (`style_save`).
    #[serde(default)]
    pub replace: Option<bool>,
}

/// `hoi4.image_generate`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerateParams {
    /// Plan id from `hoi4.image_plan`.
    pub plan_id: String,
    /// Your own key for this request. Repeating it returns the stored candidate instead of paying again.
    pub idempotency_key: String,
    /// Confirm that a call costing more than the project's per-call limit is intended.
    #[serde(default)]
    pub approve_cost: Option<bool>,
}

/// `hoi4.image_edit`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditParams {
    /// Candidate or processed artifact to refine.
    pub candidate_id: String,
    /// What to change. Everything else is asked to stay as it is.
    pub instruction: String,
    /// Project-relative PNG mask; its alpha marks the area to change. Only models that take masks accept it.
    #[serde(default)]
    pub mask: Option<String>,
    /// Extra references (style anchors and so on).
    #[serde(default)]
    pub extra_references: Vec<ReferenceInput>,
    /// Your own key for this request.
    pub idempotency_key: String,
    /// Provider; defaults to the one that made the candidate.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model; defaults to the provider's editing model.
    #[serde(default)]
    pub model: Option<String>,
    /// Quality tier.
    #[serde(default)]
    pub quality: Option<String>,
    /// Confirm a call above the per-call limit.
    #[serde(default)]
    pub approve_cost: Option<bool>,
}

/// An existing image to bring under the ledger.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportSpec {
    /// Project-relative path of the image.
    pub path: String,
    /// Category it is for.
    pub category: Category,
    /// Feature folder name.
    pub feature_slug: String,
    /// Asset name.
    pub asset: String,
}

/// `hoi4.image_process`
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessParams {
    /// Candidate to process.
    #[serde(default)]
    pub candidate_id: Option<String>,
    /// Or import an existing image (art made by another tool, or by hand) and process that.
    #[serde(default)]
    pub import: Option<ImportSpec>,
    /// Explicit steps. Omit for the profile's default chain.
    #[serde(default)]
    pub steps: Option<Vec<Op>>,
    /// Canvas for categories that have none.
    #[serde(default)]
    pub canvas: Option<[u32; 2]>,
}

/// `hoi4.image_validate`
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateParams {
    /// Artifact ids, project-relative image paths, or folders (ending in `/`) of DDS, TGA or PNG files.
    pub targets: Vec<String>,
    /// Profile id or category. Needed for paths; artifacts know their own.
    #[serde(default)]
    pub profile: Option<String>,
    /// Checks to run; defaults to the profile's list.
    #[serde(default)]
    pub checks: Option<Vec<String>>,
    /// Canvas for categories that have none.
    #[serde(default)]
    pub canvas: Option<[u32; 2]>,
}

/// `hoi4.image_preview`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewParams {
    /// Artifact id or project-relative image path.
    pub target: String,
    /// Return the contact sheet as an image in the reply.
    #[serde(default)]
    pub include_image: Option<bool>,
}

/// `hoi4.image_review`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewParams {
    /// Artifact reviewed.
    pub target: String,
    /// Verdict. An approval needs a passing validation, a preview of this exact file and every rubric answer true.
    pub verdict: Verdict,
    /// Who reviewed: a person's name or an agent's name.
    pub reviewer: String,
    /// Answers to the rubric questions (`hoi4.image_profiles` lists them), as `{key: true|false}`.
    #[serde(default)]
    pub answers: Option<Value>,
    /// Notes for the next round.
    #[serde(default)]
    pub notes: Option<String>,
}

/// How an install treats the `.gfx` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
    /// Write the sprite snippet into `gfx_handoff.md` and leave `.gfx` files to the main agent (kit default).
    #[default]
    Handoff,
    /// Insert the sprite entries into the suggested `.gfx` file (text-level; comments and layout are kept).
    Register,
}

/// What to do when a final file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnExists {
    /// Stop.
    #[default]
    Fail,
    /// Replace a file this tool installed earlier. Files it did not install are never replaced.
    Replace,
}

/// `hoi4.image_install`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallParams {
    /// Processed artifact to install.
    pub target: String,
    /// `.gfx` handling.
    #[serde(default)]
    pub mode: InstallMode,
    /// Existing-file handling.
    #[serde(default)]
    pub on_exists: OnExists,
    /// Approval id returned by an approving `hoi4.image_review` of this exact file.
    pub approval_id: String,
}

/// `hoi4.image_ledger`
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerParams {
    /// Only entries of this asset (`feature.asset`).
    #[serde(default)]
    pub asset_id: Option<String>,
    /// Only entries of this candidate or processed artifact.
    #[serde(default)]
    pub artifact_id: Option<String>,
    /// Only entries of this kind (`generated`, `validated`, `reviewed`, `installed`...).
    #[serde(default)]
    pub kind: Option<String>,
    /// Most recent entries to return (default 50, at most 500).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Include the chain of parents of `artifact_id`.
    #[serde(default)]
    pub lineage: Option<bool>,
}

/// `hoi4.image_cancel`
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelParams {
    /// Job id from the ledger.
    #[serde(default)]
    pub job_id: Option<String>,
    /// Or the idempotency key of a running generate or edit.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// For a job left behind by a crashed run: ask the provider what became of it and store the image if it finished.
    #[serde(default)]
    pub reconcile: Option<bool>,
}
