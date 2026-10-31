//! The rmcp stdio server: thirteen tools named `hoi4.image_*`.
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
use imagegen_core::plan::PlanRequest;

/// Instructions shown to the client at start-up.
pub const INSTRUCTIONS: &str = "HOI4 graphical assets. Work one asset at a time through this loop: hoi4.image_status (what is configured), hoi4.image_profiles (canvas, rubric and evidence for the category), hoi4.image_plan (a free dry run with the cost), hoi4.image_generate (one paid call per variant), hoi4.image_process (deterministic crop, fit, alpha), hoi4.image_validate (deterministic checks), hoi4.image_preview (contact sheet; look at the native-size panels), hoi4.image_review (rubric answers; an approval returns an approval_id), hoi4.image_install (writes the DDS or TGA and a sprite handoff). Install refuses unless validation passed, a review approved this exact file and the hashes are unchanged. Transparency must be proven by pixels, never by the prompt. Outputs are versioned and never overwritten; hoi4.image_ledger shows which references, settings and outputs belong together. Keys are read from environment variables only.";

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
        name = "hoi4.image_profiles",
        description = "List category profiles or get one with its evidence, canvas, frames, margins, checks, final file paths and the review rubric questions. With calibrate = true, compare the profiles with the installed game's file headers and return the proposed diff (nothing is written).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn profiles(&self, Parameters(p): Parameters<ProfilesParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.profiles_info(p)).await
    }

    #[tool(
        name = "hoi4.image_references",
        description = "Manage reference images and project style profiles: list, add (with role and rights note), approve, and list, get or save style profiles whose references are pinned by SHA-256. Paradox art (installed game or the kit's vanilla shelf) is refused unless the project opted in.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn references(&self, Parameters(p): Parameters<ReferencesParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.references(p)).await
    }

    #[tool(
        name = "hoi4.image_plan",
        description = "Dry run, no spend: resolve the profile, prompt text, references (by hash), provider, model, generation size, cost estimate with its basis, output paths and the approvals needed. Returns a plan_id that hoi4.image_generate executes. The same inputs give the same plan.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn plan(&self, Parameters(p): Parameters<PlanRequest>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.plan(p).map(|pl| serde_json::to_value(pl).unwrap_or(Value::Null))).await
    }

    #[tool(
        name = "hoi4.image_generate",
        description = "Execute a plan: one paid provider call per variant, each stored unchanged as a new versioned candidate (never overwritten). Repeating an idempotency_key returns the stored candidate instead of paying again. Candidates are raw: process, validate, preview, review and install them next.",
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
        description = "Run the deterministic chain on a candidate (trim, fit with margin, crop to aspect, Lanczos resize, defringe, grayscale), or import existing art from another tool or by hand. Writes a processed artifact and records every step with input and output hashes. Refuses to invent transparency: an opaque source cannot become a transparent icon.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn process(&self, Parameters(p): Parameters<ProcessParams>) -> Result<CallToolResult, ErrorData> {
        self.run(move |e| e.process(p)).await
    }

    #[tool(
        name = "hoi4.image_validate",
        description = "Deterministic checks (size, alpha, opaque canvas, ghosting, chroma fringe, margins, fill, fake checkerboard, frames, DDS or TGA layout) on artifacts, files or folders, with stable finding codes. Records the result for artifacts, which install requires. A pass says nothing about style or composition: that is hoi4.image_review.",
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
        description = "Record the visual review of an exact file against the profile's rubric. An approval needs a passing validation, a preview of this file and every rubric answer true, and returns the approval_id install requires. Use changes_requested or rejected otherwise.",
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

/// Names of the thirteen tools, in documentation order.
pub const TOOL_NAMES: [&str; 13] = [
    "hoi4.image_status",
    "hoi4.image_profiles",
    "hoi4.image_references",
    "hoi4.image_plan",
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
