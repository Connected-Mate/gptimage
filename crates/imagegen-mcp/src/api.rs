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
    pub fn animation(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema(Group::Animation)
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
            /// Canvas `[width, height]` of one frame, for families whose size the mod defines (GUI textures, super events, and animated strips, which take the size of the element they sit on), and for technology pictures, which take the size of the tree item the technology gets: 146x54 (default, the 183x84 item of a technology that enables equipment or sub-units in the infantry, support, armour or air folders), 64x64 (the small 72x72 item of any other technology) or 285x56 (the 204x72 item of naval, artillery and electronics equipment). Refused for the others.
            #[serde(default)]
            pub canvas: Option<[u32; 2]>,
            /// GUI textures only: the kind of texture (`button`, `frame`, `panel` or `background`). It adds that kind's vanilla form to the prompt and picks vanilla references of that kind. Refused for the others.
            #[serde(default)]
            pub texture_kind: Option<imagegen_core::GuiTextureKind>,
            /// With your own `style_anchor` references: `replace` (default) sends only yours, `add` also sends the automatic style references.
            #[serde(default)]
            pub style_mode: Option<imagegen_core::plan::StyleMode>,
            /// The sub-family to draw references and the form note from (listed by hoi4.image_families). Inferred from the brief when omitted.
            #[serde(default)]
            pub subfamily: Option<String>,
            /// Style pack to apply (`.hoi4-imagegen/packs/<name>/pack.toml`): its references for this family are sent as style anchors, its look line and form override go into the prompt, and its canvas and processing switches apply. Without it the project's default pack applies; `none` turns that off. A family the pack does not cover uses the normal style tiers.
            #[serde(default)]
            pub style_pack: Option<String>,
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
                    texture_kind: self.texture_kind,
                    style_mode: self.style_mode,
                    subfamily: self.subfamily,
                    style_pack: self.style_pack,
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
group_request!(
    /// `hoi4.image_animation`: animated multi-frame sprites (`frameAnimatedSpriteType`).
    AnimationRequest, Group::Animation, "family_schemas::animation"
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
    /// List saved style packs and the project's default pack.
    PackList,
    /// Read one style pack.
    PackGet,
    /// Save a style pack; every reference is pinned by SHA-256.
    PackSave,
    /// Check a saved style pack: pinned hashes, switches, and per family the references' sizes and how well they agree
    /// with each other (the review's pack-match check, left one out).
    PackValidate,
    /// Make a style pack from a mod's own art in one step: scan its `gfx/` folders per family, drop near-copies of
    /// vanilla and placeholders, rank by distance from vanilla's style, pick a diverse set, detect the canvas, draft
    /// the look, form and processing switches, write contact sheets for review and save the pack.
    PackDerive,
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
    /// Style or pack name (`style_get`, `pack_get`, `pack_validate`).
    #[serde(default)]
    pub name: Option<String>,
    /// Style profile to save (`style_save`). Reference hashes are filled in for you.
    #[serde(default)]
    pub style: Option<StyleProfile>,
    /// Replace an existing style or pack; the old file is backed up (`style_save`, `pack_save`).
    #[serde(default)]
    pub replace: Option<bool>,
    /// Style pack to save (`pack_save`). Reference hashes are filled in for you.
    #[serde(default)]
    pub pack: Option<imagegen_core::pack::StylePack>,
    /// Folder of the mod whose art the pack is derived from (`pack_derive`); the project itself when absent. Only
    /// image files under its `gfx/` folder are read.
    #[serde(default)]
    pub from_mod: Option<String>,
    /// Families to derive (`pack_derive`); every family it covers when absent. Other families of a replaced pack
    /// are kept.
    #[serde(default)]
    pub families: Option<Vec<imagegen_core::family::Family>>,
    /// References per family (`pack_derive`), 1 to 16, default 8.
    #[serde(default)]
    pub picks: Option<usize>,
    /// Swaps after review (`pack_derive`): `focus:3=a2` puts alternate 2 in place of pick 3, `focus:3=<path in the
    /// mod>` a file of the family.
    #[serde(default)]
    pub swap: Option<Vec<String>>,
    /// Picks to leave out after review (`pack_derive`): `focus:3`.
    #[serde(default)]
    pub drop: Option<Vec<String>>,
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
    /// Project-relative path of the image. Give this or `frames`.
    #[serde(default)]
    pub path: Option<String>,
    /// For a family that is a strip of frames (counters, logos, buildings): one project-relative file per frame, in order.
    /// Each must be exactly one accepted frame size. They are joined into the strip. Instead of `path`.
    #[serde(default)]
    pub frames: Option<Vec<String>>,
    /// The family it is for. A size of another family is refused (GFX-FAMILY-MISMATCH), and so are the pixels of an asset already installed for another family (GFX-FAMILY-REUSE).
    pub family: Family,
    /// Feature folder name.
    pub feature_slug: String,
    /// Asset name.
    pub asset: String,
    /// Style pack whose canvas and processing switches apply to the imported art. Without it the project's default
    /// pack applies when it covers the family; `none` turns that off.
    #[serde(default)]
    pub style_pack: Option<String>,
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
    /// Optional pre-steps that prepare the source: only `chroma_key`, `crop_to_aspect`, `defringe` and `trim_to_subject`
    /// are accepted. They run first. The family's own processing (fit, flatten, report card, frame strip, advisor card,
    /// achievement states, portrait crop, flag sizes...) always runs after them and cannot be replaced, skipped or
    /// shortened: any other step, or an empty list, is refused. Omit for no pre-steps.
    #[serde(default, alias = "steps")]
    pub pre_steps: Option<Vec<Op>>,
    /// Canvas for families that have none.
    #[serde(default)]
    pub canvas: Option<[u32; 2]>,
    /// Portrait families only: crop `[left, top, right, bottom]` in source pixels, used instead of face detection.
    #[serde(default)]
    pub portrait_crop: Option<[i64; 4]>,
    /// `building` only: the candidate of the ruined state for frame 2 (vanilla landmark strips show the ruined building
    /// there). Make it with `hoi4.image_edit` on the first candidate, for example "the same landmark in ruins, burning".
    #[serde(default)]
    pub second_frame_candidate: Option<String>,
    /// `counter_large` and `counter_small` only: the NATO symbol drawn in frame 2, as a preset (`infantry`,
    /// `motorized`, `mechanized`, `light_armor`, `medium_armor`, `heavy_armor`, `modern_armor`, `super_heavy_armor`,
    /// `artillery`, `motorized_artillery`, `anti_tank`, `motorized_anti_tank`, `anti_air`, `motorized_anti_air`,
    /// `cavalry`, `recon`, `engineer`, `mountain`, `paratroop`, `rocket_artillery`, `support`) or marks, joined by `+`
    /// (for example `infantry+support` or `cross+wheels`). Omit it to infer the symbol from the asset name (the
    /// sub-unit key); an infantry cross when nothing matches.
    #[serde(default)]
    pub counter_symbol: Option<String>,
    /// `technology` only: stamp a tier numeral (I to X, from `tier` 1 to 10) on the finished picture, and with `gem`
    /// the blue gem vanilla puts before it on battleship, cruiser and carrier pictures. Vanilla bakes these marks into
    /// the texture (no GUI layer or sprite draws them), so the overlay is part of the processed pixels: vanilla's serif
    /// glyphs, gold and shadow, in vanilla's place (top right) unless `position` gives the badge's top-left corner.
    #[serde(default)]
    pub overlay: Option<imagegen_core::recipes::tech_overlay::TechOverlay>,
    /// `bop_side` only: also make the weakened pair vanilla draws for a bad side: the same emblem cracked, grey with no glow and
    /// in colour with a dull red glow. The standard and improved stages are always made. Installed as
    /// `bop_<name>_bad_side` and `bop_<name>_bad_improved_side` with the sprites `GFX_bop_<name>_bad_side` and
    /// `GFX_bop_<name>_bad_improved_side`.
    #[serde(default)]
    pub cracked_stage: Option<bool>,
    /// `anim_blend_keyframes` only: the candidates of frames 2 onward, in order. Make each with `hoi4.image_edit` on the
    /// base candidate (the one being processed), so it keeps the base's framing; all frames are then fitted with one
    /// shared crop and scale.
    #[serde(default)]
    pub frame_candidates: Option<Vec<String>>,
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
    /// Verdict. An approval needs a passing validation, a preview of this exact file with the vanilla side-by-side
    /// sheet, a passing measured vanilla-match check (or a recorded override) and every rubric answer true.
    pub verdict: Verdict,
    /// Who reviewed: a person's name or an agent's name.
    pub reviewer: String,
    /// Answers to the rubric questions (`hoi4.image_families` lists them), as `{key: true|false}`.
    #[serde(default)]
    pub answers: Option<Value>,
    /// Notes for the next round.
    #[serde(default)]
    pub notes: Option<String>,
    /// Approve although the measured vanilla-match check fails or cannot run (a deliberate mod style, or no
    /// installed game). Needs `override_reason`. The override and its reason are recorded in the ledger and the review.
    #[serde(default)]
    pub override_vanilla_check: Option<bool>,
    /// Why the art may differ from vanilla (10 to 1000 characters). Required with `override_vanilla_check`.
    #[serde(default)]
    pub override_reason: Option<String>,
}

/// How an install treats the `.gfx` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
    /// Write the sprite snippet into `gfx_handoff.md` and leave `.gfx` files to the main agent (kit default).
    #[default]
    Handoff,
    /// Insert the sprite entries into the suggested `.gfx` file with the lossless parser (comments, layout, line endings and encoding are kept).
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
    /// Retire an uncertain idempotency key: a generate or edit request was cut off (the process was killed) and the same key is
    /// refused with `uncertain_previous_attempt`. The ledger records the abandonment with the time and the warning that the first
    /// attempt may have been charged, and the key (or a new one) can be used again. Refused for a key whose attempt completed.
    #[serde(default)]
    pub abandon: Option<bool>,
}

/// `hoi4.image_set`
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    /// The focus tree file, relative to the mod (for example `common/national_focus/GER.txt`).
    pub focus_tree: String,
    /// Feature folder of the set's assets (default: the file's name as snake_case).
    #[serde(default)]
    pub feature_slug: Option<String>,
    /// Only the icons the game cannot show yet: an undefined sprite or a missing texture (default true).
    #[serde(default)]
    pub only_missing: Option<bool>,
    /// Style pack every icon of the set uses.
    #[serde(default)]
    pub style_pack: Option<String>,
    /// Style anchors every icon of the set sends, for one shared look (for example the set's first approved icon).
    #[serde(default)]
    pub references: Option<Vec<ReferenceInput>>,
    /// Provider for every icon.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model for every icon.
    #[serde(default)]
    pub model: Option<String>,
    /// Quality for every icon.
    #[serde(default)]
    pub quality: Option<String>,
    /// Generate every planned icon (one paid call each). Needs `idempotency_key`.
    #[serde(default)]
    pub generate: Option<bool>,
    /// Key prefix: each icon is generated under `<idempotency_key>-<asset>`, so a rerun repeats nothing that finished.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Confirm that calls above the project's per-call cost limit are intended.
    #[serde(default)]
    pub approve_cost: Option<bool>,
}

/// `hoi4.image_audit`
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditParams {
    /// Project-relative path of an allowlist JSON file that records intentional reuse. Default:
    /// `.hoi4-imagegen/audit-allowlist.json` when it exists. The file is
    /// `{"aliases": [{"sprite" or "texture": ..., "families": [...], "reason": ...}], "no_shine": [sprite names],
    /// "missing_textures": [sprite names or texture paths]}`.
    #[serde(default)]
    pub allowlist: Option<String>,
    /// Only report these families (ids as in `hoi4.image_families`). Default: all.
    #[serde(default)]
    pub families: Option<Vec<Family>>,
    /// Findings listed per code (default 25, at most 5000; a reuse finding lists at most 100 uses and reports uses_total). Counts are always complete.
    #[serde(default)]
    pub limit: Option<u32>,
}
