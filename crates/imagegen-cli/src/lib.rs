//! `hoi4-imagegen`: the command-line twin of the MCP server.
//!
//! Every command calls the same [`imagegen_mcp::Engine`] operation the matching `hoi4.image_*` tool calls, so
//! a command and its tool can never drift apart. `call` reaches all thirteen operations with JSON parameters;
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
    /// Compare the profiles with the installed game's file headers and propose a diff. Writes nothing.
    Calibrate,
    /// Show one profile or list them all.
    Profiles {
        /// Profile id, for example focus_icon.
        id: Option<String>,
    },
    /// Make a plan: a free dry run with the cost.
    Plan(PlanArgs),
    /// Execute a plan (paid, one call per variant).
    Generate(GenerateArgs),
    /// Run the deterministic checks on artifacts, files or folders.
    Validate(ValidateArgs),
    /// Install an approved artifact.
    Install(InstallArgs),
    /// Run any hoi4.image_* operation with JSON parameters (a string, or @file).
    Call {
        /// Tool name, for example hoi4.image_process.
        tool: String,
        /// JSON parameters.
        params: Option<String>,
    },
    /// The opt-in, unofficial ChatGPT login (only in builds with the plugin).
    #[cfg(feature = "unofficial-chatgpt-login")]
    ChatgptLogin {
        /// login, status or logout
        action: String,
    },
}

/// `plan` arguments.
#[derive(Args, Debug)]
pub struct PlanArgs {
    /// Category, for example focus_icon.
    #[arg(long)]
    pub category: String,
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
    /// Canvas WxH for categories with none.
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
    /// Profile id or category (needed for files).
    #[arg(long)]
    pub profile: Option<String>,
    /// Comma-separated checks.
    #[arg(long)]
    pub checks: Option<String>,
    /// Canvas WxH for categories with none.
    #[arg(long)]
    pub canvas: Option<String>,
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

/// Run an operation by tool name with JSON parameters.
pub fn dispatch(engine: &Engine, tool: &str, params: Value) -> Result<Value, EngineError> {
    let bad = |e: serde_json::Error| EngineError::invalid(format!("parameters for {tool}: {e}"));
    let params = if params.is_null() { json!({}) } else { params };
    match tool {
        "hoi4.image_status" => Ok(engine.status()),
        "hoi4.image_profiles" => engine.profiles_info(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_references" => engine.references(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_plan" => engine.plan(serde_json::from_value(params).map_err(bad)?).map(|p| serde_json::to_value(p).unwrap_or(Value::Null)),
        "hoi4.image_generate" => engine.generate(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_edit" => engine.edit(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_process" => engine.process(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_validate" => engine.validate(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_preview" => engine.preview(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_review" => engine.review(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_install" => engine.install(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_ledger" => engine.ledger_query(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_cancel" => engine.cancel(serde_json::from_value(params).map_err(bad)?),
        other => Err(EngineError::invalid(format!("unknown tool '{other}'; the tools are {}", imagegen_mcp::server::TOOL_NAMES.join(", ")))),
    }
}

fn parse_canvas(s: &str) -> Result<[u32; 2], EngineError> {
    let (w, h) = s.split_once(['x', 'X']).ok_or_else(|| EngineError::invalid("canvas must look like 100x88"))?;
    Ok([w.trim().parse().map_err(|_| EngineError::invalid("bad canvas width"))?, h.trim().parse().map_err(|_| EngineError::invalid("bad canvas height"))?])
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
            let v = engine.profiles_info(serde_json::from_value(json!({"calibrate": true})).map_err(|e| EngineError::invalid(e.to_string()))?)?;
            let text = calibrate_text(&v["calibration"]);
            Ok((v["calibration"].clone(), 0, Some(text)))
        }
        Command::Profiles { id } => Ok((engine.profiles_info(serde_json::from_value(json!({"id": id})).map_err(|e| EngineError::invalid(e.to_string()))?)?, 0, None)),
        Command::Plan(a) => {
            let mut refs = Vec::new();
            for r in &a.refs {
                let (path, role) = r.rsplit_once(':').filter(|(_, role)| role.ends_with("_anchor")).unwrap_or((r.as_str(), "style_anchor"));
                refs.push(json!({"path": path, "role": role}));
            }
            let params = json!({
                "category": a.category, "feature_slug": a.feature, "asset": a.asset, "brief": a.brief,
                "provider": a.provider, "model": a.model, "quality": a.quality, "style": a.style,
                "references": refs, "variants": a.variants, "budget_usd": a.budget,
                "canvas": a.canvas.as_deref().map(parse_canvas).transpose()?,
            });
            let v = dispatch(engine, "hoi4.image_plan", params)?;
            let text = plan_text(&v);
            Ok((v, 0, Some(text)))
        }
        Command::Generate(a) => Ok((dispatch(engine, "hoi4.image_generate", json!({"plan_id": a.plan_id, "idempotency_key": a.key, "approve_cost": a.approve_cost}))?, 0, None)),
        Command::Validate(a) => {
            let params = json!({
                "targets": a.targets, "profile": a.profile,
                "checks": a.checks.as_ref().map(|c| c.split(',').map(str::trim).collect::<Vec<_>>()),
                "canvas": a.canvas.as_deref().map(parse_canvas).transpose()?,
            });
            let v = dispatch(engine, "hoi4.image_validate", params)?;
            let code = i32::from(v["status"] == json!("fail"));
            let text = validate_text(&v);
            Ok((v, code, Some(text)))
        }
        Command::Install(a) => Ok((dispatch(engine, "hoi4.image_install", json!({"target": a.target, "approval_id": a.approval, "mode": a.mode, "on_exists": a.on_exists}))?, 0, None)),
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
        #[cfg(feature = "unofficial-chatgpt-login")]
        Command::ChatgptLogin { action } => {
            let text = provider_chatgpt_login::cli::run(action, &*engine.env, &*engine.clock).map_err(|e| EngineError::invalid(e))?;
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
    t.push_str(&format!("hoi4-imagegen {}  project {}\n\n", s["server"]["version"].as_str().unwrap_or("?"), s["project_root"].as_str().unwrap_or("?")));
    t.push_str("Providers (keys are read from environment variables only):\n");
    for p in s["providers"].as_array().into_iter().flatten() {
        let ok = p["configured"] == json!(true);
        t.push_str(&format!(
            "  {:<8} {}  default model {}  (tables checked {})\n",
            p["id"].as_str().unwrap_or("?"),
            if ok { "ready          ".to_string() } else { format!("set {}", p["key_env"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" or ")).unwrap_or_default()) },
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
        "Policy: vanilla art as references {}; per-call limit ${}\nChatGPT-login plugin: {}\n",
        if s["policy"]["allow_vanilla_references"] == json!(true) { "ALLOWED for this project" } else { "off (default)" },
        s["policy"]["max_usd_per_call"],
        s["chatgpt_login"]["state"].as_str().unwrap_or("?"),
    ));
    t
}

fn calibrate_text(c: &Value) -> String {
    if c["available"] != json!(true) {
        return format!("Calibration unavailable: {}", c["reason"].as_str().unwrap_or("unknown reason"));
    }
    let mut t = format!("Game {} (version {})\nHeader-only survey; nothing is written.\n\n", c["game_dir"].as_str().unwrap_or("?"), c["game_version"].as_str().unwrap_or("unknown"));
    t.push_str(&format!("{:<26} {:>6}  {:>9}  {:>7}  {}\n", "profile", "files", "profile", "share", "most common sizes / proposal"));
    for r in c["rows"].as_array().into_iter().flatten() {
        let top: Vec<String> = r["top"].as_array().into_iter().flatten().take(3).map(|s| format!("{}x{} ({})", s["size"][0], s["size"][1], s["files"])).collect();
        let size = format!("{}x{}", r["profile_size"][0], r["profile_size"][1]);
        let mut line = format!("{:<26} {:>6}  {:>9}  {:>6.1}%  {}", r["profile"].as_str().unwrap_or("?"), r["files"], size, r["share_at_profile_size"].as_f64().unwrap_or(0.0) * 100.0, top.join(", "));
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
        "Plan {} ({}, {} on {})\n  profile {} canvas {}x{}, generation size {}x{}, transparent {}\n  estimate ${:.4} for {} call(s): {}\n",
        p["plan_id"].as_str().unwrap_or("?"),
        p["asset_id"].as_str().unwrap_or("?"),
        p["model"].as_str().unwrap_or("?"),
        p["provider"].as_str().unwrap_or("?"),
        p["profile"]["id"].as_str().unwrap_or("?"),
        p["canvas"][0], p["canvas"][1], p["generation"]["width"], p["generation"]["height"], p["generation"]["transparent"],
        p["estimate"]["usd"].as_f64().unwrap_or(0.0), p["estimate"]["calls"], p["estimate"]["basis"].as_str().unwrap_or(""),
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

fn validate_text(v: &Value) -> String {
    let mut t = String::new();
    for r in v["results"].as_array().into_iter().flatten() {
        t.push_str(&format!("{}  {}  ({} error(s), {} warning(s))\n", r["status"].as_str().unwrap_or("?").to_uppercase(), r["target"].as_str().unwrap_or("?"), r["errors"], r["warnings"]));
        for f in r["findings"].as_array().into_iter().flatten() {
            t.push_str(&format!("    {} {}: {}\n", f["level"].as_str().unwrap_or("?"), f["code"].as_str().unwrap_or("?"), f["message"].as_str().unwrap_or("")));
        }
    }
    t.push_str(&format!("\n{} checked, {} failed. Deterministic checks only: composition and style need hoi4-imagegen call hoi4.image_review.", v["checked"], v["failed"]));
    t
}
