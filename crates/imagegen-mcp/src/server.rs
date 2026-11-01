//! The rmcp stdio server: eighteen tools named `hoi4.image_*`.
//!
//! Every tool runs on a blocking worker so a slow provider call never stalls the protocol loop, which is what
//! lets `hoi4.image_cancel` arrive while a generation is in flight. Failures are tool results with
//! `isError = true` and a structured `{error: {code, message, retry_class}}` body, not protocol errors.

use std::sync::Arc;

use base64::Engine as _;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use serde_json::Value;

use crate::api::*;
use crate::engine::{EResult, Engine};

/// Instructions shown to the client at start-up.
pub const INSTRUCTIONS: &str = "HOI4 graphical assets. Work one asset at a time: hoi4.image_status (what is configured), hoi4.image_families (every family's measured canvas, naming, checks and rubric), then the generation tool of the asset's group with a required family: hoi4.image_icon, hoi4.image_picture, hoi4.image_portrait, hoi4.image_flag, hoi4.image_unit or hoi4.image_interface. Called without approve it returns a free plan; with approve and an idempotency_key it plans and generates (one paid call per variant). The family is fixed from the plan on: hoi4.image_process (the family's own processing always runs and cannot be skipped or replaced: callers may add only the pre-steps chroma_key, crop_to_aspect, defringe and trim_to_subject; import needs a family and refuses another family's size or pixels), hoi4.image_validate (the family's full check set), hoi4.image_preview (contact sheet; look at the native-size panels), hoi4.image_review (rubric answers; an approval returns an approval_id) and hoi4.image_install (writes the DDS or TGA and a sprite handoff) all use the artifact's own family and take no family argument for it. Install refuses unless validation ran the full check set and passed, a review approved this exact file and the hashes are unchanged. Transparency must be proven by pixels, never by the prompt. Outputs are versioned and never overwritten; hoi4.image_ledger shows which references, settings and outputs belong together. Keys are read from environment variables only.";

/// The server.
#[derive(Clone)]
pub struct ImageServer {
    engine: Arc<Engine>,
    tool_router: ToolRouter<Self>,
}

impl ImageServer {
    /// Wrap an engine.
    pub fn new(engine: Arc<Engine>) -> Self {
        ImageServer { engine, tool_router: Self::tool_router() }
    }

    async fn run<F>(&self, f: F) -> Result<CallToolResult, ErrorData>
    where
        F: FnOnce(Arc<Engine>) -> EResult<Value> + Send + 'static,
    {
        let engine = self.engine.clone();
        match tokio::task::spawn_blocking(move || f(engine)).await {
            Ok(Ok(v)) => Ok(CallToolResult::structured(v)),
            Ok(Err(e)) => Ok(CallToolResult::structured_error(e.to_json())),
            Err(join) => Err(ErrorData::internal_error(format!("the worker for this tool failed: {join}"), None)),
        }
    }
}

#[tool_router]
impl ImageServer {
    #[tool(
        name = "hoi4.image_status",
        description = "Report what is configured: providers (key present or not, never the value), model tables with their retrieval date, the game reference, the profile source and hash, the ledger chain and the ChatGPT-login plugin state. Call this first.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn status(&self) -> Result<CallToolResult, ErrorData> {
        self.run(|e| Ok(e.status())).await
    }

    #[tool(
        name = "hoi4.image_families",
        description = "List every asset family or get one: group and tool, measured canvas and frames (vanilla 1.19.4.1 counts), accepted sizes, the mod's override of them, naming and folder, checks, review rubric questions, final file paths and the family's style sources: the tiers found with counts (pinned style, the mod's asset-folder copy, the mod's own assets, vanilla), the references a plan would send with the reason for each, the measured style statistics and the drift threshold. With calibrate = true, compare the families with the installed game's file headers and return the proposed diff (nothing is written).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn families(&self, Parameters(p): Parameters<FamiliesParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.families_info(p)).await
    }

    #[tool(
        name = "hoi4.image_references",
        description = "Manage reference images and project style profiles: list, add (with role and rights note), approve, and list, get or save style profiles whose references are pinned by SHA-256. Plans also pick style references automatically, mod first, then vanilla. Paradox art may be sent to a provider and its origin is recorded.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn references(&self, Parameters(p): Parameters<ReferencesParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.references(p)).await
    }

    #[tool(
        name = "hoi4.image_icon",
        description = "Make single-frame transparent icons: focus, idea_spirit, idea_concern, officer_corps_spirit, decision (and missions), decision_category_icon, state_modifier, special_project_icon, raid, mio_department, mio_trait, mio_policy, bop_side, medal, agency_upgrade, operation_icon, commander_trait, personal_trait, doctrine. family is required and its schema lists only this tool's families: it alone selects canvas, frames, naming, references, processing and validation, and is fixed from here on. Without approve it is a free dry run that returns a plan_id (prompt, references by hash, provider, model, size, cost, output paths). With approve = true and an idempotency_key it plans and generates in one call, one paid call per variant, each stored unchanged as a new versioned candidate. Another group's family is refused.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn image_icon(&self, Parameters(p): Parameters<IconRequest>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| {
            let (req, follow) = p.into_parts();
            e.group_request(IconRequest::GROUP, req, follow)
        })
        .await
    }

    #[tool(
        name = "hoi4.image_picture",
        description = "Make pictures and card art: report_event, news_event, decision_category_picture, operation_picture, technology, achievement. family is required and its schema lists only this tool's families: it alone selects canvas, frames, naming, references, processing and validation, and is fixed from here on. Without approve it is a free dry run that returns a plan_id (prompt, references by hash, provider, model, size, cost, output paths). With approve = true and an idempotency_key it plans and generates in one call, one paid call per variant, each stored unchanged as a new versioned candidate. Another group's family is refused.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn image_picture(&self, Parameters(p): Parameters<PictureRequest>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| {
            let (req, follow) = p.into_parts();
            e.group_request(PictureRequest::GROUP, req, follow)
        })
        .await
    }

    #[tool(
        name = "hoi4.image_portrait",
        description = "Make portraits: leader_portrait, commander_portrait, operative_portrait, scientist_portrait (cropped around a detected face to 156x210) and advisor_card (composited into the mod's advisor frame template). family is required and its schema lists only this tool's families: it alone selects canvas, frames, naming, references, processing and validation, and is fixed from here on. Without approve it is a free dry run that returns a plan_id (prompt, references by hash, provider, model, size, cost, output paths). With approve = true and an idempotency_key it plans and generates in one call, one paid call per variant, each stored unchanged as a new versioned candidate. Another group's family is refused.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn image_portrait(&self, Parameters(p): Parameters<PortraitRequest>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| {
            let (req, follow) = p.into_parts();
            e.group_request(PortraitRequest::GROUP, req, follow)
        })
        .await
    }

    #[tool(
        name = "hoi4.image_flag",
        description = "Make country flags: flag (the whole 82x52, 41x26 and 10x7 ladder of one tag and ideology). family is required and its schema lists only this tool's families: it alone selects canvas, frames, naming, references, processing and validation, and is fixed from here on. Without approve it is a free dry run that returns a plan_id (prompt, references by hash, provider, model, size, cost, output paths). With approve = true and an idempotency_key it plans and generates in one call, one paid call per variant, each stored unchanged as a new versioned candidate. Another group's family is refused.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn image_flag(&self, Parameters(p): Parameters<FlagRequest>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| {
            let (req, follow) = p.into_parts();
            e.group_request(FlagRequest::GROUP, req, follow)
        })
        .await
    }

    #[tool(
        name = "hoi4.image_unit",
        description = "Make frame-strip and unit art: building, faction_logo and agency_logo are generated as one icon and the tool builds the two-frame strip (selected or disabled second frame); counter_large, counter_small and division_emblem are import-only because their second frame is different art. family is required and its schema lists only this tool's families: it alone selects canvas, frames, naming, references, processing and validation, and is fixed from here on. Without approve it is a free dry run that returns a plan_id (prompt, references by hash, provider, model, size, cost, output paths). With approve = true and an idempotency_key it plans and generates in one call, one paid call per variant, each stored unchanged as a new versioned candidate. Another group's family is refused.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn image_unit(&self, Parameters(p): Parameters<UnitRequest>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| {
            let (req, follow) = p.into_parts();
            e.group_request(UnitRequest::GROUP, req, follow)
        })
        .await
    }

    #[tool(
        name = "hoi4.image_interface",
        description = "Make interface art: modifier_stat_icon, gui_texture and super_event (the mod defines their canvas). family is required and its schema lists only this tool's families: it alone selects canvas, frames, naming, references, processing and validation, and is fixed from here on. Without approve it is a free dry run that returns a plan_id (prompt, references by hash, provider, model, size, cost, output paths). With approve = true and an idempotency_key it plans and generates in one call, one paid call per variant, each stored unchanged as a new versioned candidate. Another group's family is refused.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn image_interface(&self, Parameters(p): Parameters<InterfaceRequest>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| {
            let (req, follow) = p.into_parts();
            e.group_request(InterfaceRequest::GROUP, req, follow)
        })
        .await
    }

    #[tool(
        name = "hoi4.image_generate",
        description = "Execute a plan returned by a group tool: one paid provider call per variant, each stored unchanged as a new versioned candidate (never overwritten). Repeating an idempotency_key returns the stored candidate instead of paying again. Candidates are raw: process, validate, preview, review and install them next.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn generate(&self, Parameters(p): Parameters<GenerateParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.generate(p)).await
    }

    #[tool(
        name = "hoi4.image_edit",
        description = "Refine a candidate with a provider edit: an instruction, an optional mask and extra references. Produces a new candidate whose lineage points at the original. Same idempotency and cost rules as generate.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn edit(&self, Parameters(p): Parameters<EditParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.edit(p)).await
    }

    #[tool(
        name = "hoi4.image_process",
        description = "Run the candidate's own family's processing: optional pre-steps from the caller (only chroma_key, crop_to_aspect, defringe, trim_to_subject), then the family's mandatory finishing, which the tool always appends and the caller cannot replace, shorten or skip (fit and flatten; the report-event card; the selected or disabled second frame of logos and buildings; the advisor card from the mod's template; the achievement's grey and not-eligible states on the template backgrounds; the face-detected portrait crop; the three flag sizes). Or import existing art with a required family (refused when its size belongs to another family or its pixels are an asset already installed for another); strip families also take one file per frame in import.frames. Writes a processed artifact with its companion images and records every step, the recipe id and version, with input and output hashes: validation and install refuse an artifact whose record lacks a required stage. Refuses to invent transparency: an opaque source cannot become a transparent icon.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn process(&self, Parameters(p): Parameters<ProcessParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.process(p)).await
    }

    #[tool(
        name = "hoi4.image_validate",
        description = "The family's full set of deterministic checks, never fewer (size, alpha, opaque canvas, ghosting, chroma fringe, margins, fill, fake checkerboard, frames, DDS or TGA layout) on artifacts (their own family), or on files and folders with a family, with stable finding codes. Records the result and the check-set hash for artifacts, which install requires. A pass says nothing about style or composition: that is hoi4.image_review.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn validate(&self, Parameters(p): Parameters<ValidateParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.validate(p)).await
    }

    #[tool(
        name = "hoi4.image_preview",
        description = "Write a contact sheet: native size on dark and light, an enlarged copy on dark, light and checker backgrounds, and the plan's references side by side. Returns the sheet as an image. Judge the native-size panels first.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn preview(&self, Parameters(p): Parameters<PreviewParams>) -> Result<CallToolResult, ErrorData> {
        let include = p.include_image.unwrap_or(true);
        let engine = self.engine.clone();
        match tokio::task::spawn_blocking(move || engine.preview_with_image(&p)).await {
            Ok(Ok((report, png))) => {
                let mut result = CallToolResult::structured(report);
                if include && png.len() <= 1_500_000 {
                    result.content.push(ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(&png), "image/png"));
                }
                Ok(result)
            }
            Ok(Err(e)) => Ok(CallToolResult::structured_error(e.to_json())),
            Err(join) => Err(ErrorData::internal_error(format!("the worker for this tool failed: {join}"), None)),
        }
    }

    #[tool(
        name = "hoi4.image_review",
        description = "Record the visual review of an exact file against the family's rubric. An approval needs a passing validation, a preview of this file and every rubric answer true, and returns the approval_id install requires. Use changes_requested or rejected otherwise.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn review(&self, Parameters(p): Parameters<ReviewParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.review(p)).await
    }

    #[tool(
        name = "hoi4.image_install",
        description = "Write the final DDS (uncompressed A8R8G8B8, one mip) or TGA files and the sprite snippet. Refuses unless validation passed, the review approved this exact file under the given approval_id, and the file, its raw candidate and its plan are unchanged. Never overwrites a file it did not install. mode handoff writes the sprite into gfx_handoff.md; register inserts it into the .gfx file.",
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = false)
    )]
    async fn install(&self, Parameters(p): Parameters<InstallParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.install(p)).await
    }

    #[tool(
        name = "hoi4.image_ledger",
        description = "Query the hash-chained provenance ledger: which references, settings and outputs belong together, the lineage of an artifact, and whether the chain verifies.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn ledger(&self, Parameters(p): Parameters<LedgerParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.ledger_query(p)).await
    }

    #[tool(
        name = "hoi4.image_cancel",
        description = "Cancel a running generation by idempotency key (providers with job ids are asked to stop), or settle a job a crashed run left behind: reconcile = true asks the provider what became of it and stores the image if it finished. Synchronous providers cannot be interrupted once the request is sent.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn cancel(&self, Parameters(p): Parameters<CancelParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.cancel(p)).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ImageServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("hoi4-imagegen-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Names of the eighteen tools, in documentation order.
pub const TOOL_NAMES: [&str; 18] = [
    "hoi4.image_status",
    "hoi4.image_families",
    "hoi4.image_references",
    "hoi4.image_icon",
    "hoi4.image_picture",
    "hoi4.image_portrait",
    "hoi4.image_flag",
    "hoi4.image_unit",
    "hoi4.image_interface",
    "hoi4.image_generate",
    "hoi4.image_edit",
    "hoi4.image_process",
    "hoi4.image_validate",
    "hoi4.image_preview",
    "hoi4.image_review",
    "hoi4.image_install",
    "hoi4.image_ledger",
    "hoi4.image_cancel",
];
