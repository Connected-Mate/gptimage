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

/// Commands. Parsed once per run, so the size of the largest variant does not matter.
#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)]
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
    /// Style packs: a mod's house style across families (`.hoi4-imagegen/packs/<name>/pack.toml`).
    #[command(subcommand)]
    Pack(PackCommand),
    /// Run any hoi4.image_* operation with JSON parameters (a string, or @file).
    Call {
        /// Tool name, for example hoi4.image_process.
        tool: String,
        /// JSON parameters.
        params: Option<String>,
    },
    /// Sign in with your own ChatGPT account: opens the browser, and signing in accepts the risk note. No project needed.
    Login {
        /// Kept for old scripts: there is no question any more, so it does nothing.
        #[arg(long)]
        accept_risk: bool,
    },
    /// Show whether you are signed in to ChatGPT, and whether the server revoked the sign-in. No project needed.
    LoginStatus {
        /// Also ask the server (one token refresh, only when the token is near expiry; never a generation).
        #[arg(long)]
        check: bool,
    },
    /// Print the risk note that `login` shows.
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

/// `pack` subcommands.
#[derive(Subcommand, Debug)]
pub enum PackCommand {
    /// Build a pack from a folder with one sub-folder of reference images per family id (`decision/`,
    /// `gui_texture/`...). Each may hold `references.json` (a `references` list of `{ref}` objects, optionally with
    /// `texture_kind`) and `look.txt`; a top-level `gallery-plan.json` may give the looks. The images are copied into
    /// the mod under the pack's folder and pinned by SHA-256.
    Import {
        /// Pack name (snake_case).
        name: String,
        /// The folder to read.
        #[arg(long)]
        from: PathBuf,
        /// The family of images listed by a top-level manifest.json or references.json (for example focus).
        #[arg(long)]
        top_family: Option<String>,
        /// Import over an existing pack (its file is backed up; each entry keeps its form, canvas and switches).
        #[arg(long)]
        replace: bool,
    },
    /// Make a pack from a mod's own art in one step: per family, drop near-copies of vanilla and placeholders, rank
    /// by distance from vanilla's style, pick a diverse set, detect the canvas, draft the look, form and switches, and
    /// write a contact sheet per family for review. Swap picks after review and run it again with --replace.
    Derive {
        /// Pack name (snake_case).
        name: String,
        /// The mod folder to read (the one holding gfx/); the project itself when left out.
        #[arg(long)]
        from_mod: Option<PathBuf>,
        /// Only these families, comma-separated (for example focus,decision). Other families of a replaced pack are kept.
        #[arg(long, value_delimiter = ',')]
        families: Vec<String>,
        /// References per family (1 to 16, default 8).
        #[arg(long)]
        picks: Option<usize>,
        /// Put an alternate (or a file of the mod) in place of a pick: focus:3=a2, or focus:3=gfx/interface/goals/x.dds. Repeatable.
        #[arg(long)]
        swap: Vec<String>,
        /// Leave a pick out: focus:3. Repeatable.
        #[arg(long)]
        drop: Vec<String>,
        /// Derive over an existing pack (its file is backed up).
        #[arg(long)]
        replace: bool,
    },
    /// List the saved packs and the project's default.
    List,
    /// Print one pack.
    Get {
        /// Pack name.
        name: String,
    },
    /// Check a pack: pinned hashes, switches, reference sizes and how well each family's references agree.
    Validate {
        /// Pack name.
        name: String,
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
    /// GUI textures only: button, frame, panel or background.
    #[arg(long)]
    pub texture_kind: Option<String>,
    /// Style pack to apply (`none` turns the project default off).
    #[arg(long)]
    pub style_pack: Option<String>,
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
        Command::LoginStatus { check } => Some((if *check { "status-check" } else { "status" }, false)),
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
        "hoi4.image_set" => engine.set(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_ledger" => engine.ledger_query(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_cancel" => engine.cancel(serde_json::from_value(params).map_err(bad)?),
        "hoi4.image_login" => engine.login(),
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
            let mut s = engine.status();
            // the engine's status already carries the route's recorded state (`chatgpt_login.route`)
            note_login_revocation(&mut s, &*engine.env);
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
                "texture_kind": a.texture_kind,
                "style_pack": a.style_pack,
            });
            let fam: imagegen_core::Family = a.family.parse().map_err(EngineError::from)?;
            let v = dispatch(engine, fam.group().tool_name(), params)?;
            let text = plan_text(&v["plan"]);
            Ok((v, 0, Some(text)))
        }
        Command::Pack(cmd) => match cmd {
            PackCommand::Import { name, from, top_family, replace } => {
                let top = top_family.as_deref().map(str::parse::<imagegen_core::Family>).transpose().map_err(EngineError::from)?;
                let v = engine.import_pack_folder(name, from, top, *replace)?;
                let ok = v["validation"]["ok"] == json!(true);
                Ok((v, i32::from(!ok), None))
            }
            PackCommand::Derive { name, from_mod, families, picks, swap, drop, replace } => {
                let families =
                    families.iter().map(|f| f.trim().parse::<imagegen_core::Family>()).collect::<Result<Vec<_>, _>>().map_err(EngineError::from)?;
                let o = imagegen_mcp::engine::derive::DeriveOptions {
                    from_mod: from_mod.clone(),
                    families,
                    picks: *picks,
                    swap: swap.clone(),
                    drop: drop.clone(),
                    replace: *replace,
                };
                let v = engine.derive_pack(name, &o)?;
                let ok = v["validation"]["ok"] == json!(true);
                Ok((v, i32::from(!ok), None))
            }
            PackCommand::List => Ok((dispatch(engine, "hoi4.image_references", json!({"action": "pack_list"}))?, 0, None)),
            PackCommand::Get { name } => Ok((dispatch(engine, "hoi4.image_references", json!({"action": "pack_get", "name": name}))?, 0, None)),
            PackCommand::Validate { name } => {
                let v = dispatch(engine, "hoi4.image_references", json!({"action": "pack_validate", "name": name}))?;
                let ok = v["ok"] == json!(true);
                Ok((v, i32::from(!ok), None))
            }
        },
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
        Command::Login { .. } | Command::LoginStatus { .. } | Command::LoginRiskNote | Command::Logout | Command::ChatgptLogin { .. } => {
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

/// When the server revoked the ChatGPT sign-in, say so in the doctor's data and text (the engine's own status only
/// looks at whether tokens exist). Reads the recorded state only; no network call.
fn note_login_revocation(s: &mut Value, env: &dyn imagegen_core::config::EnvSource) {
    let Some(store) = provider_chatgpt_login::store::CredentialStore::for_environment(env).ok() else { return };
    let state = provider_chatgpt_login::route::Route::new(store.state_dir()).state();
    if let (Some(msg), Some(r)) = (state.revoked_message(), state.revoked) {
        s["chatgpt_login"]["state"] = json!(msg);
        s["chatgpt_login"]["signed_in"] = json!(false);
        s["chatgpt_login"]["revoked"] = json!({"at": r.at, "code": r.code, "status": r.status});
    }
}

/// The ChatGPT route's recorded state, as `doctor` gets it from the engine's status (tests build it from the
/// environment alone). The shape is `provider_chatgpt_login::route::report`. No request is made.
#[cfg(test)]
fn note_route_state(s: &mut Value, env: &dyn imagegen_core::config::EnvSource, now: i64) {
    use provider_chatgpt_login::route::{configured_concurrency, report, Route};
    let Some(store) = provider_chatgpt_login::store::CredentialStore::for_environment(env).ok() else { return };
    s["chatgpt_login"]["route"] = report(&Route::new(store.state_dir()).state(), configured_concurrency(env, None), now);
}

fn route_text(r: &Value) -> Option<String> {
    if r.is_null() {
        return None;
    }
    let mut t = String::from("ChatGPT route (recorded state, no request made):\n");
    t.push_str(&format!("  sign-in revoked by the server: {}\n", if r["revoked"] == json!(true) { "YES" } else { "no" }));
    let (configured, effective) = (r["concurrency"]["configured"].as_u64().unwrap_or(1), r["concurrency"]["effective"].as_u64().unwrap_or(1));
    if effective < configured {
        t.push_str(&format!("  requests at once: {effective} (configured {configured})\n"));
    } else {
        t.push_str(&format!("  requests at once: up to {configured} on this machine, each slot at least 20 s apart\n"));
    }
    if let Some(m) = r["fallback"]["message"].as_str() {
        t.push_str(&format!("  warning: {m}\n"));
    }
    if let Some(m) = r["model_access_unverified"].as_object() {
        t.push_str(&format!(
            "  \"Unable to verify model access\" answers: {} so far, the last at {} ({} ago); a slot waits {} s after one, two within {} min drop the route to one at a time for {} min
",
            m["count"],
            m["last_at"].as_str().unwrap_or("?"),
            ago(m["last_ago_secs"].as_i64().unwrap_or(0)),
            m["pause_secs"],
            m["window_secs"].as_i64().unwrap_or(0) / 60,
            m["hold_secs"].as_i64().unwrap_or(0) / 60
        ));
        for p in m["slot_pauses"].as_array().into_iter().flatten() {
            t.push_str(&format!(
                "  slot {} is paused until {} ({} left)
",
                p["slot"],
                p["until"].as_str().unwrap_or("?"),
                ago(p["remaining_secs"].as_i64().unwrap_or(0))
            ));
        }
    }
    match r["last_request_at"].as_str() {
        Some(at) => t.push_str(&format!("  last request: {at} ({} ago)\n", ago(r["last_request_ago_secs"].as_i64().unwrap_or(0)))),
        None => t.push_str("  last request: none recorded on this machine\n"),
    }
    let refused = r["refused_efforts"].as_array().cloned().unwrap_or_default();
    if refused.is_empty() {
        t.push_str("  refused reasoning efforts: none remembered (high is tried first)\n");
    } else {
        let list: Vec<String> = refused
            .iter()
            .map(|e| {
                format!(
                    "{} (skipped until {}, {} left)",
                    e["effort"].as_str().unwrap_or("?"),
                    e["skipped_until"].as_str().unwrap_or("?"),
                    ago(e["remaining_secs"].as_i64().unwrap_or(0))
                )
            })
            .collect();
        t.push_str(&format!("  refused reasoning efforts, skipped for now: {}\n", list.join("; ")));
    }
    if let Some(n) = r["expired_refusals"].as_u64().filter(|n| *n > 0) {
        t.push_str(&format!("  {n} older refusal(s) have expired and will be tried again\n"));
    }
    if let Some(b) = r["rate_limit"].as_object() {
        t.push_str(&format!(
            "  rate limit backoff: {} answer(s) of 429 in a row, no request before {}\n",
            b["failures"],
            b["retry_after"].as_str().unwrap_or("?")
        ));
    }
    Some(t)
}

/// Seconds as a short phrase: `45 s`, `12 min`, `3 h 5 min`.
fn ago(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        0..=89 => format!("{secs} s"),
        90..=5399 => format!("{} min", (secs + 30) / 60),
        _ => format!("{} h {} min", secs / 3600, (secs % 3600) / 60),
    }
}

fn doctor_text(s: &Value) -> String {
    let mut t = String::new();
    t.push_str(&format!(
        "hoi4-imagegen {}  project {}\n\n",
        s["server"]["version"].as_str().unwrap_or("?"),
        s["project_root"].as_str().unwrap_or("?")
    ));
    t.push_str("Providers (keys are read from environment variables or HOI4 Mod Setup's credential store):\n");
    for p in s["providers"].as_array().into_iter().flatten() {
        let id = p["id"].as_str().unwrap_or("?");
        let keys = p["key_env"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" or ")).unwrap_or_default();
        let state = if p["configured"] == json!(true) {
            "ready".to_string()
        } else if keys.is_empty() {
            // local servers need an address, not a key
            format!("set providers.{id}.endpoint")
        } else {
            format!("set {keys}")
        };
        t.push_str(&format!(
            "  {:<17} {}  default model {}  (tables checked {}{})\n",
            id,
            state,
            p["default_model"].as_str().unwrap_or("?"),
            p["tables_retrieved"].as_str().unwrap_or("?"),
            if p["live_verified"] == json!(true) { "" } else { "; not live-verified" },
        ));
    }
    for (id, why) in s["provider_problems"].as_object().into_iter().flatten() {
        t.push_str(&format!("  {id:<17} NOT AVAILABLE: {}\n", why.as_str().unwrap_or("")));
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
    if let Some(r) = route_text(&s["chatgpt_login"]["route"]) {
        t.push_str(&r);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use imagegen_core::config::MapEnv;
    use imagegen_providers::types::{CancelToken, RealSleeper};

    #[test]
    fn the_doctor_reports_a_server_revoked_sign_in_from_the_recorded_state_alone() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new().prefix("dr").tempdir_in(base).unwrap();
        let file = dir.path().join("auth.json").display().to_string();
        let env = MapEnv::from_pairs(&[(provider_chatgpt_login::store::PATH_ENV, file.as_str())]);
        let route = provider_chatgpt_login::route::Route::new(provider_chatgpt_login::store::CredentialStore::at(&file).state_dir());

        // SYNTHETIC engine status: the engine alone believes the user is signed in
        let engine_status = || json!({"chatgpt_login": {"model": "gpt-6-luna at high reasoning effort", "state": "signed in; the default local provider", "signed_in": true}});
        let mut s = engine_status();
        note_login_revocation(&mut s, &env);
        assert_eq!(s, engine_status(), "nothing recorded: the doctor is unchanged");

        route.acquire(&CancelToken::new(), &RealSleeper).unwrap().record_revoked(1_793_000_000, 401, b"token_revoked", "generation");
        let mut s = engine_status();
        note_login_revocation(&mut s, &env);
        assert_eq!(s["chatgpt_login"]["signed_in"], json!(false));
        assert_eq!(s["chatgpt_login"]["revoked"]["code"], json!("token_revoked"));
        let text = doctor_text(&s);
        assert!(text.contains("signed out by the server (token revoked at 2026-"), "{text}");
        assert!(text.contains("run `hoi4-imagegen login` to sign in again"), "{text}");
    }

    fn work_env(prefix: &str) -> (tempfile::TempDir, MapEnv, provider_chatgpt_login::route::Route) {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new().prefix(prefix).tempdir_in(base).unwrap();
        let file = dir.path().join("auth.json").display().to_string();
        let env = MapEnv::from_pairs(&[(provider_chatgpt_login::store::PATH_ENV, file.as_str())]);
        let route = provider_chatgpt_login::route::Route::new(provider_chatgpt_login::store::CredentialStore::at(&file).state_dir());
        (dir, env, route)
    }

    #[test]
    fn the_doctor_shows_the_recorded_route_state_and_makes_no_request() {
        use provider_chatgpt_login::route::{RateLimit, RefusedEffort};
        let (_dir, env, route) = work_env("dz");
        let now = 1_793_000_000;
        // nothing recorded
        let mut s = json!({"chatgpt_login": {"state": "signed in"}});
        note_route_state(&mut s, &env, now);
        let text = route_text(&s["chatgpt_login"]["route"]).unwrap();
        assert!(text.contains("sign-in revoked by the server: no"), "{text}");
        assert!(text.contains("last request: none recorded"), "{text}");
        assert!(text.contains("refused reasoning efforts: none remembered"), "{text}");

        // SYNTHETIC state: high and medium refused recently, low long ago, a request two minutes ago, a backoff
        let guard = route.acquire(&CancelToken::new(), &RealSleeper).unwrap();
        guard
            .update(|st| {
                st.last_request_epoch = Some(now - 120);
                st.refused_efforts = vec![
                    RefusedEffort { effort: "high".into(), until_epoch: now + 3 * 3600 + 300 },
                    RefusedEffort { effort: "medium".into(), until_epoch: now + 600 },
                    RefusedEffort { effort: "low".into(), until_epoch: now - 5 },
                ];
                st.rate_limit = Some(RateLimit { failures: 2, retry_after_epoch: now + 45 });
            })
            .unwrap();
        drop(guard);
        let mut s = json!({"chatgpt_login": {"state": "signed in"}});
        note_route_state(&mut s, &env, now);
        let r = &s["chatgpt_login"]["route"];
        assert_eq!(r["revoked"], json!(false));
        assert_eq!(r["last_request_ago_secs"], json!(120));
        assert_eq!(r["refused_efforts"].as_array().unwrap().len(), 2, "the expired refusal is not listed as active");
        assert_eq!(r["refused_efforts"][0]["effort"], json!("high"));
        assert_eq!(r["refused_efforts"][0]["remaining_secs"], json!(3 * 3600 + 300));
        assert_eq!(r["expired_refusals"], json!(1));
        assert_eq!(r["rate_limit"]["wait_secs"], json!(45));
        let text = doctor_text(&s);
        assert!(text.contains("high (skipped until 2026-"), "{text}");
        assert!(text.contains("3 h 5 min left"), "{text}");
        assert!(text.contains("medium (skipped until"), "{text}");
        assert!(text.contains("last request: 2026-") && text.contains("(2 min ago)"), "{text}");
        assert!(text.contains("1 older refusal(s) have expired"), "{text}");
        assert!(text.contains("rate limit backoff: 2 answer(s) of 429"), "{text}");

        // a revocation shows too
        let guard = route.acquire(&CancelToken::new(), &RealSleeper).unwrap();
        guard.record_revoked(now - 10, 401, b"token_revoked", "generation");
        drop(guard);
        let mut s = json!({"chatgpt_login": {"state": "signed in"}});
        note_route_state(&mut s, &env, now);
        let text = doctor_text(&s);
        assert!(text.contains("sign-in revoked by the server: YES"));
        // the revocation dropped the route to one request at a time, and the doctor says so
        assert!(text.contains("requests at once: 1 (configured 3)"), "{text}");
        assert!(
            text.contains(
                "warning: the ChatGPT-login route dropped to one request at a time on this machine after a sign-out by the server (token_revoked)"
            ),
            "{text}"
        );
        assert_eq!(s["chatgpt_login"]["route"]["fallback"]["reason"], json!("sign_out"));
        // six hours later it is back to three
        let mut s = json!({"chatgpt_login": {"state": "signed in"}});
        note_route_state(&mut s, &env, now - 10 + provider_chatgpt_login::route::FALLBACK_HOLD_SECS);
        let text = doctor_text(&s);
        assert!(text.contains("requests at once: up to 3 on this machine") && !text.contains("warning:"), "{text}");
    }

    #[test]
    fn the_doctor_shows_the_model_access_answers_and_the_slot_pause() {
        let (_dir, env, route) = work_env("dm");
        let now = 1_793_000_000;
        route
            .acquire(&CancelToken::new(), &RealSleeper)
            .unwrap()
            .update(|st| {
                let _ = st.record_model_access(0, now - 20);
            })
            .unwrap();
        let mut s = json!({"chatgpt_login": {"state": "signed in"}});
        note_route_state(&mut s, &env, now);
        let text = doctor_text(&s);
        assert!(text.contains("\"Unable to verify model access\" answers: 1 so far, the last at"), "{text}");
        assert!(text.contains("a slot waits 60 s after one, two within 15 min drop the route to one at a time for 60 min"), "{text}");
        assert!(text.contains("slot 0 is paused until") && text.contains("(40 s left)"), "{text}");
        assert!(!text.contains("warning:"), "{text}");
        route
            .acquire(&CancelToken::new(), &RealSleeper)
            .unwrap()
            .update(|st| {
                let _ = st.record_model_access(0, now - 10);
            })
            .unwrap();
        let mut s = json!({"chatgpt_login": {"state": "signed in"}});
        note_route_state(&mut s, &env, now);
        let text = doctor_text(&s);
        assert!(
            text.contains("requests at once: 1 (configured 3)") && text.contains("warning: the ChatGPT-login route dropped to one request at a time"),
            "{text}"
        );
        assert!(text.contains("(model_access_unverified)"), "{text}");
    }

    #[test]
    fn the_route_state_holds_no_secret_and_a_missing_section_prints_nothing() {
        let (_dir, env, route) = work_env("dy");
        route.acquire(&CancelToken::new(), &RealSleeper).unwrap().record_revoked(5, 401, b"access-token-secret text", "generation");
        let mut s = json!({"chatgpt_login": {}});
        note_route_state(&mut s, &env, 100);
        let dumped = s.to_string();
        assert!(!dumped.contains("secret"), "{dumped}");
        assert_eq!(route_text(&Value::Null), None);
    }
}
