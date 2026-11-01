//! Parameter types of the `hoi4.image_*` tools. Unknown fields are rejected everywhere.

use imagegen_core::family::{Family, Group};
use imagegen_core::plan::PlanRequest;
use imagegen_core::process::Op;
use imagegen_core::records::Verdict;
use imagegen_core::refs::{RefRole, ReferenceInput};
use imagegen_core::style::StyleProfile;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `hoi4.image_families`
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FamiliesParams {
    /// A family id such as `focus`. Omit to list every family.
    #[serde(default)]
    pub family: Option<Family>,
    /// Compare the families with the installed game's file headers and return the proposed diff.
    #[serde(default)]
    pub calibrate: Option<bool>,
}

/// What a group tool does after it has planned.
#[derive(Debug, Clone, Default)]
pub struct GroupFollowUp {
    /// Generate now (one paid call per variant). Without it the tool only returns the free plan.
    pub approve: bool,
    /// Needed with `approve`.
    pub idempotency_key: Option<String>,
    /// Confirm a call above the per-call limit.
    pub approve_cost: bool,
}

/// Schema functions of the `family` field: a string whose enum lists exactly the families of one group.
mod family_schemas {
    use imagegen_core::family::Group;

    fn schema(group: Group) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "enum": group.family_ids(),
            "description": format!(
                "Which {} asset to make. The family alone selects canvas, frames, naming, references, processing and validation, and it never changes afterwards.",
                group.as_str()
            )
        })
    }

    pub fn icon(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema(Group::Icon)
    }
    pub fn picture(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema(Group::Picture)
    }
    pub fn portrait(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema(Group::Portrait)
    }
    pub fn flag(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema(Group::Flag)
    }
    pub fn unit(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema(Group::Unit)
    }
    pub fn interface(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema(Group::Interface)
    }
}

macro_rules! group_request {
    ($(#[$doc:meta])* $name:ident, $group:expr, $schema_fn:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            /// The family to make. Required; the schema lists only this tool's families.
            #[schemars(schema_with = $schema_fn)]
            pub family: Family,
            /// Feature folder name (snake_case).
            pub feature_slug: String,
            /// Asset name (snake_case; a country tag with an optional ideology suffix for flags).
            pub asset: String,
            /// What to depict, in plain words (at most 4000 characters).
            pub brief: String,
            /// Project style profile to apply.
            #[serde(default)]
            pub style: Option<String>,
            /// Reference images of your own. Style references are also chosen automatically (the mod first, then vanilla), up to the provider limit.
            #[serde(default)]
            pub references: Vec<ReferenceInput>,
            /// Provider id; defaults to the style's, the project's, or the first ready one.
            #[serde(default)]
            pub provider: Option<String>,
            /// Model id; defaults to the provider's recommended model.
            #[serde(default)]
            pub model: Option<String>,
            /// Quality tier where the model has tiers.
            #[serde(default)]
            pub quality: Option<String>,
            /// Canvas `[width, height]` for families that have none (the mod defines it). Refused for the others.
            #[serde(default)]
            pub canvas: Option<[u32; 2]>,
            /// Candidates to produce in one go (1 to 4); each is a separate provider call.
            #[serde(default)]
            pub variants: Option<u8>,
            /// Refuse the plan if the estimate exceeds this many USD.
            #[serde(default)]
            pub budget_usd: Option<f64>,
            /// true: plan and generate in this call. Omitted or false: return the free plan only.
            #[serde(default)]
            pub approve: Option<bool>,
            /// Your own key for this request (needed with approve). Repeating it returns the stored candidate instead of paying again.
            #[serde(default)]
            pub idempotency_key: Option<String>,
            /// Confirm that a call costing more than the project's per-call limit is intended.
            #[serde(default)]
            pub approve_cost: Option<bool>,
        }

        impl $name {
            /// The group this tool serves.
            pub const GROUP: Group = $group;

            /// Split into the plan request and what to do after planning.
            pub fn into_parts(self) -> (PlanRequest, GroupFollowUp) {
                let follow = GroupFollowUp {
                    approve: self.approve.unwrap_or(false),
                    idempotency_key: self.idempotency_key,
                    approve_cost: self.approve_cost.unwrap_or(false),
                };
                let req = PlanRequest {
                    family: self.family,
                    feature_slug: self.feature_slug,
                    asset: self.asset,
                    brief: self.brief,
                    style: self.style,
                    references: self.references,
                    provider: self.provider,
                    model: self.model,
                    transparent: None,
                    variants: self.variants,
                    budget_usd: self.budget_usd,
                    quality: self.quality,
                    canvas: self.canvas,
                };
                (req, follow)
            }
        }
    };
}

group_request!(
    /// `hoi4.image_icon`: single-frame transparent icons.
    IconRequest, Group::Icon, "family_schemas::icon"
);
group_request!(
    /// `hoi4.image_picture`: opaque pictures and card art.
    PictureRequest, Group::Picture, "family_schemas::picture"
);
group_request!(
    /// `hoi4.image_portrait`: character portraits and advisor cards.
    PortraitRequest, Group::Portrait, "family_schemas::portrait"
);
group_request!(
    /// `hoi4.image_flag`: country flags (one call writes the whole three-size ladder).
    FlagRequest, Group::Flag, "family_schemas::flag"
);
group_request!(
    /// `hoi4.image_unit`: counters, emblems, buildings and logos (frame strips).
    UnitRequest, Group::Unit, "family_schemas::unit"
);
group_request!(
    /// `hoi4.image_interface`: interface art whose canvas the mod defines.
    InterfaceRequest, Group::Interface, "family_schemas::interface"
);

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
    /// Plan id returned by a group tool (`hoi4.image_icon` and the others) called without `approve`.
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
    /// The family it is for. A size of another family is refused (GFX-FAMILY-MISMATCH), and so are the pixels of an asset already installed for another family (GFX-FAMILY-REUSE).
    pub family: Family,
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
    /// Explicit steps. Omit for the family's default chain.
    #[serde(default)]
    pub steps: Option<Vec<Op>>,
    /// Canvas for families that have none.
    #[serde(default)]
    pub canvas: Option<[u32; 2]>,
}

/// `hoi4.image_validate`
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateParams {
    /// Artifact ids, project-relative image paths, or folders (ending in `/`) of DDS, TGA or PNG files.
    pub targets: Vec<String>,
    /// The family to check plain files as. Needed for paths and folders. An artifact always uses its own family,
    /// and naming another one for it is refused (`GFX-FAMILY-MISMATCH`). The full check set of the family always runs.
    #[serde(default)]
    pub family: Option<Family>,
    /// Canvas for families that have none.
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
    /// Answers to the rubric questions (`hoi4.image_families` lists them), as `{key: true|false}`.
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
