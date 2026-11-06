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
//! The first tier with at least three usable images wins. Its images are measured ([`crate::styledesc`]) and up to
//! 24 k-medoids feed the style statistics, so the measured style does not depend on the request. The images sent to
//! the provider depend on the request ([`crate::refselect`]): nation-specific art is left out unless the brief names
//! the country, files that match the brief's theme are drawn beside style-typical ones, and a hash of the request
//! seeds the draws. The plan reports each choice and each exclusion with its tier, reason and hash. Vanilla art may
//! always be sent; the tool never copies it into the mod.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::diversity;
use crate::family::{Family, GuiTextureKind};
use crate::fsio::{display_path, read_limited, ProjectFs};
use crate::hashing::sha256_hex;
use crate::pixels::{decode, DecodeLimits};
use crate::profile::Profile;
use crate::refs::{classify_project_path, RefOrigin, MAX_REFERENCE_BYTES};
use crate::refselect::{self, Brief, NationMark, Nations, Rng};
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
/// Most excluded candidates listed in a report.
pub const MAX_EXCLUDED_LISTED: usize = 40;
/// Theme matches considered per sent theme slot.
const THEME_WINDOW_PER_SLOT: usize = 4;
/// Style-typical images considered per sent typical slot, at least (the window is at least the more typical half).
const TYPICAL_WINDOW_PER_SLOT: usize = 6;
/// Style-typical candidates compared per pick (the one furthest from the images already chosen wins).
const TYPICAL_LOOKAHEAD: usize = 6;
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

/// A candidate of the used tier that the request left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExcludedRef {
    /// Project-relative path, or an absolute path inside the installed game.
    pub path: String,
    /// Why it was left out.
    pub reason: String,
}

/// How the near-duplicate check (`GFX-NEAR-DUPLICATE`) is set up for the family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NearDuplicateSummary {
    /// Images of the used tier that were hashed.
    pub images: usize,
    /// Distinct pairs measured.
    pub pairs: usize,
    /// dHash distance (of 128 bits) at or below which two outputs are near-duplicates: the 1st percentile of the
    /// distances between distinct images of the set.
    pub threshold_bits: u32,
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
    /// Near-duplicate check set-up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near_duplicate: Option<NearDuplicateSummary>,
    /// The sub-family the references were drawn from, when the request had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subfamily: Option<SubfamilyUse>,
    /// Candidates of the used tier the request left out, with the reason (at most [`MAX_EXCLUDED_LISTED`]; `notes`
    /// gives the full count).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<ExcludedRef>,
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
    /// The near-duplicate threshold measured on the used tier, in dHash bits.
    pub near_duplicate_bits: Option<u32>,
}

impl Resolved {
    /// Paths of the references to send, in order.
    pub fn send_paths(&self) -> Vec<String> {
        self.report.chosen.iter().filter(|c| c.sent_to_provider).map(|c| c.path.clone()).collect()
    }

    /// Mark every sent reference of `tier` as not sent.
    pub fn drop_tier(&mut self, tier: u8, why: &str) {
        for c in self.report.chosen.iter_mut().filter(|c| c.sent_to_provider && c.tier == tier) {
            c.sent_to_provider = false;
            c.reason = format!("{} (not sent: {why})", c.reason);
        }
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
    /// The kind of a `gui_texture` (button, frame, panel, background): it picks the vanilla references. Without
    /// it a GUI texture has no automatic reference.
    pub gui_kind: Option<GuiTextureKind>,
    /// The request the references are for. Without one, the send set is the style-typical k-medoids of the
    /// family's generic art.
    pub request: Option<RequestKey<'a>>,
}

/// What the per-request selection reads from a plan.
#[derive(Debug, Clone, Copy)]
pub struct RequestKey<'a> {
    /// Asset name.
    pub asset: &'a str,
    /// The brief.
    pub brief: &'a str,
    /// The sub-family to draw from (named in the request or inferred from the brief).
    pub subfamily: Option<&'a str>,
}

/// How a request's sub-family limited the references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SubfamilyUse {
    /// Sub-family id.
    pub id: String,
    /// True when the references were drawn from the sub-family alone.
    pub used: bool,
    /// Eligible files of the used tier in the sub-family.
    pub files: usize,
    /// Why, for a person.
    pub note: String,
}

// ---- family tables ----

struct Dir {
    path: &'static str,
    recursive: bool,
    prefixes: &'static [&'static str],
    not_prefix: &'static [&'static str],
    not_contains: &'static [&'static str],
    not_suffix: &'static [&'static str],
    contains: &'static [&'static str],
    suffixes: &'static [&'static str],
    /// Folders below `path` a file must lie under (any one of them, compared case-blind), empty for any.
    under: &'static [&'static str],
    /// Folders below `path` whose files are left out.
    skip: &'static [&'static str],
    /// Smallest width and height a file must have (0: any). Read from the header.
    min_side: u32,
}

fn dir(path: &'static str, recursive: bool) -> Dir {
    Dir {
        path,
        recursive,
        prefixes: &[],
        not_prefix: &[],
        not_contains: &[],
        not_suffix: &[],
        contains: &[],
        suffixes: &[],
        under: &[],
        skip: &[],
        min_side: 0,
    }
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
    fn under(mut self, p: &'static [&'static str]) -> Dir {
        self.under = p;
        self
    }
    fn skip(mut self, p: &'static [&'static str]) -> Dir {
        self.skip = p;
        self
    }
    /// True when the folders between `path` and the file (`folders`, lower case) meet `under` and `skip`.
    fn folders_ok(&self, folders: &[String]) -> bool {
        (self.under.is_empty() || folders.iter().any(|f| self.under.contains(&f.as_str())))
            && !folders.iter().any(|f| self.skip.contains(&f.as_str()))
    }
    fn has(mut self, p: &'static [&'static str]) -> Dir {
        self.contains = p;
        self
    }
    fn end(mut self, p: &'static [&'static str]) -> Dir {
        self.suffixes = p;
        self
    }
    fn min(mut self, side: u32) -> Dir {
        self.min_side = side;
        self
    }
    fn accepts(&self, stem: &str) -> bool {
        (self.prefixes.is_empty() || self.prefixes.iter().any(|p| stem.starts_with(p)))
            && (self.contains.is_empty() || self.contains.iter().any(|p| stem.contains(p)))
            && (self.suffixes.is_empty() || self.suffixes.iter().any(|p| stem.ends_with(p)))
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
        GuiTexture | AnimBlendGlow | AnimBlendKeyframes | AnimScroll | AnimSpin => return None,
    })
}

/// Texture folders (game-relative, also used inside a mod) and file-name rules of a family. Each family's rules
/// keep to the files that show its one vanilla form (measured on HOI4 1.19.4.1): commander traits leave out the
/// operative traits (a shield plaque without a ribbon) and the placeholders, the operation phase icon and picture keep
/// to their own sizes, and a GUI texture takes the files of its kind.
fn texture_dirs(f: Family, gui_kind: Option<GuiTextureKind>) -> Vec<Dir> {
    use Family::*;
    match f {
        GuiTexture => match gui_kind {
            None => vec![],
            // 34 files: rounded bevelled plates
            Some(GuiTextureKind::Button) => vec![dir("gfx/interface", false).pre(&["button_"]), dir("gfx/interface/buttons", false)],
            // 50 files: borders with an empty middle, without the multi-frame strips
            Some(GuiTextureKind::Frame) => vec![dir("gfx/interface", false).has(&["_frame"]).not_has(&["frames", "frame_bg"])],
            // 48 files: the stretchable window and dialog plates
            Some(GuiTextureKind::Panel) => vec![dir("gfx/interface/tiles", false).pre(&["tiled_"])],
            // 71 files: whole window backgrounds of at least 256 px a side
            Some(GuiTextureKind::Background) => vec![dir("gfx/interface", false).end(&["_bg"]).min(256)],
        },
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
        // the base side is a greyscale portrait or bust; the _improved and _max variants are its coloured, stronger states
        BopSide => vec![dir("gfx/interface/bop", true).not_has(&["_improved", "rubble"]).not_end(&["_max"])],
        Medal => vec![dir("gfx/interface/medals", true)],
        AgencyUpgrade => vec![dir("gfx/interface/operatives/icons", true)],
        AgencyLogo => vec![dir("gfx/interface/operatives/agencies", true)],
        // the 59x58 phase icons (phases_small); the 85x85 and 48x48 operation icons are a different, coloured form
        OperationIcon => vec![dir("gfx/interface/operations", true).pre(&["phase_"])],
        CommanderTrait => {
            vec![dir("gfx/interface/traits", false).pre(&["trait_"]).not_pre(&["trait_operative_", "trait_add_new", "trait_unknown"])]
        }
        PersonalTrait => vec![dir("gfx/interface/traits/personal", true)],
        Doctrine => vec![dir("gfx/interface/doctrines", true).pre(&["doctrine_"])],
        ReportEvent => vec![dir("gfx/event_pictures", true).pre(&["report_event"])],
        NewsEvent => vec![dir("gfx/event_pictures", true).pre(&["news_event"])],
        DecisionCategoryPicture => vec![dir("gfx/interface/decisions", true).pre(&["decision_cat_"])],
        OperationPicture => vec![dir("gfx/interface/operations", true).pre(&["phase_"]).not_has(&["_small"])],
        Technology => vec![dir("gfx/interface/technologies", true)],
        Achievement => vec![dir("gfx/achievements", false).not_has(&["_grey", "_not_eligible"])],
        // Each portrait family keeps to the files with its own backdrop (measured on 1.19.4.1, see
        // validate::portrait_backdrop): scientists are the gfx/leaders/scientists folder (pale blue-grey), operatives
        // La Resistance's portrait_*_operative_* files (163 at 156x210, dark warm charcoal with a spotlight behind the
        // head) plus a mod's own operatives folder, and leaders and commanders the rest of gfx/leaders (one pale
        // cream-grey template). The base game's operative_unknown silhouette is a placeholder and belongs to none.
        LeaderPortrait | CommanderPortrait => {
            vec![dir("gfx/leaders", true).skip(&["scientists", "operatives", "placeholder"]).not_has(&["_operative_"]).not_pre(&["operative_"])]
        }
        OperativePortrait => {
            vec![dir("gfx/leaders", true).has(&["_operative_"]).skip(&["scientists"]), dir("gfx/leaders", true).under(&["operatives"])]
        }
        ScientistPortrait => vec![dir("gfx/leaders", true).under(&["scientists"])],
        AdvisorCard => vec![dir("gfx/interface/ideas", true)],
        Flag => vec![dir("gfx/flags", false)],
        CounterLarge => vec![dir("gfx/interface/counters/divisions_large", true)],
        CounterSmall => vec![dir("gfx/interface/counters/divisions_small", true)],
        DivisionEmblem => vec![dir("gfx/interface/counters/division_templates_large", true)],
        Building => vec![dir("gfx/interface/buildings", true)],
        FactionLogo => vec![dir("gfx/interface/factions/faction_logos", true).not_end(&["_miniature"])],
        ModifierStatIcon => vec![dir("gfx/interface/modifiers", true).pre(&["modifier_"])],
        SuperEvent | AnimBlendGlow | AnimBlendKeyframes | AnimScroll | AnimSpin => vec![],
    }
}

/// The strip sizes whose files show a family's one form, where the family accepts other sizes too. A landmark's
/// 64x64 frames are painted miniatures, its 32x32 frames flat orange pictograms, so the references keep to the
/// 128x64 strips the family generates.
fn reference_sizes(f: Family) -> Option<&'static [[u32; 2]]> {
    match f {
        Family::Building => Some(&[[128, 64]]),
        _ => None,
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
            let big_enough = d.min_side == 0 || header_size(&p).is_some_and(|s| s[0] >= d.min_side && s[1] >= d.min_side);
            let folders: Vec<String> = p
                .strip_prefix(base.join(d.path))
                .ok()
                .and_then(|r| r.parent())
                .map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect())
                .unwrap_or_default();
            if exts.contains(&ext_of(&p).as_str())
                && d.accepts(&stem_of(&p))
                && d.folders_ok(&folders)
                && big_enough
                && size_ok(&p, sizes)
                && !files.contains(&p)
            {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

/// The folders the game loads art from: the game folder, then each DLC folder under `dlc/` in name order. A DLC keeps
/// its art in the same layout (`dlc/<dlc>/gfx/...`), and some families live mostly there: vanilla's 163 operative
/// portraits at 156x210 are La Resistance's (`dlc028_la_resistance/gfx/leaders/<region>/portrait_*_operative_*`).
/// A DLC packed as a zip archive is not read.
pub fn game_roots(game: &Path) -> Vec<PathBuf> {
    let mut roots = vec![game.to_path_buf()];
    let mut dlcs: Vec<PathBuf> = std::fs::read_dir(game.join("dlc"))
        .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect())
        .unwrap_or_default();
    dlcs.sort();
    roots.extend(dlcs);
    roots
}

/// [`scan_dirs`] over every root of [`game_roots`], sorted by path.
fn scan_game(game: &Path, dirs: &[Dir], sizes: &[[u32; 2]], exts: &[&str]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = game_roots(game).iter().flat_map(|root| scan_dirs(root, dirs, sizes, exts)).collect();
    files.sort();
    files.dedup();
    files
}

/// The family's vanilla texture files in the installed game and its DLC folders ([`game_roots`]), limited to `sizes`
/// (empty: any size), sorted by path.
/// The same folders and name rules as the vanilla style tier, so the review's vanilla-match check and the style
/// sources agree on what a family's vanilla art is.
pub fn vanilla_files(game: &Path, family: Family, sizes: &[[u32; 2]]) -> Vec<PathBuf> {
    scan_game(game, &texture_dirs(family, None), sizes, &RASTER)
}

/// The vanilla GUI texture files of one texture kind, as [`vanilla_files`] (any size).
pub fn vanilla_kind_files(game: &Path, kind: GuiTextureKind) -> Vec<PathBuf> {
    scan_game(game, &texture_dirs(Family::GuiTexture, Some(kind)), &[], &RASTER)
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
        files.into_iter().map(|abs| Cand { display: unify(&display_path(&abs)), origin: RefOrigin::InstalledGame, abs }).collect();
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

#[derive(Clone)]
struct Pooled {
    cand: Cand,
    file_sha: String,
    analysis: Analysis,
    dhash: u128,
}

/// Read, decode and measure one candidate.
fn measure(c: &Cand) -> Option<Pooled> {
    let bytes = read_limited(&c.abs, MAX_REFERENCE_BYTES).ok()?;
    let (img, _) = decode(&bytes, c.abs.file_name().map(|n| n.to_string_lossy().to_string()).as_deref(), DecodeLimits::default()).ok()?;
    let analysis = styledesc::analyse(&img)?;
    Some(Pooled { cand: c.clone(), file_sha: sha256_hex(&bytes), analysis, dhash: diversity::dhash(&img) })
}

/// Measure candidates: sample evenly beyond the cap, drop what cannot be read, drop duplicates, order by content hash.
fn build_pool(cands: &[Cand], cap: usize, notes: &mut Vec<String>) -> Vec<Pooled> {
    let picked: Vec<&Cand> = if cands.len() > cap { (0..cap).map(|i| &cands[i * cands.len() / cap]).collect() } else { cands.iter().collect() };
    let mut pool: Vec<Pooled> = Vec::new();
    let mut unreadable = 0usize;
    for c in picked {
        let Some(p) = measure(c) else {
            unreadable += 1;
            continue;
        };
        if pool.iter().any(|q| q.file_sha == p.file_sha) {
            continue;
        }
        pool.push(p);
    }
    if unreadable > 0 {
        notes.push(format!("{unreadable} candidate file(s) could not be decoded or were empty and were left out"));
    }
    pool.sort_by(|a, b| a.file_sha.cmp(&b.file_sha).then(a.cand.display.cmp(&b.cand.display)));
    pool
}

/// What the per-request selection needs to know about the used tier.
struct Selection<'s> {
    tier: Tier,
    family: Family,
    label: &'s str,
    found: usize,
    sampled: &'s str,
    model: &'s DriftModel,
    send_slots: usize,
}

/// Folders of a candidate below `gfx/` (or all of them for a file outside it), for tag-named folders.
fn rel_dirs(display: &str) -> Vec<&str> {
    let parts: Vec<&str> = display.split('/').collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    match dirs.iter().rposition(|d| d.eq_ignore_ascii_case("gfx")) {
        Some(i) => dirs[i + 1..].to_vec(),
        None => dirs.to_vec(),
    }
}

/// The original-case file stem.
fn raw_stem(c: &Cand) -> String {
    c.abs.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// File-name tokens plus the name of the folder that holds the file.
fn file_tokens(c: &Cand) -> Vec<String> {
    let mut t = refselect::tokens(&raw_stem(c));
    if let Some(dir) = rel_dirs(&c.display).last() {
        t.extend(refselect::tokens(dir));
    }
    t
}

/// The send set of an automatic tier (2 to 4) for one request.
fn select(
    sel: &Selection,
    ctx: &SourceCtx,
    cands: &[Cand],
    pool: &[Pooled],
    excluded: &mut Vec<ExcludedRef>,
    sub_use: &mut Option<SubfamilyUse>,
    notes: &mut Vec<String>,
) -> Vec<(Pooled, String)> {
    let Selection { tier, family, label, found, sampled, model, send_slots } = *sel;
    let tier_name = match tier {
        Tier::ModAssetFolderCopy => "mod asset folder copy",
        Tier::ModAssets => "mod asset",
        _ => "vanilla",
    };
    let nations = Nations::load(ctx.game_dir);
    let brief = ctx.request.map(|r| Brief::new(family, r.asset, r.brief, &nations));
    let empty = Brief { words: vec![], themes: Default::default(), nations: Default::default(), seed: 0 };
    let b = brief.as_ref().unwrap_or(&empty);

    // nation-specific files are left out unless the brief names the country
    let marks: Vec<Option<NationMark>> = cands.iter().map(|c| nations.nation_of(&raw_stem(c), &rel_dirs(&c.display))).collect();
    let mut eligible: Vec<usize> = Vec::new();
    let mut left_out: Vec<(usize, String)> = Vec::new();
    for (i, m) in marks.iter().enumerate() {
        match m {
            Some(m) if !Nations::allows(b, m) => left_out.push((i, format!("nation-specific: {}, and the brief names no such country", m.reason))),
            _ => eligible.push(i),
        }
    }
    let need = MIN_TIER_IMAGES.max(send_slots);
    if eligible.len() < need && !left_out.is_empty() {
        notes.push(format!(
            "only {} of {} {label} images are not nation-specific (fewer than {need}), so nation-specific files stay in the selection",
            eligible.len(),
            cands.len()
        ));
        eligible = (0..cands.len()).collect();
        left_out.clear();
    }
    if !left_out.is_empty() {
        notes.push(format!(
            "{} of {} {label} images were left out as nation-specific ({}); the first {} are listed under excluded",
            left_out.len(),
            cands.len(),
            nations.source,
            left_out.len().min(MAX_EXCLUDED_LISTED)
        ));
    }
    excluded.extend(left_out.iter().take(MAX_EXCLUDED_LISTED).map(|(i, why)| ExcludedRef { path: cands[*i].display.clone(), reason: why.clone() }));

    // the request's sub-family: only its files, unless it has too few, then the whole family
    if let (Some(id), Some(fam)) = (ctx.request.and_then(|r| r.subfamily), crate::subfamily::for_family(family)) {
        let member = |i: usize| {
            let dirs = rel_dirs(&cands[i].display);
            fam.member(&raw_stem(&cands[i]), &dirs[dirs.len().saturating_sub(1)..]) == Some(id)
        };
        let (inside, outside): (Vec<usize>, Vec<usize>) = eligible.iter().partition(|&&i| member(i));
        if inside.len() >= need {
            notes.push(format!(
                "{} of the {} eligible {label} images are outside the sub-family {id} and were left out",
                outside.len(),
                eligible.len()
            ));
            excluded.extend(
                outside
                    .iter()
                    .take(MAX_EXCLUDED_LISTED / 2)
                    .map(|&i| ExcludedRef { path: cands[i].display.clone(), reason: format!("outside the sub-family {id}") }),
            );
            *sub_use = Some(SubfamilyUse {
                id: id.to_string(),
                used: true,
                files: inside.len(),
                note: format!("references drawn from the {} eligible {label} images of the sub-family {id}", inside.len()),
            });
            eligible = inside;
        } else {
            let note = format!(
                "the sub-family {id} has only {} eligible {label} images here (fewer than {need}), so the references come from the whole family",
                inside.len()
            );
            notes.push(note.clone());
            *sub_use = Some(SubfamilyUse { id: id.to_string(), used: false, files: inside.len(), note });
        }
    }

    // the style-typical images among the eligible ones: generic files first, then by distance from the style
    let eligible_set: Vec<&Cand> = eligible.iter().map(|&i| &cands[i]).collect();
    let own_pool;
    let typical_pool: &[Pooled] = if eligible.len() == cands.len() {
        pool
    } else {
        let list: Vec<Cand> = eligible_set.iter().map(|c| (*c).clone()).collect();
        own_pool = build_pool(&list, POOL_CAP, &mut Vec::new());
        &own_pool
    };
    if typical_pool.is_empty() {
        return Vec::new();
    }
    let Some(brief) = brief else {
        // no request: the k-medoids of the eligible images, as a stable overview
        let points: Vec<Vector> = typical_pool.iter().map(|p| p.analysis.v).collect();
        let m = styledesc::kmedoids(&points, send_slots.min(points.len()));
        return m
            .medoids
            .iter()
            .zip(&m.sizes)
            .enumerate()
            .map(|(rank, (&i, size))| {
                (typical_pool[i].clone(), format!("{tier_name}: medoid {} of {found} {label} images{sampled} (cluster of {size})", rank + 1))
            })
            .collect();
    };

    let mut rng = Rng::new(brief.seed);
    let mut out: Vec<(Pooled, String)> = Vec::new();

    // theme matches: about half the slots, drawn from the best matches
    let mut scored: Vec<(u32, u64, usize, Vec<String>)> = eligible
        .iter()
        .map(|&i| {
            let toks = file_tokens(&cands[i]);
            let (mut score, mut terms) = brief.score(&toks);
            if let Some(m) = &marks[i] {
                if Nations::allows(&brief, m) {
                    score += 3;
                    terms.push(format!("country {}", m.tags.iter().next().cloned().or(m.region.clone()).unwrap_or_default()));
                }
            }
            (score, rng.next_u64(), i, terms)
        })
        .filter(|s| s.0 > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let theme_slots = if scored.is_empty() { 0 } else { send_slots.div_ceil(2) };
    let window = scored.len().min((THEME_WINDOW_PER_SLOT * theme_slots).max(8));
    let matches = scored.len();
    let mut theme: Vec<_> = scored.into_iter().take(window).collect();
    rng.shuffle(&mut theme);
    for (score, _, i, terms) in theme {
        if out.len() >= theme_slots {
            break;
        }
        let Some(p) = measure(&cands[i]) else { continue };
        if out.iter().any(|(q, _)| q.file_sha == p.file_sha) {
            continue;
        }
        let d = model.check(&p.analysis.v);
        if d.drifted {
            if excluded.len() < MAX_EXCLUDED_LISTED * 2 {
                excluded.push(ExcludedRef {
                    path: p.cand.display.clone(),
                    reason: format!("theme match left out: off the family's style (z {:.2} above {:.2})", d.z_rms, d.threshold),
                });
            }
            continue;
        }
        let why = format!(
            "{tier_name}: theme match ({}; score {score}), drawn from the {window} best of {matches} matching {label} images by the request seed",
            terms.join(", ")
        );
        out.push((p, why));
    }

    // style-typical picks for the other slots, drawn from the most typical (generic first)
    let mut ranked: Vec<(bool, f64, usize)> =
        typical_pool.iter().enumerate().map(|(i, p)| (!refselect::is_generic(&file_tokens(&p.cand)), model.check(&p.analysis.v).z_rms, i)).collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).then(a.2.cmp(&b.2)));
    let left = send_slots.saturating_sub(out.len());
    // the more typical half of the eligible images (generic first), so the draw keeps the family's palette spread
    let window = ranked.len().min((TYPICAL_WINDOW_PER_SLOT * left).max(ranked.len() / 2).max(8));
    let generic = ranked.iter().take(window).filter(|r| !r.0).count();
    let mut typical: Vec<_> = ranked.into_iter().take(window).collect();
    rng.shuffle(&mut typical);
    // each pick takes, of the next few in the seeded order, the one furthest in style from the images already chosen,
    // so a set spreads over the family's frames and palettes instead of four near neighbours
    typical.retain(|t| !out.iter().any(|(q, _)| q.file_sha == typical_pool[t.2].file_sha));
    while out.len() < send_slots && !typical.is_empty() {
        let look = typical.len().min(TYPICAL_LOOKAHEAD);
        let chosen_z: Vec<Vector> = out.iter().map(|(q, _)| model.std.z(&q.analysis.v)).collect();
        let far = |t: &(bool, f64, usize)| {
            let z = model.std.z(&typical_pool[t.2].analysis.v);
            chosen_z.iter().map(|c| c.iter().zip(&z).map(|(a, b)| (a - b).powi(2)).sum::<f64>()).fold(f64::INFINITY, f64::min)
        };
        let pick = (0..look).fold(0, |best, k| if far(&typical[k]) > far(&typical[best]) + 1e-12 { k } else { best });
        let (_, z, i) = typical.remove(pick);
        let p = &typical_pool[i];
        if out.iter().any(|(q, _)| q.file_sha == p.file_sha) {
            continue;
        }
        let kind = if generic == window { "generic " } else { "" };
        let why = format!(
            "{tier_name}: style-typical, drawn from the {window} most typical {kind}{label} images of {} by the request seed, the furthest in style from the others of {TYPICAL_LOOKAHEAD} looked at (z {z:.2})",
            typical_pool.len()
        );
        out.push((p.clone(), why));
    }
    out
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
    let mut chosen_cands: Vec<Cand> = Vec::new();
    // a pinned style with fewer than three images has no statistics but is always sent
    let mut pinned_small: Vec<Cand> = Vec::new();

    let effective_sizes = ctx.profile.accepted_strips();
    // a family whose canvas the mod defines takes vanilla references of any size; a family with several accepted
    // sizes of different forms takes references of its one form only
    let vanilla_sizes: Vec<[u32; 2]> = match (reference_sizes(family), ctx.builtin) {
        (Some(form), _) => form.to_vec(),
        (None, Some(b)) if b.needs_mod_canvas() => Vec::new(),
        (None, Some(b)) if !b.accepted_strips().is_empty() => b.accepted_strips(),
        _ => effective_sizes.clone(),
    };

    for tier in Tier::ALL {
        let mut summary =
            TierSummary { tier: tier.number(), name: tier.label().into(), scanned: false, found: 0, chosen: false, note: String::new() };
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
            Tier::ModAssetFolderCopy => {
                match shelf_folder(family) {
                    None => {
                        summary.note = "this family has no shelf folder".into();
                        vec![]
                    }
                    Some(shelf) => {
                        let mut sizes = vanilla_sizes.clone();
                        if reference_sizes(family).is_none() && !sizes.is_empty() {
                            for s in &effective_sizes {
                                if !sizes.contains(s) {
                                    sizes.push(*s);
                                }
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
                }
            }
            Tier::ModAssets => {
                let dirs: Vec<Dir> = texture_dirs(family, ctx.gui_kind).into_iter().filter(|d| ctx.fs.resolve(d.path).is_ok()).collect();
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
                    let list = game_cands(scan_game(game, &texture_dirs(family, ctx.gui_kind), &vanilla_sizes, &RASTER));
                    if family == Family::GuiTexture && ctx.gui_kind.is_none() {
                        summary.note =
                            "a GUI texture takes vanilla references by its kind: pass texture_kind (button, frame, panel or background)".into();
                    } else if list.is_empty() {
                        summary.note = "the game folder has no vanilla art of this family".into();
                    }
                    list
                }
            },
        };
        summary.found = cands.len();
        // the kit's shelf copy is vanilla art, mostly nation-specific: for a request, it counts only with enough files
        // the brief may use, otherwise the request falls through to the next tier instead of getting other countries'
        // art (the mod's own art in tier 3 keeps its tags: they are the mod's style)
        let shelf_short = (tier == Tier::ModAssetFolderCopy && chosen_tier.is_none() && cands.len() >= MIN_TIER_IMAGES)
            .then(|| ctx.request.map(|r| (r, Nations::load(ctx.game_dir))))
            .flatten()
            .and_then(|(r, nations)| {
                let b = Brief::new(family, r.asset, r.brief, &nations);
                let usable =
                    cands.iter().filter(|c| nations.nation_of(&raw_stem(c), &rel_dirs(&c.display)).is_none_or(|m| Nations::allows(&b, &m))).count();
                (usable < MIN_TIER_IMAGES.max(DEFAULT_SEND)).then_some(usable)
            });
        if let Some(usable) = shelf_short {
            summary.note = format!(
                "only {usable} of {} image(s) are not specific to a country the brief does not name (at least {} are needed), so the next tier is used",
                cands.len(),
                MIN_TIER_IMAGES.max(DEFAULT_SEND)
            );
        } else if chosen_tier.is_none() && !cands.is_empty() {
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
                    summary.note = format!(
                        "used: {} image(s) measured{}",
                        pool.len(),
                        if cands.len() > pool.len() { format!(" of {} found", cands.len()) } else { String::new() }
                    );
                    chosen_tier = Some(tier);
                    chosen_found = cands.len();
                    chosen_cands = cands.clone();
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

    let mut excluded: Vec<ExcludedRef> = Vec::new();
    let mut subfamily_use: Option<SubfamilyUse> = None;
    let mut near_duplicate = None;
    let mut near_duplicate_bits = None;
    if let Some(tier) = chosen_tier {
        let pool = &chosen_pool;
        let points: Vec<Vector> = pool.iter().map(|p| p.analysis.v).collect();
        let n = pool.len();
        let label = family.as_str();
        let sampled = if chosen_found > n { format!(" ({n} sampled evenly)") } else { String::new() };
        if !sampled.is_empty() {
            notes.push(format!("tier {} has {chosen_found} images; {n} were measured, sampled evenly by path", tier.number()));
        }
        let model = DriftModel::fit(&points);
        let stats_idx: Vec<usize> = if n <= STATS_IMAGES { (0..n).collect() } else { styledesc::kmedoids(&points, STATS_IMAGES).medoids };
        // what is sent: the pinned style as its author ordered it, else the per-request selection
        let sends: Vec<(Pooled, String)> = if tier == Tier::PinnedStyle {
            let (idx, why): (Vec<usize>, Vec<String>) = if n <= send_slots {
                let order: Vec<usize> = ctx
                    .pinned
                    .map(|s| s.references.iter().filter_map(|r| pool.iter().position(|p| p.cand.display == unify(&r.path))).collect())
                    .unwrap_or_default();
                let why = (0..order.len())
                    .map(|i| format!("pinned style '{}': reference {} of {}", ctx.pinned.map(|s| s.name.as_str()).unwrap_or(""), i + 1, order.len()))
                    .collect();
                (order, why)
            } else {
                let m = styledesc::kmedoids(&points, send_slots.min(n));
                let why = m
                    .sizes
                    .iter()
                    .enumerate()
                    .map(|(rank, size)| format!("pinned style: medoid {} of {} references (cluster of {size})", rank + 1, n))
                    .collect();
                (m.medoids, why)
            };
            idx.into_iter().zip(why).map(|(i, w)| (pool[i].clone(), w)).collect()
        } else {
            let sel = Selection { tier, family, label, found: chosen_found, sampled: &sampled, model: &model, send_slots };
            select(&sel, ctx, &chosen_cands, pool, &mut excluded, &mut subfamily_use, &mut notes)
        };
        let stats_sha: Vec<&str> = stats_idx.iter().map(|&i| pool[i].file_sha.as_str()).collect();
        for (p, why) in &sends {
            chosen.push(ChosenRef {
                path: p.cand.display.clone(),
                origin: p.cand.origin,
                tier: tier.number(),
                reason: why.clone(),
                sha256: p.file_sha.clone(),
                sent_to_provider: true,
                used_for_statistics: stats_sha.contains(&p.file_sha.as_str()),
            });
        }
        for (rank, &i) in stats_idx.iter().enumerate() {
            let p = &pool[i];
            if sends.iter().any(|(s, _)| s.file_sha == p.file_sha) {
                continue;
            }
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
        paragraph = Some(styledesc::style_paragraph_for(&subject_phrase(tier, family, ctx.pinned), &stats, Some(family)));
        statistics = Some(stats);
        drift = Some(DriftSummary {
            images: model.images,
            p95_leave_one_out: (model.p95_leave_one_out * 1000.0).round() / 1000.0,
            threshold: (model.threshold * 1000.0).round() / 1000.0,
            minimum_threshold: styledesc::MIN_DRIFT_THRESHOLD,
        });
        drift_model = Some(model);
        let hashes: Vec<u128> = pool.iter().map(|p| p.dhash).collect();
        if let Some((bits, pairs)) = diversity::threshold_from(&hashes) {
            near_duplicate = Some(NearDuplicateSummary { images: n, pairs, threshold_bits: bits });
            near_duplicate_bits = Some(bits);
        }
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
                "descriptors (Lab mean/std, saturation, Sobel edge density, alpha coverage, dark-outline share, 6-bin hue histogram) are standardised; deterministic k-medoids (PAM, no random start, candidates ordered by content hash) picks up to {STATS_IMAGES} images for statistics and drift. Up to {} images are sent: a pinned style as ordered; otherwise nation-specific files are left out unless the brief names the country, about half the slots are drawn from the files whose names best match the brief's words and themes and the rest from the family's most style-typical generic files, all seeded by a hash of the family, asset and brief{}",
                ctx.max_send,
                if ctx.request.is_none() { " (no request here: the style-typical k-medoids of the generic files are shown)" } else { "" }
            ),
            chosen,
            statistics,
            style_paragraph: paragraph,
            drift,
            near_duplicate,
            subfamily: subfamily_use,
            excluded,
            notes,
        },
        drift: drift_model,
        tier: chosen_tier,
        subject: chosen_tier.map(|t| subject_phrase(t, family, ctx.pinned)),
        near_duplicate_bits,
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
            // animated strips have no reference shelf: their style is the interface element they sit on
            let none = *f == Family::GuiTexture || f.group() == crate::family::Group::Animation;
            assert_eq!(shelf_folder(*f).is_none(), none, "{f}");
        }
        assert_eq!(shelf_folder(Family::Focus), Some("icons/national_focus"));
    }

    /// Each family keeps to the files of its one vanilla form, and a GUI texture takes the files of its kind.
    #[test]
    fn form_rules_keep_each_family_to_its_vanilla_form() {
        let accepts = |f: Family, kind: Option<GuiTextureKind>, stem: &str| texture_dirs(f, kind).iter().any(|d| d.accepts(stem));
        assert!(accepts(Family::CommanderTrait, None, "trait_panzer_leader"));
        for stem in ["trait_operative_seducer", "trait_add_new", "trait_unknown"] {
            assert!(!accepts(Family::CommanderTrait, None, stem), "{stem}");
        }
        assert!(accepts(Family::OperationIcon, None, "phase_bruneval_attack_small"));
        assert!(!accepts(Family::OperationIcon, None, "infiltrate_naval_forces"), "the 85x85 operation icons are another form");
        assert!(!accepts(Family::OperationPicture, None, "phase_bruneval_attack_small"));
        assert!(accepts(Family::OperationPicture, None, "phase_bruneval_attack"));
        assert!(accepts(Family::BopSide, None, "bop_fin_mannerheim_good_side"));
        for stem in ["bop_fin_mannerheim_good_improved_side", "prc_bop_left_side_max", "bop_fin_bad_rubble_side"] {
            assert!(!accepts(Family::BopSide, None, stem), "{stem}: a coloured state variant");
        }
        assert_eq!(reference_sizes(Family::Building), Some(&[[128, 64]][..]), "the painted 64x64 landmarks, not the orange 32x32 ones");
        // a GUI texture without a kind has no automatic reference; each kind has its own folders and names
        assert!(texture_dirs(Family::GuiTexture, None).is_empty());
        let cases = [
            // every file of gfx/interface/buttons is a button, so the name rule is the top folder's
            (GuiTextureKind::Button, "button_94x31", "tiled_window"),
            (GuiTextureKind::Frame, "diplo_leader_frame", "mic_btn_frames"),
            (GuiTextureKind::Panel, "tiled_window_1b_border", "button_94x31"),
            (GuiTextureKind::Background, "diplo_action_declare_war_bg", "large_flag_frame"),
        ];
        for (kind, yes, no) in cases {
            assert!(accepts(Family::GuiTexture, Some(kind), yes), "{kind}: {yes}");
            assert!(!texture_dirs(Family::GuiTexture, Some(kind))[0].accepts(no), "{kind}: {no}");
        }
        let bg = texture_dirs(Family::GuiTexture, Some(GuiTextureKind::Background));
        assert!(bg.iter().all(|d| d.min_side == 256), "a background is a whole window");
    }

    #[test]
    fn name_rules_follow_the_kit_patterns() {
        let d = dir("x", true).pre(&["decision_"]).not_pre(&["decision_cat"]);
        assert!(d.accepts("decision_gold") && !d.accepts("decision_category_x") && !d.accepts("goal_x"));
        let f = dir("x", true).pre(&["goal", "focus"]).not_has(&["untitl"]);
        assert!(f.accepts("focus_a") && f.accepts("goal_b") && !f.accepts("focus_untitled"));
    }
}
