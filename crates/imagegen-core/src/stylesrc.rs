//! Automatic style references: where a family's style comes from and which images show it.
//!
//! Sources come from the mod first (maintainer decision of 2026-11-01), then from the game:
//!
//! 1. the project's pinned style profile for the family,
//! 2. the mod's own copy of the asset skill's `assets/` shelf for the family
//!    (`.claude/skills/*/assets/*/<shelf folder>/`; the kit calls the root `vanilla_reference`, Chaos Redux
//!    `gfx_references`),
//! 3. the mod's own installed assets of the family (its texture folders, and the textures its `.gfx` files register
//!    under the family's sprite name pattern),
//! 4. vanilla art from the installed game.
//!
//! The first tier with at least three usable images wins. Its images are measured ([`crate::styledesc`]), a
//! deterministic k-medoids picks the few that are sent to the provider, and up to 24 medoids feed the style
//! statistics. The plan reports each choice with its tier, reason and hash. Vanilla art may always be sent; the tool
//! never copies it into the mod.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::family::Family;
use crate::fsio::{display_path, read_limited, ProjectFs};
use crate::hashing::sha256_hex;
use crate::pixels::{decode, DecodeLimits};
use crate::profile::Profile;
use crate::refs::{classify_project_path, RefOrigin, MAX_REFERENCE_BYTES};
use crate::style::StyleProfile;
use crate::styledesc::{self, Analysis, DriftModel, StyleStats, Vector, STATS_IMAGES};

/// Fewest usable images a tier needs to be used.
pub const MIN_TIER_IMAGES: usize = 3;
/// Default number of references sent.
pub const DEFAULT_SEND: usize = 4;
/// Most candidates measured per tier. Larger sets are sampled evenly by path.
pub const POOL_CAP: usize = 256;
/// Most directory entries looked at per scan.
const SCAN_BUDGET: usize = 12_000;
/// Most `.gfx` files read when looking for registered sprites.
const MAX_GFX_FILES: usize = 400;
/// Largest `.gfx` file read.
const MAX_GFX_BYTES: usize = 2 * 1024 * 1024;

/// A source tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// A pinned style profile of the project.
    PinnedStyle,
    /// The mod's copy of the asset skill's shelf folder.
    ModAssetFolderCopy,
    /// The mod's own installed assets.
    ModAssets,
    /// Vanilla art from the installed game.
    Vanilla,
}

impl Tier {
    /// Every tier, best first.
    pub const ALL: [Tier; 4] = [Tier::PinnedStyle, Tier::ModAssetFolderCopy, Tier::ModAssets, Tier::Vanilla];

    /// 1 to 4.
    pub fn number(self) -> u8 {
        match self {
            Tier::PinnedStyle => 1,
            Tier::ModAssetFolderCopy => 2,
            Tier::ModAssets => 3,
            Tier::Vanilla => 4,
        }
    }

    /// Short name.
    pub fn label(self) -> &'static str {
        match self {
            Tier::PinnedStyle => "pinned style",
            Tier::ModAssetFolderCopy => "mod asset folder copy",
            Tier::ModAssets => "mod assets",
            Tier::Vanilla => "vanilla",
        }
    }
}

/// What one tier offered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TierSummary {
    /// 1 to 4.
    pub tier: u8,
    /// Tier name.
    pub name: String,
    /// True when this tier was looked at (lower tiers are skipped once a higher one is used, except in `families`).
    pub scanned: bool,
    /// Images found (after the name and size filters).
    pub found: usize,
    /// True for the tier whose images are used.
    pub chosen: bool,
    /// Why, or why not.
    pub note: String,
}

/// One image the selection picked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChosenRef {
    /// Project-relative path, or an absolute path inside the installed game.
    pub path: String,
    /// Where it comes from.
    pub origin: RefOrigin,
    /// Tier (1 to 4).
    pub tier: u8,
    /// Why this image.
    pub reason: String,
    /// SHA-256 (of the clean PNG for a sent reference, of the file for the others).
    pub sha256: String,
    /// True when the image goes to the provider.
    pub sent_to_provider: bool,
    /// True when it also feeds the local statistics.
    pub used_for_statistics: bool,
}

/// How the drift check is set up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DriftSummary {
    /// Images in the distribution.
    pub images: usize,
    /// 95th percentile of leave-one-out RMS z-scores of the set's own images.
    pub p95_leave_one_out: f64,
    /// Threshold for `GFX-STYLE-DRIFT`.
    pub threshold: f64,
    /// Lowest threshold ever used.
    pub minimum_threshold: f64,
}

/// What `hoi4.image_families` and a plan say about a family's style sources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StyleReport {
    /// Family.
    pub family: Family,
    /// Tier used, 1 to 4; `None` when no tier has three usable images.
    pub chosen_tier: Option<u8>,
    /// Every tier.
    pub tiers: Vec<TierSummary>,
    /// References that may be sent at most.
    pub send_limit: usize,
    /// How images were selected.
    pub method: String,
    /// Chosen images.
    pub chosen: Vec<ChosenRef>,
    /// Statistics of the chosen tier's statistics set.
    pub statistics: Option<StyleStats>,
    /// Paragraph added to the prompt.
    pub style_paragraph: Option<String>,
    /// Drift check set-up.
    pub drift: Option<DriftSummary>,
    /// Things to know (skipped tiers, undecodable files).
    pub notes: Vec<String>,
}

/// The outcome of resolving a family's sources.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The report.
    pub report: StyleReport,
    /// The drift model, when a tier qualified.
    pub drift: Option<DriftModel>,
    /// Tier used.
    pub tier: Option<Tier>,
    /// What the distribution describes, for messages (for example `the mod's existing national focus icon art`).
    pub subject: Option<String>,
}

impl Resolved {
    /// Paths of the references to send, in order.
    pub fn send_paths(&self) -> Vec<String> {
        self.report.chosen.iter().filter(|c| c.sent_to_provider).map(|c| c.path.clone()).collect()
    }

    /// Mark all but the first `cap` sent references as not sent.
    pub fn limit_send(&mut self, cap: usize, why: &str) {
        let mut sent = 0usize;
        for c in &mut self.report.chosen {
            if c.sent_to_provider {
                if sent >= cap {
                    c.sent_to_provider = false;
                    c.reason = format!("{} (not sent: {why})", c.reason);
                } else {
                    sent += 1;
                }
            }
        }
        self.report.send_limit = self.report.send_limit.min(cap);
    }
}

/// What a resolution needs.
pub struct SourceCtx<'a> {
    /// The project.
    pub fs: &'a ProjectFs,
    /// The installed game, when found.
    pub game_dir: Option<&'a Path>,
    /// The family's effective profile.
    pub profile: &'a Profile,
    /// The built-in (vanilla-measured) profile of the family, for vanilla sizes.
    pub builtin: Option<&'a Profile>,
    /// A pinned style to use as tier 1.
    pub pinned: Option<&'a StyleProfile>,
    /// References to send at most.
    pub max_send: usize,
    /// Look at every tier, not only down to the first usable one.
    pub scan_all: bool,
}

// ---- family tables ----

struct Dir {
    path: &'static str,
    recursive: bool,
    prefixes: &'static [&'static str],
    not_prefix: &'static [&'static str],
    not_contains: &'static [&'static str],
    not_suffix: &'static [&'static str],
}

fn dir(path: &'static str, recursive: bool) -> Dir {
    Dir { path, recursive, prefixes: &[], not_prefix: &[], not_contains: &[], not_suffix: &[] }
}

impl Dir {
    fn pre(mut self, p: &'static [&'static str]) -> Dir {
        self.prefixes = p;
        self
    }
    fn not_pre(mut self, p: &'static [&'static str]) -> Dir {
        self.not_prefix = p;
        self
    }
    fn not_has(mut self, p: &'static [&'static str]) -> Dir {
        self.not_contains = p;
        self
    }
    fn not_end(mut self, p: &'static [&'static str]) -> Dir {
        self.not_suffix = p;
        self
    }
    fn accepts(&self, stem: &str) -> bool {
        (self.prefixes.is_empty() || self.prefixes.iter().any(|p| stem.starts_with(p)))
            && !self.not_prefix.iter().any(|p| stem.starts_with(p))
            && !self.not_contains.iter().any(|p| stem.contains(p))
            && !self.not_suffix.iter().any(|p| stem.ends_with(p))
    }
}

/// Folder of the family inside the asset skill's `assets/<root>/` shelf (the kit's `build_shelf.py` layout).
pub fn shelf_folder(f: Family) -> Option<&'static str> {
    use Family::*;
    Some(match f {
        Focus => "icons/national_focus",
        IdeaSpirit => "icons/ideas",
        IdeaConcern => "icons/idea_concerns",
        OfficerCorpsSpirit => "icons/officer_corps_spirits",
        Decision => "icons/decisions",
        DecisionCategoryIcon => "icons/decision_categories",
        StateModifier => "icons/state_modifiers",
        SpecialProjectIcon => "icons/special_projects",
        Raid => "icons/military_raids",
        MioDepartment => "icons/military_industrial_organizations",
        MioTrait => "icons/mio_traits",
        MioPolicy => "icons/mio_policies",
        BopSide => "icons/balance_of_power",
        Medal => "icons/medals",
        AgencyUpgrade => "icons/intelligence_agency",
        AgencyLogo => "icons/agency_logos",
        OperationIcon => "icons/intelligence_operations",
        CommanderTrait => "icons/commander_traits",
        PersonalTrait => "icons/personal_traits",
        Doctrine => "icons/doctrines",
        ReportEvent => "event_art/report",
        NewsEvent => "event_art/news",
        DecisionCategoryPicture => "icons/decision_categories/pictures",
        OperationPicture => "event_art/operation",
        Technology => "icons/technologies",
        Achievement => "icons/achievements",
        LeaderPortrait => "portraits/leaders",
        CommanderPortrait => "portraits/commanders",
        OperativePortrait => "portraits/operatives",
        ScientistPortrait => "portraits/scientists",
        AdvisorCard => "portraits/advisors",
        Flag => "flags",
        CounterLarge => "units/land/counters_large",
        CounterSmall => "units/land/map_counters",
        DivisionEmblem => "units/land/division_template_emblems",
        Building => "icons/buildings",
        FactionLogo => "icons/factions",
        ModifierStatIcon => "icons/modifiers",
        SuperEvent => "event_art/super_event",
        GuiTexture => return None,
    })
}

/// Texture folders (game-relative, also used inside a mod) and file-name rules of a family.
fn texture_dirs(f: Family) -> Vec<Dir> {
    use Family::*;
    match f {
        Focus => vec![dir("gfx/interface/goals", true).pre(&["goal", "focus"]).not_has(&["untitl"])],
        IdeaSpirit => vec![dir("gfx/interface/ideas", true).pre(&["idea_"])],
        IdeaConcern => vec![dir("gfx/interface/ideas", true)],
        OfficerCorpsSpirit => vec![dir("gfx/interface/officer_corp/spirits", true)],
        Decision => vec![dir("gfx/interface/decisions", true).pre(&["decision_"]).not_pre(&["decision_cat"])],
        DecisionCategoryIcon => vec![dir("gfx/interface/decisions", true).pre(&["decision_category"]).not_pre(&["decision_cat_"])],
        StateModifier => vec![dir("gfx/interface/state_modifiers", true)],
        SpecialProjectIcon => vec![dir("gfx/interface/special_project/project_icons", true)],
        Raid => vec![dir("gfx/interface/military_raids", true).pre(&["raid_target_", "raid_type_", "raid_unit_"])],
        MioDepartment => vec![dir("gfx/interface/military_industrial_organization/department_icons", true)],
        MioTrait => vec![dir("gfx/interface/military_industrial_organization/trait_icons", true)],
        MioPolicy => vec![dir("gfx/interface/military_industrial_organization/policy_icons", true)],
        BopSide => vec![dir("gfx/interface/bop", true)],
        Medal => vec![dir("gfx/interface/medals", true)],
        AgencyUpgrade => vec![dir("gfx/interface/operatives/icons", true)],
        AgencyLogo => vec![dir("gfx/interface/operatives/agencies", true)],
        OperationIcon => vec![dir("gfx/interface/operations", true)],
        CommanderTrait => vec![dir("gfx/interface/traits", false).pre(&["trait_"])],
        PersonalTrait => vec![dir("gfx/interface/traits/personal", true)],
        Doctrine => vec![dir("gfx/interface/doctrines", true).pre(&["doctrine_"])],
        ReportEvent => vec![dir("gfx/event_pictures", true).pre(&["report_event"])],
        NewsEvent => vec![dir("gfx/event_pictures", true).pre(&["news_event"])],
        DecisionCategoryPicture => vec![dir("gfx/interface/decisions", true).pre(&["decision_cat_"])],
        OperationPicture => vec![dir("gfx/interface/operations", true).pre(&["phase_"]).not_has(&["_small"])],
        Technology => vec![dir("gfx/interface/technologies", true)],
        Achievement => vec![dir("gfx/achievements", false).not_has(&["_grey", "_not_eligible"])],
        LeaderPortrait | CommanderPortrait | OperativePortrait | ScientistPortrait => vec![dir("gfx/leaders", true)],
        AdvisorCard => vec![dir("gfx/interface/ideas", true)],
        Flag => vec![dir("gfx/flags", false)],
        CounterLarge => vec![dir("gfx/interface/counters/divisions_large", true)],
        CounterSmall => vec![dir("gfx/interface/counters/divisions_small", true)],
        DivisionEmblem => vec![dir("gfx/interface/counters/division_templates_large", true)],
        Building => vec![dir("gfx/interface/buildings", true)],
        FactionLogo => vec![dir("gfx/interface/factions/faction_logos", true).not_end(&["_miniature"])],
        ModifierStatIcon => vec![dir("gfx/interface/modifiers", true).pre(&["modifier_"])],
        SuperEvent | GuiTexture => vec![],
    }
}

// ---- scanning ----

#[derive(Debug, Clone)]
struct Cand {
    display: String,
    abs: PathBuf,
    origin: RefOrigin,
}

const RASTER: [&str; 3] = ["png", "dds", "tga"];
const RASTER_ALL: [&str; 6] = ["png", "dds", "tga", "jpg", "jpeg", "webp"];

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn stem_of(p: &Path) -> String {
    p.file_stem().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Files below `root`, sorted by name at every level, without following symlinks.
fn walk(root: &Path, recursive: bool, depth: u32, budget: &mut usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if recursive && depth < 6 {
                walk(&e.path(), true, depth + 1, budget, out);
            }
        } else if ft.is_file() {
            out.push(e.path());
        }
    }
}

/// Native size from the file header (DDS, TGA and PNG). `None` when unreadable or another container.
pub fn header_size(path: &Path) -> Option<[u32; 2]> {
    let mut f = File::open(path).ok()?;
    let mut buf = [0u8; 128];
    let n = f.read(&mut buf).ok()?;
    match ext_of(path).as_str() {
        "dds" if n >= 128 => crate::dds::parse_header(&buf).ok().map(|i| [i.width, i.height]),
        "tga" if n >= 18 => crate::tga::parse_header(&buf).ok().map(|i| [i.width, i.height]),
        "png" if n >= 24 && buf[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] => {
            Some([u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]), u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]])])
        }
        _ => None,
    }
}

fn size_ok(path: &Path, sizes: &[[u32; 2]]) -> bool {
    if sizes.is_empty() {
        return true;
    }
    match ext_of(path).as_str() {
        "dds" | "tga" | "png" => header_size(path).map(|d| sizes.contains(&d)).unwrap_or(false),
        _ => true,
    }
}

fn scan_dirs(base: &Path, dirs: &[Dir], sizes: &[[u32; 2]], exts: &[&str]) -> Vec<PathBuf> {
    let mut budget = SCAN_BUDGET;
    let mut files = Vec::new();
    for d in dirs {
        let mut found = Vec::new();
        walk(&base.join(d.path), d.recursive, 0, &mut budget, &mut found);
        for p in found {
            if exts.contains(&ext_of(&p).as_str()) && d.accepts(&stem_of(&p)) && size_ok(&p, sizes) && !files.contains(&p) {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

fn unify(p: &str) -> String {
    p.replace('\\', "/")
}

fn project_cands(fs: &ProjectFs, files: Vec<PathBuf>) -> Vec<Cand> {
    let mut out: Vec<Cand> = files
        .into_iter()
        .filter_map(|abs| {
            let rel = unify(&abs.strip_prefix(fs.root()).ok()?.to_string_lossy());
            Some(Cand { origin: classify_project_path(&rel), display: rel, abs })
        })
        .collect();
    out.sort_by(|a, b| a.display.cmp(&b.display));
    out.dedup_by(|a, b| a.display == b.display);
    out
}

fn game_cands(files: Vec<PathBuf>) -> Vec<Cand> {
    let mut out: Vec<Cand> =
        files.into_iter().map(|abs| Cand { display: display_path(&abs), origin: RefOrigin::InstalledGame, abs }).collect();
    out.sort_by(|a, b| a.display.cmp(&b.display));
    out
}

/// Textures that the mod's `.gfx` files register under the sprite names of the family.
fn registered_textures(fs: &ProjectFs, profile: &Profile) -> Vec<String> {
    let Some(spec) = &profile.imagegen.sprite else { return vec![] };
    let pattern = spec.name.replace("{prefix}", &profile.imagegen.prefix).to_ascii_lowercase();
    let Some((pre, post)) = pattern.split_once("{asset}") else { return vec![] };
    let Ok(interface) = fs.resolve("interface") else { return vec![] };
    let mut budget = SCAN_BUDGET;
    let mut files = Vec::new();
    walk(&interface, true, 0, &mut budget, &mut files);
    let mut out: Vec<String> = Vec::new();
    for p in files.into_iter().filter(|p| ext_of(p) == "gfx").take(MAX_GFX_FILES) {
        let Ok(bytes) = read_limited(&p, MAX_GFX_BYTES) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        for (name, tex) in parse_sprites(&text) {
            let lower = name.to_ascii_lowercase();
            if lower.len() > pre.len() + post.len() && lower.starts_with(pre) && lower.ends_with(post) {
                let tex = unify(&tex);
                if !out.contains(&tex) {
                    out.push(tex);
                }
            }
        }
    }
    out
}

fn quoted_after(block: &str, lower: &str, key: &str) -> Option<String> {
    let mut from = 0;
    while let Some(pos) = lower[from..].find(key) {
        let at = from + pos;
        from = at + key.len();
        let boundary = at == 0 || !(lower.as_bytes()[at - 1].is_ascii_alphanumeric() || lower.as_bytes()[at - 1] == b'_');
        if !boundary {
            continue;
        }
        let rest = block[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('"') else { continue };
        return rest.split_once('"').map(|(v, _)| v.to_string());
    }
    None
}

/// `(sprite name, texturefile)` pairs of a `.gfx` file's `spriteType` blocks.
pub fn parse_sprites(text: &str) -> Vec<(String, String)> {
    let clean: String = text.lines().map(|l| l.split('#').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let lower = clean.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut starts: Vec<usize> = lower.match_indices("spritetype").map(|(i, _)| i).collect();
    starts.push(clean.len());
    for w in starts.windows(2) {
        let (block, lblock) = (&clean[w[0]..w[1]], &lower[w[0]..w[1]]);
        if let (Some(name), Some(tex)) = (quoted_after(block, lblock, "name"), quoted_after(block, lblock, "texturefile")) {
            out.push((name, tex));
        }
    }
    out
}

struct Pooled {
    cand: Cand,
    file_sha: String,
    analysis: Analysis,
}

/// Measure candidates: sample evenly beyond the cap, drop what cannot be read, drop duplicates, order by content hash.
fn build_pool(cands: &[Cand], cap: usize, notes: &mut Vec<String>) -> Vec<Pooled> {
    let picked: Vec<&Cand> = if cands.len() > cap { (0..cap).map(|i| &cands[i * cands.len() / cap]).collect() } else { cands.iter().collect() };
    let mut pool: Vec<Pooled> = Vec::new();
    let mut unreadable = 0usize;
    for c in picked {
        let Ok(bytes) = read_limited(&c.abs, MAX_REFERENCE_BYTES) else {
            unreadable += 1;
            continue;
        };
        let Ok((img, _)) = decode(&bytes, c.abs.file_name().map(|n| n.to_string_lossy().to_string()).as_deref(), DecodeLimits::default()) else {
            unreadable += 1;
            continue;
        };
        let Some(analysis) = styledesc::analyse(&img) else {
            unreadable += 1;
            continue;
        };
        let file_sha = sha256_hex(&bytes);
        if pool.iter().any(|p| p.file_sha == file_sha) {
            continue;
        }
        pool.push(Pooled { cand: c.clone(), file_sha, analysis });
    }
    if unreadable > 0 {
        notes.push(format!("{unreadable} candidate file(s) could not be decoded or were empty and were left out"));
    }
    pool.sort_by(|a, b| a.file_sha.cmp(&b.file_sha).then(a.cand.display.cmp(&b.cand.display)));
    pool
}

fn subject_phrase(tier: Tier, family: Family, pinned: Option<&StyleProfile>) -> String {
    match tier {
        Tier::PinnedStyle => format!("the pinned reference images of the style '{}'", pinned.map(|s| s.name.as_str()).unwrap_or("")),
        Tier::ModAssetFolderCopy => format!("the mod's own reference shelf for {} art", family.label()),
        Tier::ModAssets => format!("the mod's existing {} art", family.label()),
        Tier::Vanilla => format!("vanilla Hearts of Iron IV {} art", family.label()),
    }
}

/// Resolve the style sources of a family. Never fails: a tier that cannot be read is reported and skipped.
pub fn resolve(ctx: &SourceCtx) -> Resolved {
    let family = ctx.profile.family;
    let mut notes: Vec<String> = Vec::new();
    let mut summaries: Vec<TierSummary> = Vec::new();
    let mut chosen_tier: Option<Tier> = None;
    let mut chosen_pool: Vec<Pooled> = Vec::new();
    let mut chosen_found = 0usize;
    // a pinned style with fewer than three images has no statistics but is always sent
    let mut pinned_small: Vec<Cand> = Vec::new();

    let effective_sizes = ctx.profile.accepted_strips();
    let mut vanilla_sizes = ctx.builtin.map(|b| b.accepted_strips()).unwrap_or_default();
    if vanilla_sizes.is_empty() {
        vanilla_sizes = effective_sizes.clone();
    }

    for tier in Tier::ALL {
        let mut summary = TierSummary { tier: tier.number(), name: tier.label().into(), scanned: false, found: 0, chosen: false, note: String::new() };
        if chosen_tier.is_some() && !ctx.scan_all {
            summary.note = "not scanned: a higher tier is used".into();
            summaries.push(summary);
            continue;
        }
        summary.scanned = true;
        let cands: Vec<Cand> = match tier {
            Tier::PinnedStyle => match ctx.pinned {
                None => {
                    summary.note = "no pinned style profile applies to this family".into();
                    vec![]
                }
                Some(s) => {
                    let list: Vec<Cand> = s
                        .references
                        .iter()
                        .filter_map(|r| {
                            let abs = ctx.fs.resolve(&r.path).ok()?;
                            abs.is_file().then(|| Cand { display: unify(&r.path), origin: classify_project_path(&r.path), abs })
                        })
                        .collect();
                    if list.is_empty() {
                        summary.note = format!("style '{}' pins no readable reference", s.name);
                    }
                    list
                }
            },
            Tier::ModAssetFolderCopy => match shelf_folder(family) {
                None => {
                    summary.note = "this family has no shelf folder".into();
                    vec![]
                }
                Some(shelf) => {
                    let mut sizes = vanilla_sizes.clone();
                    for s in &effective_sizes {
                        if !sizes.contains(s) {
                            sizes.push(*s);
                        }
                    }
                    let mut files = Vec::new();
                    for skill in sorted_names(ctx.fs, ".claude/skills") {
                        let assets = format!(".claude/skills/{skill}/assets");
                        for root in sorted_names(ctx.fs, &assets) {
                            let rel = format!("{assets}/{root}/{shelf}");
                            let Ok(abs) = ctx.fs.resolve(&rel) else { continue };
                            if !abs.is_dir() {
                                continue;
                            }
                            let mut budget = SCAN_BUDGET;
                            let mut found = Vec::new();
                            walk(&abs, true, 0, &mut budget, &mut found);
                            files.extend(found.into_iter().filter(|p| {
                                RASTER_ALL.contains(&ext_of(p).as_str()) && !stem_of(p).contains("contact_sheet") && size_ok(p, &sizes)
                            }));
                        }
                    }
                    let list = project_cands(ctx.fs, files);
                    if list.is_empty() {
                        summary.note = format!("no `.claude/skills/*/assets/*/{shelf}` folder with images in this mod");
                    }
                    list
                }
            },
            Tier::ModAssets => {
                let dirs: Vec<Dir> = texture_dirs(family).into_iter().filter(|d| ctx.fs.resolve(d.path).is_ok()).collect();
                let mut files = scan_dirs(ctx.fs.root(), &dirs, &effective_sizes, &RASTER);
                for tex in registered_textures(ctx.fs, ctx.profile) {
                    if let Ok(abs) = ctx.fs.resolve(&tex) {
                        if abs.is_file() && RASTER.contains(&ext_of(&abs).as_str()) && size_ok(&abs, &effective_sizes) && !files.contains(&abs) {
                            files.push(abs);
                        }
                    }
                }
                let list = project_cands(ctx.fs, files);
                if list.is_empty() {
                    summary.note = "the mod has no installed asset of this family (its texture folders and registered sprites were searched)".into();
                }
                list
            }
            Tier::Vanilla => match ctx.game_dir {
                None => {
                    summary.note = "the installed game folder was not found".into();
                    vec![]
                }
                Some(game) => {
                    let list = game_cands(scan_dirs(game, &texture_dirs(family), &vanilla_sizes, &RASTER));
                    if list.is_empty() {
                        summary.note = "the game folder has no vanilla art of this family".into();
                    }
                    list
                }
            },
        };
        summary.found = cands.len();
        if chosen_tier.is_none() && !cands.is_empty() {
            if cands.len() < MIN_TIER_IMAGES && tier != Tier::PinnedStyle {
                summary.note = format!("only {} image(s), at least {MIN_TIER_IMAGES} are needed", cands.len());
            } else if tier == Tier::PinnedStyle && cands.len() < MIN_TIER_IMAGES {
                summary.note = format!("{} pinned reference(s): always sent, but fewer than {MIN_TIER_IMAGES} give no statistics", cands.len());
                pinned_small = cands.clone();
            } else {
                let cap = if tier == Tier::PinnedStyle { 64 } else { POOL_CAP };
                let pool = build_pool(&cands, cap, &mut notes);
                if pool.len() >= MIN_TIER_IMAGES {
                    summary.chosen = true;
                    summary.note = format!("used: {} image(s) measured{}", pool.len(), if cands.len() > pool.len() { format!(" of {} found", cands.len()) } else { String::new() });
                    chosen_tier = Some(tier);
                    chosen_found = cands.len();
                    chosen_pool = pool;
                } else {
                    summary.note = format!("only {} of {} image(s) could be decoded, at least {MIN_TIER_IMAGES} are needed", pool.len(), cands.len());
                }
            }
        }
        summaries.push(summary);
    }

    let mut chosen: Vec<ChosenRef> = Vec::new();
    let mut drift_model = None;
    let mut drift = None;
    let mut statistics = None;
    let mut paragraph = None;
    let mut send_slots = ctx.max_send;

    // pinned references below the statistics minimum
    for (i, c) in pinned_small.iter().enumerate() {
        if send_slots == 0 {
            break;
        }
        send_slots -= 1;
        chosen.push(ChosenRef {
            path: c.display.clone(),
            origin: c.origin,
            tier: 1,
            reason: format!("pinned style '{}': reference {} of {}", ctx.pinned.map(|s| s.name.as_str()).unwrap_or(""), i + 1, pinned_small.len()),
            sha256: read_limited(&c.abs, MAX_REFERENCE_BYTES).map(|b| sha256_hex(&b)).unwrap_or_default(),
            sent_to_provider: true,
            used_for_statistics: false,
        });
    }

    if let Some(tier) = chosen_tier {
        let pool = &chosen_pool;
        let points: Vec<Vector> = pool.iter().map(|p| p.analysis.v).collect();
        let n = pool.len();
        let label = family.as_str();
        let (send_idx, send_why): (Vec<usize>, Vec<String>) = if tier == Tier::PinnedStyle && n <= send_slots {
            // pinned references are sent as the style's author ordered them
            let order: Vec<usize> = ctx
                .pinned
                .map(|s| s.references.iter().filter_map(|r| pool.iter().position(|p| p.cand.display == unify(&r.path))).collect())
                .unwrap_or_default();
            let why = (0..order.len()).map(|i| format!("pinned style '{}': reference {} of {}", ctx.pinned.map(|s| s.name.as_str()).unwrap_or(""), i + 1, order.len())).collect();
            (order, why)
        } else {
            let m = styledesc::kmedoids(&points, send_slots.min(n));
            let why = m
                .medoids
                .iter()
                .enumerate()
                .zip(&m.sizes)
                .map(|((rank, _), size)| match tier {
                    Tier::PinnedStyle => format!("pinned style: medoid {} of {} references (cluster of {size})", rank + 1, n),
                    Tier::ModAssetFolderCopy => {
                        format!("mod asset folder copy: medoid {} of {chosen_found} {label} images (cluster of {size})", rank + 1)
                    }
                    Tier::ModAssets => format!("mod asset: medoid {} of {chosen_found} {label} images (cluster of {size})", rank + 1),
                    Tier::Vanilla => format!("vanilla: medoid {} of {chosen_found} {label} images (cluster of {size})", rank + 1),
                })
                .collect();
            (m.medoids, why)
        };
        let stats_idx: Vec<usize> = if n <= STATS_IMAGES { (0..n).collect() } else { styledesc::kmedoids(&points, STATS_IMAGES).medoids };
        let sampled = if chosen_found > n { format!(" (measured {n} of them, sampled evenly)") } else { String::new() };
        if !sampled.is_empty() {
            notes.push(format!("tier {} has {chosen_found} images{sampled}", tier.number()));
        }
        for (k, &i) in send_idx.iter().enumerate() {
            let p = &pool[i];
            chosen.push(ChosenRef {
                path: p.cand.display.clone(),
                origin: p.cand.origin,
                tier: tier.number(),
                reason: send_why[k].clone(),
                sha256: p.file_sha.clone(),
                sent_to_provider: true,
                used_for_statistics: stats_idx.contains(&i),
            });
        }
        for (rank, &i) in stats_idx.iter().enumerate() {
            if send_idx.contains(&i) {
                continue;
            }
            let p = &pool[i];
            chosen.push(ChosenRef {
                path: p.cand.display.clone(),
                origin: p.cand.origin,
                tier: tier.number(),
                reason: format!("statistics only: medoid {} of {} for {} images (local, never sent)", rank + 1, stats_idx.len(), label),
                sha256: p.file_sha.clone(),
                sent_to_provider: false,
                used_for_statistics: true,
            });
        }
        let items: Vec<&Analysis> = stats_idx.iter().map(|&i| &pool[i].analysis).collect();
        let stats = styledesc::aggregate(&items);
        paragraph = Some(styledesc::style_paragraph(&subject_phrase(tier, family, ctx.pinned), &stats));
        statistics = Some(stats);
        let model = DriftModel::fit(&points);
        drift = Some(DriftSummary {
            images: model.images,
            p95_leave_one_out: (model.p95_leave_one_out * 1000.0).round() / 1000.0,
            threshold: (model.threshold * 1000.0).round() / 1000.0,
            minimum_threshold: styledesc::MIN_DRIFT_THRESHOLD,
        });
        drift_model = Some(model);
    } else {
        notes.push(format!("no tier has {MIN_TIER_IMAGES} usable images: nothing is chosen automatically and no style statistics exist"));
    }

    Resolved {
        report: StyleReport {
            family,
            chosen_tier: chosen_tier.map(Tier::number),
            tiers: summaries,
            send_limit: ctx.max_send,
            method: format!(
                "descriptors (Lab mean/std, saturation, Sobel edge density, alpha coverage, dark-outline share, 6-bin hue histogram) are standardised; deterministic k-medoids (PAM, no random start, candidates ordered by content hash) picks up to {} images to send and up to {STATS_IMAGES} for statistics",
                ctx.max_send
            ),
            chosen,
            statistics,
            style_paragraph: paragraph,
            drift,
            notes,
        },
        drift: drift_model,
        tier: chosen_tier,
        subject: chosen_tier.map(|t| subject_phrase(t, family, ctx.pinned)),
    }
}

fn sorted_names(fs: &ProjectFs, dir: &str) -> Vec<String> {
    let mut v = fs.list_names(dir).unwrap_or_default();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_blocks_pair_name_and_texture() {
        let text = r#"
spriteTypes = {
    spriteType = { name = "GFX_idea_a" texturefile = "gfx/interface/ideas/a.dds" }
    SpriteType = {
        # a comment with name = "no"
        textureFile = "gfx/interface/ideas/b.dds"
        name = "GFX_idea_b"
        noOfFrames = 2
    }
    frameAnimatedSpriteType = { name = "GFX_anim" texturefile = "gfx/x.dds" }
    spriteType = { name = "GFX_missing_texture" }
}"#;
        let got = parse_sprites(text);
        assert_eq!(
            got,
            vec![
                ("GFX_idea_a".to_string(), "gfx/interface/ideas/a.dds".to_string()),
                ("GFX_idea_b".to_string(), "gfx/interface/ideas/b.dds".to_string()),
                ("GFX_anim".to_string(), "gfx/x.dds".to_string())
            ]
        );
    }

    #[test]
    fn every_family_but_gui_texture_has_a_shelf_folder() {
        for f in Family::ALL {
            assert_eq!(shelf_folder(*f).is_none(), *f == Family::GuiTexture, "{f}");
        }
        assert_eq!(shelf_folder(Family::Focus), Some("icons/national_focus"));
    }

    #[test]
    fn name_rules_follow_the_kit_patterns() {
        let d = dir("x", true).pre(&["decision_"]).not_pre(&["decision_cat"]);
        assert!(d.accepts("decision_gold") && !d.accepts("decision_category_x") && !d.accepts("goal_x"));
        let f = dir("x", true).pre(&["goal", "focus"]).not_has(&["untitl"]);
        assert!(f.accepts("focus_a") && f.accepts("goal_b") && !f.accepts("focus_untitled"));
    }
}
