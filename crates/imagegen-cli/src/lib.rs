//! `hoi4-imagegen`: the command-line twin of the MCP server.
//!
//! Every command calls the same [`imagegen_mcp::Engine`] operation the matching `hoi4.image_*` tool calls, so
//! a command and its tool can never drift apart. `call` reaches every operation with JSON parameters;
//! the first-class commands (`doctor`, `calibrate`, `plan`, `generate`, `validate`, `install`) add readable output.
//!
//! Exit codes: 0 success, 1 a finding or a refused operation, 2 usage or an unreadable project.

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use imagegen_mcp::{Engine, EngineError};
use serde_json::{json, Value};

/// Command line.
#[derive(Parser, Debug)]
#[command(name = "hoi4-imagegen", version, about = "Plan, generate, validate and install Hearts of Iron IV graphical assets")]
pub struct Cli {
    /// The mod project folder.
    #[arg(long, global = true, env = "HOI4_IMAGEGEN_PROJECT_ROOT", default_value = ".")]
    pub project_root: PathBuf,
    /// Print JSON instead of a readable summary.
    #[arg(long, global = true)]
    pub json: bool,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// Commands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check the setup: providers and keys, game folder, profiles, ledger.
    Doctor,
    /// Compare the families with the installed game's file headers and propose a diff. Writes nothing.
    Calibrate,
    /// Show one family (measured spec, mod override, style sources) or list them all.
    Families {
        /// Family id, for example focus.
        family: Option<String>,
    },
    /// Make a plan: a free dry run with the cost. Runs the group tool of the family.
    Plan(PlanArgs),
    /// Execute a plan (paid, one call per variant).
    Generate(GenerateArgs),
    /// Run the deterministic checks on artifacts, files or folders.
    Validate(ValidateArgs),
    /// Install an approved artifact.
    Install(InstallArgs),
    /// Trace the sprites and textures the mod uses to their families and report reuse, missing and mis-sized art. Read only.
    Audit(AuditArgs),
    /// Run any hoi4.image_* operation with JSON parameters (a string, or @file).
    Call {
        /// Tool name, for example hoi4.image_process.
        tool: String,
        /// JSON parameters.
        params: Option<String>,
    },
    /// Sign in with your own ChatGPT account (opens the browser; shows the risk note once). No project needed.
    Login {
        /// Accept the risk note without the interactive question.
        #[arg(long)]
        accept_risk: bool,
    },
    /// Show whether you are signed in to ChatGPT. No project needed.
    LoginStatus,
    /// Print the risk note that `login` shows, so a setup tool can show it and then run `login --accept-risk`.
    LoginRiskNote,
    /// Sign out of ChatGPT and remove the stored tokens. No project needed.
    Logout,
    /// Same as `login`, `login-status` and `logout` (kept for version 0.1 scripts).
    #[command(hide = true)]
    ChatgptLogin {
        /// login, status or logout
        action: String,
    },
}

/// `plan` arguments.
#[derive(Args, Debug)]
pub struct PlanArgs {
    /// Family, for example focus.
    #[arg(long)]
    pub family: String,
    /// Feature folder name.
    #[arg(long)]
    pub feature: String,
    /// Asset name.
    #[arg(long)]
    pub asset: String,
    /// What to depict.
    #[arg(long)]
    pub brief: String,
    /// Provider id.
    #[arg(long)]
    pub provider: Option<String>,
    /// Model id.
    #[arg(long)]
    pub model: Option<String>,
    /// Quality tier.
    #[arg(long)]
    pub quality: Option<String>,
    /// Style profile name.
    #[arg(long)]
    pub style: Option<String>,
    /// Reference as path:role (roles: style_anchor, subject_anchor, geometry_anchor). Repeatable.
    #[arg(long = "ref")]
    pub refs: Vec<String>,
    /// Candidates to produce (1 to 4).
    #[arg(long)]
    pub variants: Option<u8>,
    /// Refuse the plan above this many USD.
    #[arg(long)]
    pub budget: Option<f64>,
    /// Canvas WxH for families with none.
    #[arg(long)]
    pub canvas: Option<String>,
}

/// `generate` arguments.
#[derive(Args, Debug)]
pub struct GenerateArgs {
    /// Plan id.
    #[arg(long)]
    pub plan_id: String,
    /// Idempotency key.
    #[arg(long)]
    pub key: String,
    /// Confirm a call above the per-call limit.
    #[arg(long)]
    pub approve_cost: bool,
}

/// `validate` arguments.
#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Artifact ids, files or folders (ending in /).
    pub targets: Vec<String>,
    /// Family to check plain files as (artifacts use their own).
    #[arg(long)]
    pub family: Option<String>,
    /// Canvas WxH for families with none.
    #[arg(long)]
    pub canvas: Option<String>,
}

/// `audit` arguments.
#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Allowlist JSON inside the project (default .hoi4-imagegen/audit-allowlist.json when it exists).
    #[arg(long)]
    pub allowlist: Option<String>,
    /// Only this family. Repeatable.
    #[arg(long = "family")]
    pub families: Vec<String>,
    /// Findings listed per code.
    #[arg(long)]
    pub limit: Option<u32>,
}

/// `install` arguments.
#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Processed artifact id.
    #[arg(long)]
    pub target: String,
    /// Approval id from an approving review.
    #[arg(long)]
    pub approval: String,
    /// handoff (default) or register.
    #[arg(long, default_value = "handoff")]
    pub mode: String,
    /// fail (default) or replace.
    #[arg(long, default_value = "fail")]
    pub on_exists: String,
}

/// The account commands, which need no project. `None` for every other command.
pub fn account_action(command: &Command) -> Option<(&str, bool)> {
    match command {
        Command::Login { accept_risk } => Some(("login", *accept_risk)),
        Command::LoginStatus => Some(("status", false)),
        Command::LoginRiskNote => Some(("risk-note", false)),
        Command::Logout => Some(("logout", false)),
        Command::ChatgptLogin { action } => Some((action.as_str(), false)),
        _ => None,
    }
}

/// Run an account command (sign in, status, sign out).
pub fn account(command: &Command, env: &dyn imagegen_core::config::EnvSource, clock: &dyn imagegen_core::hashing::Clock) -> Result<String, String> {
    let (action, accept) = account_action(command).ok_or("not an account command")?;
    provider_chatgpt_login::cli::run(action, env, clock, accept)
}

/// Run an operation by tool name with JSON parameters.
pub fn dispatch(engine: &Engine, tool: &str, params: Value) -> Result<Value, EngineError> {
    let bad = |e: serde_json::Error| EngineError::invalid(format!("parameters for {tool}: {e}"));
    let params = if params.is_null() { json!({}) } else { params };
    match tool {
        "hoi4.image_status" => Ok(engine.status()),
        "hoi4.image_families" => engine.families_info(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_icon" => group(engine, params, bad, |p: imagegen_mcp::api::IconRequest| (imagegen_mcp::api::IconRequest::GROUP, p.into_parts())),
        "hoi4.image_picture" => {
            group(engine, params, bad, |p: imagegen_mcp::api::PictureRequest| (imagegen_mcp::api::PictureRequest::GROUP, p.into_parts()))
        }
        "hoi4.image_portrait" => {
            group(engine, params, bad, |p: imagegen_mcp::api::PortraitRequest| (imagegen_mcp::api::PortraitRequest::GROUP, p.into_parts()))
        }
        "hoi4.image_flag" => group(engine, params, bad, |p: imagegen_mcp::api::FlagRequest| (imagegen_mcp::api::FlagRequest::GROUP, p.into_parts())),
        "hoi4.image_unit" => group(engine, params, bad, |p: imagegen_mcp::api::UnitRequest| (imagegen_mcp::api::UnitRequest::GROUP, p.into_parts())),
        "hoi4.image_interface" => {
            group(engine, params, bad, |p: imagegen_mcp::api::InterfaceRequest| (imagegen_mcp::api::InterfaceRequest::GROUP, p.into_parts()))
        }
        "hoi4.image_animation" => {
            group(engine, params, bad, |p: imagegen_mcp::api::AnimationRequest| (imagegen_mcp::api::AnimationRequest::GROUP, p.into_parts()))
        }
        "hoi4.image_references" => engine.references(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_generate" => engine.generate(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_edit" => engine.edit(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_process" => engine.process(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_validate" => engine.validate(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_preview" => engine.preview(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_review" => engine.review(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_install" => engine.install(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_audit" => engine.audit(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_ledger" => engine.ledger_query(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_cancel" => engine.cancel(serde_json::from_value(params).map_err(bad)?),
        other => Err(EngineError::invalid(format!("unknown tool '{other}'; the tools are {}", imagegen_mcp::server::TOOL_NAMES.join(", ")))),
    }
}

fn group<T: serde::de::DeserializeOwned>(
    engine: &Engine,
    params: Value,
    bad: impl Fn(serde_json::Error) -> EngineError,
    split: impl FnOnce(T) -> (imagegen_core::Group, (imagegen_core::plan::PlanRequest, imagegen_mcp::api::GroupFollowUp)),
) -> Result<Value, EngineError> {
    let (g, (req, follow)) = split(serde_json::from_value(params).map_err(bad)?);
    engine.group_request(g, req, follow)
}

fn parse_canvas(s: &str) -> Result<[u32; 2], EngineError> {
    let (w, h) = s.split_once(['x', 'X']).ok_or_else(|| EngineError::invalid("canvas must look like 100x88"))?;
    Ok([
        w.trim().parse().map_err(|_| EngineError::invalid("bad canvas width"))?,
        h.trim().parse().map_err(|_| EngineError::invalid("bad canvas height"))?,
    ])
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

/// Execute a parsed command against an open engine, writing to `out`. Returns the exit code.
pub fn execute(engine: &Engine, cli: &Cli, out: &mut dyn Write) -> i32 {
    let result: Result<(Value, i32, Option<String>), EngineError> = (|| match &cli.command {
        Command::Doctor => {
            let s = engine.status();
            let text = doctor_text(&s);
            let broken = s["ledger"]["chain_ok"] == json!(false);
            Ok((s, i32::from(broken), Some(text)))
        }
        Command::Calibrate => {
            let v = engine.families_info(serde_json::from_value(json!({"calibrate": true})).map_err(|e| EngineError::invalid(e.to_string()))?)?;
            let text = calibrate_text(&v["calibration"]);
            Ok((v["calibration"].clone(), 0, Some(text)))
        }
        Command::Families { family } => {
            Ok((engine.families_info(serde_json::from_value(json!({"family": family})).map_err(|e| EngineError::invalid(e.to_string()))?)?, 0, None))
        }
        Command::Plan(a) => {
            let mut refs = Vec::new();
            for r in &a.refs {
                let (path, role) = r.rsplit_once(':').filter(|(_, role)| role.ends_with("_anchor")).unwrap_or((r.as_str(), "style_anchor"));
                refs.push(json!({"path": path, "role": role}));
            }
            let params = json!({
                "family": a.family, "feature_slug": a.feature, "asset": a.asset, "brief": a.brief,
                "provider": a.provider, "model": a.model, "quality": a.quality, "style": a.style,
                "references": refs, "variants": a.variants, "budget_usd": a.budget,
                "canvas": a.canvas.as_deref().map(parse_canvas).transpose()?,
            });
            let fam: imagegen_core::Family = a.family.parse().map_err(EngineError::from)?;
            let v = dispatch(engine, fam.group().tool_name(), params)?;
            let text = plan_text(&v["plan"]);
            Ok((v, 0, Some(text)))
        }
        Command::Generate(a) => Ok((
            dispatch(engine, "hoi4.image_generate", json!({"plan_id": a.plan_id, "idempotency_key": a.key, "approve_cost": a.approve_cost}))?,
            0,
            None,
        )),
        Command::Validate(a) => {
            let params = json!({
                "targets": a.targets, "family": a.family,
                "canvas": a.canvas.as_deref().map(parse_canvas).transpose()?,
            });
            let v = dispatch(engine, "hoi4.image_validate", params)?;
            let code = i32::from(v["status"] == json!("fail"));
            let text = validate_text(&v);
            Ok((v, code, Some(text)))
        }
        Command::Install(a) => Ok((
            dispatch(engine, "hoi4.image_install", json!({"target": a.target, "approval_id": a.approval, "mode": a.mode, "on_exists": a.on_exists}))?,
            0,
            None,
        )),
        Command::Audit(a) => {
            let v = dispatch(
                engine,
                "hoi4.image_audit",
                json!({"allowlist": a.allowlist, "families": if a.families.is_empty() { Value::Null } else { json!(a.families) }, "limit": a.limit}),
            )?;
            let text = audit_text(&v);
            Ok((v, 0, Some(text)))
        }
        Command::Call { tool, params } => {
            let parsed = match params {
                None => Value::Null,
                Some(p) => {
                    let text = match p.strip_prefix('@') {
                        Some(file) => std::fs::read_to_string(file).map_err(|e| EngineError::invalid(format!("cannot read {file}: {e}")))?,
                        None => p.clone(),
                    };
                    serde_json::from_str(&text).map_err(|e| EngineError::invalid(format!("parameters are not valid JSON: {e}")))?
                }
            };
            Ok((dispatch(engine, tool, parsed)?, 0, None))
        }
        Command::Login { .. } | Command::LoginStatus | Command::LoginRiskNote | Command::Logout | Command::ChatgptLogin { .. } => {
            let text = account(&cli.command, &*engine.env, &*engine.clock).map_err(EngineError::invalid)?;
            Ok((json!({"result": text}), 0, Some(text)))
        }
    })();
    match result {
        Ok((value, code, text)) => {
            let body = match (cli.json, text) {
                (false, Some(t)) => t,
                _ => pretty(&value),
            };
            let _ = writeln!(out, "{body}");
            code
        }
        Err(e) => {
            let body = if cli.json { pretty(&e.to_json()) } else { format!("error [{}]: {}", e.code, e.message) };
            let _ = writeln!(out, "{body}");
            1
        }
    }
}

fn doctor_text(s: &Value) -> String {
    let mut t = String::new();
    t.push_str(&format!(
        "hoi4-imagegen {}  project {}\n\n",
        s["server"]["version"].as_str().unwrap_or("?"),
        s["project_root"].as_str().unwrap_or("?")
    ));
    t.push_str("Providers (keys are read from environment variables only):\n");
    for p in s["providers"].as_array().into_iter().flatten() {
        let ok = p["configured"] == json!(true);
        t.push_str(&format!(
            "  {:<8} {}  default model {}  (tables checked {})\n",
            p["id"].as_str().unwrap_or("?"),
            if ok {
                "ready          ".to_string()
            } else {
                format!(
                    "set {}",
                    p["key_env"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" or ")).unwrap_or_default()
                )
            },
            p["default_model"].as_str().unwrap_or("?"),
            p["tables_retrieved"].as_str().unwrap_or("?"),
        ));
    }
    for (id, why) in s["provider_problems"].as_object().into_iter().flatten() {
        t.push_str(&format!("  {id:<8} NOT AVAILABLE: {}\n", why.as_str().unwrap_or("")));
    }
    for w in s["warnings"].as_array().into_iter().flatten() {
        t.push_str(&format!("  warning: {}\n", w.as_str().unwrap_or("")));
    }
    t.push_str(&format!(
        "\nGame folder: {}\nProfiles: {} (kit file {}{})\nLedger: {}\n",
        match s["game"]["found"] == json!(true) {
            true => format!("{} (version {})", s["game"]["dir"].as_str().unwrap_or("?"), s["game"]["version"].as_str().unwrap_or("unknown")),
            false => "not found (set HOI4_GAME_DIR to enable calibration)".to_string(),
        },
        s["profiles"]["count"],
        s["profiles"]["kit_file"].as_str().unwrap_or("?"),
        if s["profiles"]["kit_file_present"] == json!(true) { ", read" } else { ", absent: built-in vanilla-measured defaults are used" },
        match (s["ledger"]["exists"] == json!(true), s["ledger"]["chain_ok"].as_bool()) {
            (false, _) => "none yet".to_string(),
            (true, Some(true)) => format!("{} entries, hash chain intact", s["ledger"]["entries"]),
            _ => format!("HASH CHAIN BROKEN at {}", s["ledger"]["first_bad"]),
        },
    ));
    t.push_str(&format!(
        "Policy: vanilla art as references is always allowed (D4 amended 2026-11-01); per-call limit ${}\nChatGPT login ({}): {}\n",
        s["policy"]["max_usd_per_call"],
        s["chatgpt_login"]["model"].as_str().unwrap_or("?"),
        s["chatgpt_login"]["state"].as_str().unwrap_or("?"),
    ));
    t
}

fn calibrate_text(c: &Value) -> String {
    if c["available"] != json!(true) {
        return format!("Calibration unavailable: {}", c["reason"].as_str().unwrap_or("unknown reason"));
    }
    let mut t = format!(
        "Game {} (version {})\nHeader-only survey; nothing is written.\n\n",
        c["game_dir"].as_str().unwrap_or("?"),
        c["game_version"].as_str().unwrap_or("unknown")
    );
    t.push_str(&format!("{:<26} {:>6}  {:>9}  {:>7}  {}\n", "profile", "files", "profile", "share", "most common sizes / proposal"));
    for r in c["rows"].as_array().into_iter().flatten() {
        let top: Vec<String> =
            r["top"].as_array().into_iter().flatten().take(3).map(|s| format!("{}x{} ({})", s["size"][0], s["size"][1], s["files"])).collect();
        let size = format!("{}x{}", r["profile_size"][0], r["profile_size"][1]);
        let mut line = format!(
            "{:<26} {:>6}  {:>9}  {:>6.1}%  {}",
            r["profile"].as_str().unwrap_or("?"),
            r["files"],
            size,
            r["share_at_profile_size"].as_f64().unwrap_or(0.0) * 100.0,
            top.join(", ")
        );
        if let Some(p) = r["proposed_canvas"].as_array() {
            line.push_str(&format!("  -> PROPOSE {}x{}", p[0], p[1]));
        }
        t.push_str(&line);
        t.push('\n');
        for n in r["notes"].as_array().into_iter().flatten().filter(|n| !n.as_str().unwrap_or("").contains("mixes several")) {
            t.push_str(&format!("    note: {}\n", n.as_str().unwrap_or("")));
        }
    }
    t
}

fn plan_text(p: &Value) -> String {
    let mut t = format!(
        "Plan {} ({}, {} on {})\n  family {} canvas {}x{}, generation size {}x{}, transparent {}\n  estimate ${:.4} for {} call(s): {}\n",
        p["plan_id"].as_str().unwrap_or("?"),
        p["asset_id"].as_str().unwrap_or("?"),
        p["model"].as_str().unwrap_or("?"),
        p["provider"].as_str().unwrap_or("?"),
        p["request"]["family"].as_str().unwrap_or("?"),
        p["canvas"][0],
        p["canvas"][1],
        p["generation"]["width"],
        p["generation"]["height"],
        p["generation"]["transparent"],
        p["estimate"]["usd"].as_f64().unwrap_or(0.0),
        p["estimate"]["calls"],
        p["estimate"]["basis"].as_str().unwrap_or(""),
    );
    t.push_str("  final files:\n");
    for f in p["paths"]["final_files"].as_array().into_iter().flatten() {
        t.push_str(&format!("    {}\n", f.as_str().unwrap_or("")));
    }
    for a in p["approvals"].as_array().into_iter().flatten() {
        t.push_str(&format!("  needs: {}\n", a.as_str().unwrap_or("")));
    }
    for w in p["warnings"].as_array().into_iter().flatten() {
        t.push_str(&format!("  warning: {}\n", w.as_str().unwrap_or("")));
    }
    t.push_str("\nNext: hoi4-imagegen generate --plan-id <id> --key <your key>\n");
    t
}

fn audit_text(v: &Value) -> String {
    let s = &v["summary"];
    let mut t = format!(
        "Audit of {}\n  game {}\n  {} sprites ({} definitions), {} traced uses, {} textures resolved, {} missing\n",
        v["project_root"].as_str().unwrap_or("?"),
        v["game_dir"].as_str().unwrap_or("not found"),
        s["sprites_indexed"],
        s["definitions"],
        s["uses"],
        s["textures_resolved"],
        s["textures_missing"],
    );
    if !v["allowlist"].is_null() {
        t.push_str(&format!("  allowlist {}\n", v["allowlist"]["path"].as_str().unwrap_or("?")));
    }
    t.push_str("\nUses per family:\n");
    for (family, c) in v["uses_by_family"].as_object().into_iter().flatten() {
        t.push_str(&format!(
            "  {:<26} {:>6} uses  {} defined  {} undefined  {} direct  {} texture missing\n",
            family, c["uses"], c["defined"], c["undefined"], c["direct_texture"], c["texture_missing"]
        ));
    }
    t.push_str("\nFindings:\n");
    for (code, g) in v["findings"].as_object().into_iter().flatten() {
        t.push_str(&format!("  {:<26} {:>6} ({})\n", code, g["count"], g["severity"].as_str().unwrap_or("?")));
    }
    for (code, g) in v["findings"].as_object().into_iter().flatten() {
        let list = g["findings"].as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            continue;
        }
        t.push_str(&format!("\n{code} ({} of {} listed)\n", g["shown"], g["count"]));
        for f in list.iter().take(10) {
            t.push_str(&format!("  {}\n", f["message"].as_str().unwrap_or("")));
        }
    }
    t.push_str(&format!(
        "\nFull detail: add --json. Checked {} textures for canvas and {} report cards for corners.",
        v["canvas_checked"], v["alpha_checked"]
    ));
    for n in v["notes"].as_array().into_iter().flatten() {
        t.push_str(&format!("\nnote: {}", n.as_str().unwrap_or("")));
    }
    t
}

fn validate_text(v: &Value) -> String {
    let mut t = String::new();
    for r in v["results"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "{}  {}  ({} error(s), {} warning(s))\n",
            r["status"].as_str().unwrap_or("?").to_uppercase(),
            r["target"].as_str().unwrap_or("?"),
            r["errors"],
            r["warnings"]
        ));
        for f in r["findings"].as_array().into_iter().flatten() {
            t.push_str(&format!(
                "    {} {}: {}\n",
                f["level"].as_str().unwrap_or("?"),
                f["code"].as_str().unwrap_or("?"),
                f["message"].as_str().unwrap_or("")
            ));
        }
    }
    t.push_str(&format!(
        "\n{} checked, {} failed. Deterministic checks only: composition and style need hoi4-imagegen call hoi4.image_review.",
        v["checked"], v["failed"]
    ));
    t
}
