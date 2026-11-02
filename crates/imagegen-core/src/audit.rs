//! The mod audit: which sprites and textures the mod actually uses, for which family, and whether the art fits.
//!
//! The index and the content-to-sprite trace come from the shared HOI4 engine (`hoi4-engine`, repository
//! hoi4-agent-tools). This module adds what only this tool knows: the family profiles. Every traced texture that
//! belongs to a known family is checked against that family's accepted sizes from the texture's header (no
//! pixels are decoded), and the textures of `report_event` are decoded to prove their corners are transparent.
//!
//! Existing reuse is a warning (a mod that shipped it already has it), and a per-mod allowlist silences
//! intentional aliases. The same trace backs [`ensure_no_trace_reuse`], the cheap pre-install
//! refusal that stops a new install from making such reuse.
//!
//! Nothing here writes to the project or the game.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use hoi4_engine::findings::Severity;
use hoi4_engine::sprite_audit::{Allowlist, FamilyTally, SpriteReport, SpriteSummary};
use hoi4_engine::sprite_trace::{Family as TraceFamily, SpriteUse, TraceOptions};
use hoi4_engine::sprites::texture_key;
use hoi4_engine::{Workspace, WorkspaceOptions};
use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{Error, Result, RetryClass};
use crate::family::Family;
use crate::lock::REUSE_CODE;
use crate::pixels::{decode, DecodeLimits};
use crate::profile::ProfileSet;

/// Where a project keeps its allowlist when the caller names none.
pub const DEFAULT_ALLOWLIST: &str = ".hoi4-imagegen/audit-allowlist.json";
/// Findings listed per code unless the caller asks for another number.
pub const DEFAULT_LIMIT: usize = 25;
/// Largest `limit`.
pub const MAX_LIMIT: usize = 5000;
/// Uses listed on one reuse finding.
const MAX_USES_PER_FINDING: usize = 100;
/// Report-event textures decoded for the corner check in one run.
const MAX_ALPHA_DECODES: usize = 3000;
/// Directory entries the pre-install trace walks before it gives up and leaves the check to the pixel-hash guard.
const MAX_INSTALL_CHECK_ENTRIES: usize = 60_000;
const MAX_TEXTURE_BYTES: u64 = 8 * 1024 * 1024;

/// The codes the report always lists, even with no finding.
pub const REPORT_CODES: [&str; 6] =
    ["GFX-FAMILY-REUSE", "GFX-TEXTURE-MISSING", "GFX-SPRITE-UNDEFINED", "GFX-FOCUS-SHINE-MISSING", "GFX-CANVAS-SIZE", "GFX-CARD-CORNERS"];

/// What to audit.
pub struct AuditInput<'a> {
    /// The mod folder.
    pub project_root: &'a Path,
    /// The installed game, when found. Without it vanilla sprites and textures are unknown.
    pub game_dir: Option<&'a Path>,
    /// Allowlist file, already resolved inside the project. `None` runs with no allowlist.
    pub allowlist: Option<&'a Path>,
    /// Only report these families (empty: all).
    pub families: &'a [Family],
    /// Findings listed per code.
    pub limit: usize,
    /// The effective profiles (accepted sizes per family).
    pub profiles: &'a ProfileSet,
}

/// One place a sprite or texture is used.
#[derive(Debug, Clone, Serialize)]
pub struct UseRef {
    /// Family of the slot.
    pub family: String,
    /// Sprite the game shows (empty for a texture path).
    pub sprite: String,
    /// Texture, when known.
    pub texture: Option<String>,
    /// File inside the layer.
    pub path: String,
    /// 1-based line.
    pub line: u32,
    /// Enclosing definition, such as `focus:GER_rearm`.
    pub subject: Option<String>,
    /// Rule id such as `focus.icon`.
    pub content_type: String,
}

/// One finding of the audit, from the engine or from this tool's own checks.
#[derive(Debug, Clone, Serialize)]
pub struct AuditFinding {
    /// Stable code.
    pub code: String,
    /// `info`, `warning` or `error`.
    pub severity: String,
    /// What is wrong.
    pub message: String,
    /// File inside the layer (empty for a texture check).
    pub path: String,
    /// 1-based line.
    pub line: u32,
    /// Sprite name, texture path or definition the finding is about.
    pub subject: Option<String>,
    /// The families involved.
    pub families: Vec<String>,
    /// Every use behind a reuse finding.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<UseRef>,
    /// Uses behind the finding when `uses` was cut short.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uses_total: Option<usize>,
    /// Numbers behind a check of this tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Value>,
    /// Suggested fix, when the engine has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Findings of one code.
#[derive(Debug, Clone, Serialize)]
pub struct CodeGroup {
    /// Severity of the code.
    pub severity: String,
    /// All findings of the code.
    pub count: usize,
    /// Findings listed below (at most `limit`).
    pub shown: usize,
    /// The findings.
    pub findings: Vec<AuditFinding>,
}

/// The audit result.
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    /// Schema id.
    pub schema: &'static str,
    /// Mod folder.
    pub project_root: String,
    /// Game folder used, if any.
    pub game_dir: Option<String>,
    /// Allowlist read, if any, and what it holds.
    pub allowlist: Value,
    /// Family filter in force (empty: all).
    pub families_filter: Vec<String>,
    /// Traced uses per family (after the filter).
    pub uses_by_family: BTreeMap<String, FamilyTally>,
    /// The engine's totals (index, trace and timing, before the family filter).
    pub summary: SpriteSummary,
    /// Findings by code. Every code in [`REPORT_CODES`] is present.
    pub findings: BTreeMap<String, CodeGroup>,
    /// Textures compared with their family's accepted sizes.
    pub canvas_checked: usize,
    /// `report_event` textures decoded for the corner check.
    pub alpha_checked: usize,
    /// True when any code listed fewer findings than it has.
    pub truncated: bool,
    /// Caveats of this run.
    pub notes: Vec<String>,
    /// Milliseconds spent in this tool's own checks.
    pub checks_ms: u128,
}

fn parse_family(f: TraceFamily) -> Option<Family> {
    if f == TraceFamily::Unknown {
        return None;
    }
    f.as_str().parse().ok()
}

fn use_ref(u: &SpriteUse) -> UseRef {
    UseRef {
        family: u.family.as_str().to_string(),
        sprite: u.sprite.clone(),
        texture: u.texture.as_ref().map(|t| t.path.clone()),
        path: u.path.clone(),
        line: u.line,
        subject: u.subject.clone(),
        content_type: u.content_type.clone(),
    }
}

fn severity_name(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

fn load_workspace(root: &Path, game: Option<&Path>) -> Workspace {
    Workspace::load(&WorkspaceOptions {
        game: game.map(Path::to_path_buf),
        target: Some(root.to_path_buf()),
        include_dlc: game.is_some(),
        ..Default::default()
    })
}

/// Run the audit. Reads the mod, the game and the allowlist; writes nothing.
pub fn run_audit(input: &AuditInput) -> Result<AuditReport> {
    let mut notes: Vec<String> = Vec::new();
    let allow = match input.allowlist {
        Some(path) => Allowlist::load(path).map_err(Error::Invalid)?,
        None => Allowlist::default(),
    };
    if input.game_dir.is_none() {
        notes.push(
            "the installed game folder was not found (set HOI4_GAME_DIR or game_dir in .hoi4-imagegen/config.toml): vanilla sprites and textures are unknown, so GFX-SPRITE-UNDEFINED and GFX-TEXTURE-MISSING may name vanilla content that is fine".into(),
        );
    }
    let ws = load_workspace(input.project_root, input.game_dir);
    if ws.target_layer().is_none() {
        return Err(Error::NotFound(format!("no mod files under {}", input.project_root.display())));
    }
    let (index, report) = SpriteReport::run(&ws, &allow, &TraceOptions::default());

    let t0 = Instant::now();
    let want: BTreeSet<Family> = input.families.iter().copied().collect();
    let keep = |f: TraceFamily| want.is_empty() || parse_family(f).is_some_and(|x| want.contains(&x));

    let mut by_sprite: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_texture: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, u) in report.uses.iter().enumerate() {
        if !u.sprite.is_empty() {
            by_sprite.entry(u.sprite.to_ascii_lowercase()).or_default().push(i);
        }
        if let Some(t) = &u.texture {
            by_texture.entry(texture_key(&t.path)).or_default().push(i);
        }
    }
    let uses_of = |subject: &str| -> Vec<usize> {
        let mut v: Vec<usize> = by_sprite.get(&subject.to_ascii_lowercase()).cloned().unwrap_or_default();
        for i in by_texture.get(&texture_key(subject)).into_iter().flatten() {
            if !v.contains(i) {
                v.push(*i);
            }
        }
        v
    };

    let mut groups: BTreeMap<String, Vec<AuditFinding>> = REPORT_CODES.iter().map(|c| (c.to_string(), Vec::new())).collect();
    let mut severities: BTreeMap<String, &'static str> = REPORT_CODES.iter().map(|c| (c.to_string(), "warning")).collect();
    severities.insert("GFX-FOCUS-SHINE-MISSING".into(), "info");

    // the engine's findings, with their uses and families attached
    for f in &report.findings {
        let subject = f.subject.clone().unwrap_or_default();
        let idx = if subject.is_empty() { Vec::new() } else { uses_of(&subject) };
        let mut fams: BTreeSet<String> =
            idx.iter().map(|i| report.uses[*i].family).filter(|f| *f != TraceFamily::Unknown).map(|f| f.as_str().to_string()).collect();
        if f.code == "GFX-FOCUS-SHINE-MISSING" {
            fams.insert("focus".into());
        }
        if !want.is_empty() {
            let touches = idx.iter().any(|i| keep(report.uses[*i].family))
                || (idx.is_empty() && want.contains(&Family::Focus) && f.code == "GFX-FOCUS-SHINE-MISSING");
            if !touches {
                continue;
            }
        }
        let reuse = f.code == "GFX-FAMILY-REUSE";
        // the engine's message repeats every use; the structured `uses` carry them, so keep the lead sentence only
        let message = if reuse {
            f.message.split_once("): ").map(|(lead, _)| format!("{lead})")).unwrap_or_else(|| f.message.clone())
        } else {
            f.message.clone()
        };
        let uses: Vec<UseRef> = if reuse { idx.iter().take(MAX_USES_PER_FINDING).map(|i| use_ref(&report.uses[*i])).collect() } else { Vec::new() };
        severities.insert(f.code.clone(), severity_name(f.severity));
        groups.entry(f.code.clone()).or_default().push(AuditFinding {
            code: f.code.clone(),
            severity: severity_name(f.severity).to_string(),
            message,
            path: f.path.clone(),
            line: f.line,
            subject: f.subject.clone(),
            families: fams.into_iter().collect(),
            uses_total: (reuse && idx.len() > MAX_USES_PER_FINDING).then_some(idx.len()),
            uses,
            evidence: None,
            suggestion: f.suggestion.clone(),
        });
    }

    // this tool's checks on every traced texture that belongs to a known family
    let mut canvas_checked = 0usize;
    let mut alpha_checked = 0usize;
    let mut seen_canvas: BTreeSet<(Family, String)> = BTreeSet::new();
    let mut seen_alpha: BTreeSet<String> = BTreeSet::new();
    let mut alpha_cap_hit = false;
    for u in &report.uses {
        let Some(family) = parse_family(u.family) else { continue };
        if !want.is_empty() && !want.contains(&family) {
            continue;
        }
        let Some(t) = &u.texture else { continue };
        if !t.exists {
            continue;
        }
        let key = texture_key(&t.path);
        if let (Some(w), Some(h)) = (t.width, t.height) {
            if seen_canvas.insert((family, key.clone())) {
                if let Some(profile) = input.profiles.for_family(family) {
                    let accepted = profile.accepted_strips();
                    if !accepted.is_empty() {
                        canvas_checked += 1;
                        if !accepted.contains(&[w, h]) {
                            let list: Vec<String> = accepted.iter().map(|s| format!("{}x{}", s[0], s[1])).collect();
                            severities.insert("GFX-CANVAS-SIZE".into(), "warning");
                            groups.entry("GFX-CANVAS-SIZE".into()).or_default().push(AuditFinding {
                                code: "GFX-CANVAS-SIZE".into(),
                                severity: "warning".into(),
                                message: format!(
                                    "{} is {w}x{h} and {family} art is {} ({} {})",
                                    t.path,
                                    list.join(", "),
                                    u.content_type,
                                    u.subject.as_deref().unwrap_or("")
                                )
                                .trim_end()
                                .to_string(),
                                path: u.path.clone(),
                                line: u.line,
                                subject: Some(t.path.clone()),
                                families: vec![family.to_string()],
                                uses: Vec::new(),
                                uses_total: None,
                                evidence: Some(json!({"size": [w, h], "accepted": accepted, "sprite": u.sprite, "layer": t.layer})),
                                suggestion: None,
                            });
                        }
                    }
                }
            }
        }
        if family == Family::ReportEvent && seen_alpha.insert(key.clone()) {
            if alpha_checked >= MAX_ALPHA_DECODES {
                alpha_cap_hit = true;
                continue;
            }
            let resolved = index.texture(&t.path);
            let Some(abs) = resolved.absolute else { continue };
            if std::fs::metadata(&abs).map(|m| m.len() > MAX_TEXTURE_BYTES).unwrap_or(true) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&abs) else { continue };
            let Ok((img, _)) = decode(&bytes, Some(&t.path), DecodeLimits::default()) else { continue };
            alpha_checked += 1;
            let (w, h) = (img.width.saturating_sub(1), img.height.saturating_sub(1));
            let corners = [img.alpha(0, 0), img.alpha(w, 0), img.alpha(0, h), img.alpha(w, h)];
            if corners.iter().any(|a| *a != 0) {
                severities.insert("GFX-CARD-CORNERS".into(), "warning");
                groups.entry("GFX-CARD-CORNERS".into()).or_default().push(AuditFinding {
                    code: "GFX-CARD-CORNERS".into(),
                    severity: "warning".into(),
                    message: format!(
                        "{} is a report event card and a corner is not fully transparent (alpha TL {}, TR {}, BL {}, BR {}); vanilla cards have transparent corners",
                        t.path, corners[0], corners[1], corners[2], corners[3]
                    ),
                    path: u.path.clone(),
                    line: u.line,
                    subject: Some(t.path.clone()),
                    families: vec![family.to_string()],
                    uses: Vec::new(),
                    uses_total: None,
                    evidence: Some(json!({"corner_alpha": corners, "size": [img.width, img.height]})),
                    suggestion: None,
                });
            }
        }
    }
    if alpha_cap_hit {
        notes.push(format!("the corner check stopped after {MAX_ALPHA_DECODES} report event textures"));
    }

    let limit = input.limit.clamp(1, MAX_LIMIT);
    let mut truncated = false;
    let findings: BTreeMap<String, CodeGroup> = groups
        .into_iter()
        .map(|(code, list)| {
            let count = list.len();
            let shown: Vec<AuditFinding> = list.into_iter().take(limit).collect();
            truncated |= shown.len() < count;
            let severity = severities.get(&code).copied().unwrap_or("warning").to_string();
            (code, CodeGroup { severity, count, shown: shown.len(), findings: shown })
        })
        .collect();

    let mut uses_by_family: BTreeMap<String, FamilyTally> = BTreeMap::new();
    for (k, v) in &report.summary.uses_by_family {
        let fam = TraceFamily::parse(k);
        if want.is_empty() || fam.is_some_and(keep) {
            uses_by_family.insert(k.clone(), v.clone());
        }
    }
    if report.summary.unknown_family_uses > 0 && want.is_empty() {
        notes.push(format!(
            "{} uses sit in slots no family covers (family unknown); they are counted but not checked",
            report.summary.unknown_family_uses
        ));
    }
    if report.summary.header_errors > 0 {
        notes.push(format!("{} textures have an unreadable header and are not size checked", report.summary.header_errors));
    }

    let allowlist_info = match input.allowlist {
        Some(p) => {
            json!({"path": crate::fsio::display_path(p), "aliases": allow.aliases.len(), "no_shine": allow.no_shine.len(), "missing_textures": allow.missing_textures.len()})
        }
        None => Value::Null,
    };
    Ok(AuditReport {
        schema: "hoi4-imagegen-audit/1",
        project_root: crate::fsio::display_path(input.project_root),
        game_dir: input.game_dir.map(crate::fsio::display_path),
        allowlist: allowlist_info,
        families_filter: input.families.iter().map(|f| f.to_string()).collect(),
        uses_by_family,
        summary: report.summary.clone(),
        findings,
        canvas_checked,
        alpha_checked,
        truncated,
        notes,
        checks_ms: t0.elapsed().as_millis(),
    })
}

fn alias_allows(allow: &Allowlist, sprite: &str, texture: &str, families: &BTreeSet<String>) -> bool {
    let key = texture_key(texture);
    allow.aliases.iter().any(|a| {
        let target =
            a.sprite.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(sprite)) || a.texture.as_deref().is_some_and(|t| texture_key(t) == key);
        target && a.families.as_ref().is_none_or(|list| families.iter().all(|f| list.iter().any(|l| l == f)))
    })
}

/// Counts directory entries under `root`, stopping at `cap`. `.git` and this tool's own folder are not mod content.
fn count_entries(root: &Path, cap: usize) -> usize {
    let mut n = 0usize;
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            n += 1;
            if n > cap {
                return n;
            }
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if name == ".git" || name == ".hoi4-imagegen" {
                continue;
            }
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stack.push(e.path());
            }
        }
    }
    n
}

/// How the pre-install trace ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceGuard {
    /// The trace ran and found no other family using these names or textures.
    Checked,
    /// The trace was not run (too large, or no mod files); the pixel-hash guard still applies.
    Skipped(String),
}

/// Refuse an install that would make a sprite name or texture serve two families.
///
/// Traces the mod's own files (no game: only the mod's use of these exact names and paths matters, and the mod
/// alone is cheap to read) and refuses with `GFX-FAMILY-REUSE` when any of `sprites` (names) or `textures`
/// (project-relative final paths) is already used by the mod for a family other than `family`, unless the
/// allowlist covers it. A mod with more than [`MAX_INSTALL_CHECK_ENTRIES`] directory entries skips the trace
/// and says so; the pixel-hash guard in [`crate::reuse`] always runs separately.
pub fn ensure_no_trace_reuse(
    project_root: &Path,
    allowlist: Option<&Path>,
    family: Family,
    sprites: &[String],
    textures: &[String],
) -> Result<TraceGuard> {
    if sprites.is_empty() && textures.is_empty() {
        return Ok(TraceGuard::Checked);
    }
    if count_entries(project_root, MAX_INSTALL_CHECK_ENTRIES) > MAX_INSTALL_CHECK_ENTRIES {
        return Ok(TraceGuard::Skipped(format!(
            "the mod has more than {MAX_INSTALL_CHECK_ENTRIES} files and folders; only the pixel-hash reuse guard ran"
        )));
    }
    let allow = match allowlist {
        Some(p) => Allowlist::load(p).map_err(Error::Invalid)?,
        None => Allowlist::default(),
    };
    let ws = load_workspace(project_root, None);
    if ws.target_layer().is_none() {
        return Ok(TraceGuard::Skipped("no mod files to trace".into()));
    }
    let index = hoi4_engine::sprites::SpriteIndex::build(&ws);
    let uses = hoi4_engine::sprite_trace::trace(&ws, &index, &TraceOptions::default());
    let names: BTreeSet<String> = sprites.iter().map(|s| s.to_ascii_lowercase()).collect();
    let keys: BTreeSet<String> = textures.iter().map(|t| texture_key(t)).collect();
    let mut clashes: Vec<String> = Vec::new();
    let mut other_families: BTreeSet<String> = BTreeSet::new();
    for u in &uses {
        let Some(other) = parse_family(u.family) else { continue };
        if other == family {
            continue;
        }
        let by_name = !u.sprite.is_empty() && names.contains(&u.sprite.to_ascii_lowercase());
        let by_texture = u.texture.as_ref().is_some_and(|t| keys.contains(&texture_key(&t.path)));
        if !by_name && !by_texture {
            continue;
        }
        let fams: BTreeSet<String> = [other.to_string(), family.to_string()].into();
        let texture = u.texture.as_ref().map(|t| t.path.as_str()).unwrap_or("");
        if alias_allows(&allow, &u.sprite, texture, &fams) {
            continue;
        }
        other_families.insert(other.to_string());
        if clashes.len() < 5 {
            clashes.push(format!(
                "{} used as {} at {}:{}{}",
                if by_name { u.sprite.as_str() } else { texture },
                other,
                u.path,
                u.line,
                u.subject.as_ref().map(|s| format!(" ({s})")).unwrap_or_default()
            ));
        }
    }
    if clashes.is_empty() {
        return Ok(TraceGuard::Checked);
    }
    Err(Error::refused(
        REUSE_CODE,
        format!(
            "this {family} install would reuse a sprite name or texture the mod already uses as {}: {}. One sprite or texture is one family. Choose another asset name, or record an intentional alias in {DEFAULT_ALLOWLIST}",
            other_families.into_iter().collect::<Vec<_>>().join(", "),
            clashes.join("; ")
        ),
        RetryClass::Terminal,
    ))
}
