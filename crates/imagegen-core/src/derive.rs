//! Deriving a style pack from a mod's own art (`pack derive`).
//!
//! Adapting the tool to a mod's house style used to be manual: pick about eight references per family by hand, import
//! them, then write the canvas, form, look and processing switches into `pack.toml`. The gallery's TNO and TFR packs
//! were picked with local scripts, and this module is their method in the tool, so a modder or an agent can do it in
//! one step. Per family:
//!
//! 1. The mod's files in the family's folder ([`specs`]), filtered by name (advisor portraits are not national
//!    spirits, and placeholders are not a house style) and split where two families share a folder.
//! 2. The family's own canvas: the dominant size cluster of those files ([`dominant_cluster`]). Only files in that
//!    cluster are measured, so the references are drawn at the canvas the mod uses.
//! 3. Each file is measured ([`measure`]): a 64-bit dHash and a style vector (alpha shape, 8x8 colour layout, hue
//!    histogram, saturation, brightness, coverage, aspect, edge density and grey share).
//! 4. Files within [`NEAR_COPY_BITS`] dHash bits of any vanilla file of the family are near-copies of vanilla and are
//!    dropped, as are exact duplicates and flat or empty images.
//! 5. The rest are ranked by distance from vanilla's style minus distance from the mod's other files ([`rank`]), so
//!    the top is art that is typical of the mod and unlike vanilla. Portraits are ranked by typicality alone, because
//!    their outliers were placeholders in both mods measured.
//! 6. The picks come from a k-means clustering of the top of the ranking ([`pick_diverse`]): one per cluster first,
//!    at most two per sub-folder, and no two within the near-copy distance of each other. The maintainer rejected a
//!    TFR focus set picked by rank alone, where three of eight icons showed the same landmass.
//! 7. A family is skipped, with the reason, when too few files remain or its art is mostly vanilla's: many
//!    near-copies and the rest in vanilla's palette ([`vanilla_like`]).
//! 8. The measured figures give a draft look line ([`draft_look`]) and the processing switches the art needs
//!    ([`suggest`]).
//!
//! What a measurement cannot see is left to a person or an agent: extremist symbols, recognisable real people and
//! art that is off-topic for the family. The command writes a contact sheet per family for that review and a way to
//! swap any pick for an alternate.
//!
//! Everything here is a pure function of the files read, so a run repeats exactly.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::family::{Family, GuiTextureKind};
use crate::pixels::{decode, DecodeLimits, Rgba};
use crate::refselect::Rng;

/// A mod file within this many bits (of 64) of a vanilla file's dHash is a near-copy of vanilla.
pub const NEAR_COPY_BITS: u32 = 12;
/// Most files measured per family and side. Larger sets are sampled evenly by path.
pub const MAX_FILES: usize = 1500;
/// References picked per family by default.
pub const DEFAULT_PICKS: usize = 8;
/// Most references a family may take (the pack's limit).
pub const MAX_PICKS: usize = crate::pack::MAX_REFS;
/// Fewest usable files (after the filters) a family needs.
pub const MIN_FILES: usize = 8;
/// Neighbours in the density estimates of the ranking.
pub const KNN: usize = 10;
/// Size of the top of the ranking the diverse pick clusters.
pub const POOL: usize = 400;
/// Most picks from one sub-folder (often one country tag).
pub const PER_FOLDER: usize = 2;
/// Most alternates offered per family.
pub const MAX_ALTERNATES: usize = 16;
/// A file belongs to a size cluster when each side is within this share of the cluster's size.
pub const SIZE_TOLERANCE: f64 = 0.25;
/// ... and its aspect within this share of the cluster's aspect.
pub const ASPECT_TOLERANCE: f64 = 0.15;
/// A family is mostly vanilla's when at least this share of its measured files are near-copies of vanilla ...
pub const COPY_SHARE_MAX: f64 = 0.4;
/// ... and the rest are no further from vanilla's palette than [`PALETTE_MIN`] times vanilla's own spread.
///
/// Measured on TFR and TNO (`docs/reference.md`, deriving a pack): TFR's large counters (green glyphs in vanilla's
/// grey box, 35 of 71 near-copies) measure 1.38 and are skipped; TNO's leader portraits (1,831 of 2,456 near-copies,
/// the rest softly colourised photographs) measure 1.48, its commander traits (44 of 92) 1.84, and every other family
/// of either mod with that many near-copies 2.4 or more.
pub const PALETTE_MIN: f64 = 1.45;
/// Where the palette part of the style vector starts (after the alpha shape and the colour layout).
pub const PALETTE_START: usize = 256 + 192;
/// Length of the style vector.
pub const STYLE_DIMS: usize = 256 + 192 + 12 + 8;
/// Files walked per folder at most.
const WALK_BUDGET: usize = 60_000;

/// Name tokens that mark a placeholder rather than the house style.
pub const PLACEHOLDER_TOKENS: [&str; 10] = ["placeholder", "unknown", "missing", "blank", "empty", "dummy", "todo", "tbd", "temp", "debug"];

/// Name tokens that mark an interface part (a backdrop, a frame, a progress bar) kept in an art folder. They leave a
/// file out of every family but `gui_texture`. TFR keeps `circular_progress_bar.dds` among its operation icons and a
/// `se_background.tga` among its super events.
pub const UI_TOKENS: [&str; 16] = [
    "bg",
    "background",
    "button",
    "buttons",
    "border",
    "frame",
    "frames",
    "mask",
    "overlay",
    "shadow",
    "highlight",
    "glow",
    "progress",
    "bar",
    "tile",
    "tiled",
];

/// Which files of a shared folder belong to the family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Every size.
    Any,
    /// Files with no side longer than this (small icons in a folder shared with pictures).
    MaxSide(u32),
    /// Files with no side shorter than this.
    MinSide(u32),
    /// Files at least this wide.
    MinWidth(u32),
    /// The largest size cluster of an event folder: report pictures.
    EventReport,
    /// The second size cluster of an event folder: news pictures.
    EventNews,
}

/// Where a family's art sits in a mod and how it is told apart.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// The family.
    pub family: Family,
    /// Folder inside the mod.
    pub dir: &'static str,
    /// Whether sub-folders are read.
    pub recursive: bool,
    /// Size rule inside a shared folder.
    pub class: Class,
    /// Path tokens that leave a file out (another family in the same folder).
    pub exclude: &'static [&'static str],
    /// Path tokens of which a file needs one (empty: none needed).
    pub require: &'static [&'static str],
    /// Rank by typicality alone (portraits).
    pub typical: bool,
}

const fn spec(family: Family, dir: &'static str, recursive: bool, class: Class, exclude: &'static [&'static str]) -> Spec {
    Spec { family, dir, recursive, class, exclude, require: &[], typical: false }
}

/// The families `pack derive` covers, with the folders a mod keeps them in. The folders are vanilla's; the size rules
/// split the folders two families share (decisions and their category pictures, report and news pictures, operation
/// icons and pictures).
pub fn specs() -> Vec<Spec> {
    use Family::*;
    vec![
        spec(Focus, "gfx/interface/goals", true, Class::Any, &[]),
        // the ideas folder also holds advisor portraits, laws shown as portraits and industrial concerns
        spec(
            IdeaSpirit,
            "gfx/interface/ideas",
            true,
            Class::Any,
            &["advisor", "advisors", "minister", "ministers", "portrait", "portraits", "companies", "concern", "concerns"],
        ),
        spec(Decision, "gfx/interface/decisions", true, Class::MaxSide(64), &[]),
        spec(DecisionCategoryPicture, "gfx/interface/decisions", true, Class::MinSide(80), &[]),
        spec(ReportEvent, "gfx/event_pictures", true, Class::EventReport, &[]),
        spec(NewsEvent, "gfx/event_pictures", true, Class::EventNews, &[]),
        Spec { typical: true, ..spec(LeaderPortrait, "gfx/leaders", true, Class::Any, &["operative", "operatives", "scientist", "scientists"]) },
        spec(Technology, "gfx/interface/technologies", true, Class::MinWidth(120), &[]),
        spec(CounterLarge, "gfx/interface/counters/divisions_large", true, Class::Any, &[]),
        spec(CommanderTrait, "gfx/interface/traits", false, Class::Any, &["operative"]),
        // vanilla's operation art is the phase art (phase_*); the folder also holds map markers and interface parts
        Spec { require: &["phase"], ..spec(OperationPicture, "gfx/interface/operations", true, Class::MinSide(100), &[]) },
        Spec { require: &["phase"], ..spec(OperationIcon, "gfx/interface/operations", true, Class::MaxSide(99), &[]) },
        spec(Achievement, "gfx/achievements", false, Class::Any, &["grey", "eligible"]),
        spec(SuperEvent, "gfx/super_events", true, Class::MinSide(300), &[]),
        spec(GuiTexture, "gfx/interface", false, Class::MinSide(150), &[]),
    ]
}

/// The spec of a family, if `pack derive` covers it.
pub fn spec_of(family: Family) -> Option<Spec> {
    specs().into_iter().find(|s| s.family == family)
}

/// Lower-case alphanumeric tokens of a path.
pub fn path_tokens(rel: &str) -> Vec<String> {
    rel.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// Whether a path's name marks a placeholder.
pub fn is_placeholder_name(rel: &str) -> bool {
    path_tokens(rel).iter().any(|t| PLACEHOLDER_TOKENS.contains(&t.as_str()))
}

/// One image file found in a folder: its path relative to the scanned root, its absolute path and its header size.
#[derive(Debug, Clone, PartialEq)]
pub struct FileInfo {
    /// Path relative to the root it was found under, with `/`.
    pub rel: String,
    /// Absolute path.
    pub abs: PathBuf,
    /// Size from the file header.
    pub size: [u32; 2],
}

fn walk(dir: &Path, recursive: bool, depth: u32, budget: &mut usize, out: &mut Vec<(PathBuf, Stamp)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
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
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if !(name.ends_with(".png") || name.ends_with(".dds") || name.ends_with(".tga")) {
                continue;
            }
            // the directory listing carries the length and time, so no file is opened here
            let meta = e.metadata().ok();
            let time =
                meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            out.push((e.path(), [meta.map_or(0, |m| m.len()), time]));
        }
    }
}

/// A file's length and modification time (seconds), which key the scan cache.
pub type Stamp = [u64; 2];

/// Sizes already read, by path relative to the scanned root: the file's [`Stamp`] when it was read, and its size.
pub type KnownSizes = BTreeMap<String, (Stamp, [u32; 2])>;

/// The PNG, DDS and TGA files in `root/dir` with a readable header size, sorted by path. A file in `known` with the
/// same length and time is not opened again. Returns the files and the sizes now known for them.
pub fn scan(root: &Path, dir: &str, recursive: bool, known: &KnownSizes) -> (Vec<FileInfo>, KnownSizes) {
    let mut found = Vec::new();
    let mut budget = WALK_BUDGET;
    walk(&root.join(dir), recursive, 0, &mut budget, &mut found);
    let mut now = KnownSizes::new();
    let mut out: Vec<FileInfo> = found
        .into_iter()
        .filter_map(|(abs, stamp)| {
            let rel = abs.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
            let size = match known.get(&rel) {
                Some((s, size)) if *s == stamp => *size,
                _ => crate::stylesrc::header_size(&abs)?,
            };
            now.insert(rel.clone(), (stamp, size));
            Some(FileInfo { rel, abs, size })
        })
        .collect();
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    (out, now)
}

fn in_cluster(size: [u32; 2], centre: [u32; 2]) -> bool {
    let (w, h) = (f64::from(size[0].max(1)), f64::from(size[1].max(1)));
    let (cw, ch) = (f64::from(centre[0].max(1)), f64::from(centre[1].max(1)));
    (w / cw - 1.0).abs() <= SIZE_TOLERANCE && (h / ch - 1.0).abs() <= SIZE_TOLERANCE && ((w / h) / (cw / ch) - 1.0).abs() <= ASPECT_TOLERANCE
}

/// The dominant size cluster of a set of sizes: the size whose neighbourhood ([`SIZE_TOLERANCE`],
/// [`ASPECT_TOLERANCE`]) holds the most files (ties: the more common exact size, then the smaller size). Returns the
/// cluster's most common exact size (the canvas) and which sizes belong to it. `None` for no sizes.
pub fn dominant_cluster(sizes: &[[u32; 2]]) -> Option<([u32; 2], Vec<bool>)> {
    let mut counts: BTreeMap<[u32; 2], usize> = BTreeMap::new();
    for s in sizes {
        *counts.entry(*s).or_default() += 1;
    }
    let (centre, _) = counts
        .iter()
        .map(|(s, n)| (*s, (counts.iter().filter(|(t, _)| in_cluster(**t, *s)).map(|(_, m)| m).sum::<usize>(), *n)))
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))?;
    let member: Vec<bool> = sizes.iter().map(|s| in_cluster(*s, centre)).collect();
    let canvas = counts.iter().filter(|(s, _)| in_cluster(**s, centre)).max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(s, _)| *s)?;
    Some((canvas, member))
}

/// The files of a family in a mod: the folder's files that pass the name filters and the size rule, then the
/// dominant size cluster. Returns the files, the canvas and how many files each filter took out.
pub fn family_files(spec: &Spec, all: &[FileInfo]) -> (Vec<FileInfo>, Option<[u32; 2]>, BTreeMap<&'static str, usize>) {
    let mut dropped: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut keep: Vec<FileInfo> = Vec::new();
    for f in all {
        let tokens = path_tokens(&f.rel);
        if is_placeholder_name(&f.rel) {
            *dropped.entry("placeholder_name").or_default() += 1;
            continue;
        }
        let ui = spec.family != Family::GuiTexture && tokens.iter().any(|t| UI_TOKENS.contains(&t.as_str()));
        if ui
            || tokens.iter().any(|t| spec.exclude.contains(&t.as_str()))
            || (!spec.require.is_empty() && !tokens.iter().any(|t| spec.require.contains(&t.as_str())))
        {
            *dropped.entry("other_family_name").or_default() += 1;
            continue;
        }
        let [w, h] = f.size;
        let ok = match spec.class {
            Class::Any | Class::EventReport | Class::EventNews => true,
            Class::MaxSide(m) => w.max(h) <= m,
            Class::MinSide(m) => w.min(h) >= m,
            Class::MinWidth(m) => w >= m,
        };
        if !ok {
            *dropped.entry("other_size_class").or_default() += 1;
            continue;
        }
        keep.push(f.clone());
    }
    let sizes: Vec<[u32; 2]> = keep.iter().map(|f| f.size).collect();
    let Some((first_canvas, first)) = dominant_cluster(&sizes) else { return (Vec::new(), None, dropped) };
    let (canvas, member) = match spec.class {
        Class::EventReport | Class::EventNews => {
            // report pictures outnumber news pictures in vanilla and in every mod measured; a path that says "news"
            // more often than the other cluster's paths moves the news label to its cluster
            let rest: Vec<[u32; 2]> = sizes.iter().zip(&first).filter(|(_, m)| !**m).map(|(s, _)| *s).collect();
            let second = dominant_cluster(&rest).filter(|(_, m)| m.iter().filter(|x| **x).count() * 20 >= keep.len().max(1));
            let news_share = |m: &[bool]| {
                let n = m.iter().filter(|x| **x).count().max(1);
                keep.iter().zip(m).filter(|(f, x)| **x && path_tokens(&f.rel).iter().any(|t| t.contains("news"))).count() as f64 / n as f64
            };
            let first_is_news = second.as_ref().is_some_and(|(c, _)| {
                let m: Vec<bool> = sizes.iter().map(|s| in_cluster(*s, *c) && !in_cluster(*s, first_canvas)).collect();
                news_share(&first) > news_share(&m)
            });
            let news =
                |c: [u32; 2]| -> ([u32; 2], Vec<bool>) { (c, sizes.iter().map(|s| in_cluster(*s, c) && !in_cluster(*s, first_canvas)).collect()) };
            match (spec.class, second) {
                (Class::EventReport, Some((c, _))) if first_is_news => news(c),
                (Class::EventNews, Some(_)) if first_is_news => (first_canvas, first),
                (Class::EventNews, Some((c, _))) => news(c),
                (Class::EventNews, None) => {
                    *dropped.entry("other_size_class").or_default() += keep.len();
                    return (Vec::new(), None, dropped);
                }
                _ => (first_canvas, first),
            }
        }
        _ => (first_canvas, first),
    };
    let mut out = Vec::new();
    for (f, m) in keep.into_iter().zip(member) {
        if m {
            out.push(f);
        } else {
            *dropped.entry("other_size_cluster").or_default() += 1;
        }
    }
    (out, Some(canvas), dropped)
}

/// Evenly sample at most `n` items, keeping their order.
pub fn sample<T: Clone>(items: &[T], n: usize) -> Vec<T> {
    if items.len() <= n {
        return items.to_vec();
    }
    (0..n).map(|i| items[i * items.len() / n].clone()).collect()
}

/// Simple measured figures of an image. Saturation and value are over opaque pixels (alpha over 128).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// Mean saturation.
    pub saturation: f64,
    /// Mean value (brightness).
    pub value: f64,
    /// Share of opaque pixels with saturation under 0.12.
    pub grey_share: f64,
    /// Share of opaque pixels.
    pub opaque: f64,
    /// Mean absolute horizontal step of grey at 48x48.
    pub edges: f64,
    /// Share of opaque pixels covered by the 6 most common colours (4 bits a channel): near 1 for flat art.
    pub flat_share: f64,
    /// Saturation-weighted hue histogram in 12 bins of 30 degrees, summing to 1 (0 for grey art).
    pub hue: [f64; 12],
}

/// One measured image.
#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    /// Path relative to its root.
    pub rel: String,
    /// Size.
    pub size: [u32; 2],
    /// SHA-256 of the file.
    pub sha256: String,
    /// 64-bit dHash (9x8 grey, each pixel brighter than its left neighbour).
    pub dhash: u64,
    /// Style vector ([`STYLE_DIMS`]).
    pub style: Vec<f32>,
    /// Figures.
    pub stats: Stats,
}

/// Box-average an image onto a `gw` x `gh` grid: straight (not premultiplied) RGB and the alpha, 0 to 1.
fn grid(img: &Rgba, gw: usize, gh: usize) -> Vec<[f64; 4]> {
    let (iw, ih) = (img.width as usize, img.height as usize);
    let mut acc = vec![[0f64; 4]; gw * gh];
    let mut n = vec![0f64; gw * gh];
    for y in 0..ih {
        let ty = y * gh / ih;
        for x in 0..iw {
            let t = ty * gw + x * gw / iw;
            let p = &img.data[(y * iw + x) * 4..(y * iw + x) * 4 + 4];
            for c in 0..4 {
                acc[t][c] += f64::from(p[c]) / 255.0;
            }
            n[t] += 1.0;
        }
    }
    acc.iter().zip(&n).map(|(a, k)| if *k > 0.0 { [a[0] / k, a[1] / k, a[2] / k, a[3] / k] } else { [0.0; 4] }).collect()
}

/// Hue (0 to 1), saturation and value of an RGB colour in 0 to 1.
pub fn hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let s = if max > 0.0 { d / max } else { 0.0 };
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, max)
}

fn mean_std(v: &[f64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let m = v.iter().sum::<f64>() / v.len() as f64;
    (m, (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt())
}

/// The 64-bit dHash: the image box-averaged to 9x8 grey (alpha ignored, as the reference scripts did), one bit per
/// pixel brighter than its left neighbour.
pub fn dhash(img: &Rgba) -> u64 {
    let g = grid(img, 9, 8);
    let grey = |p: &[f64; 4]| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
    let mut bits = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            bits = (bits << 1) | u64::from(grey(&g[y * 9 + x + 1]) > grey(&g[y * 9 + x]));
        }
    }
    bits
}

/// Bits that differ between two hashes.
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Measure an image: dHash, style vector and figures. `None` for an empty image.
pub fn measure(img: &Rgba) -> Option<(u64, Vec<f32>, Stats)> {
    if img.width == 0 || img.height == 0 {
        return None;
    }
    let mut style: Vec<f32> = Vec::with_capacity(STYLE_DIMS);
    // alpha shape at 16x16, half weight
    style.extend(grid(img, 16, 16).iter().map(|p| (p[3] * 0.5) as f32));
    // colour layout at 8x8
    for p in grid(img, 8, 8) {
        style.extend([p[0] as f32, p[1] as f32, p[2] as f32]);
    }
    // palette over opaque pixels at 32x32
    let px: Vec<[f64; 4]> = grid(img, 32, 32);
    let opaque: Vec<&[f64; 4]> = px.iter().filter(|p| p[3] > 0.5).collect();
    let pool: Vec<&[f64; 4]> = if opaque.is_empty() { px.iter().collect() } else { opaque.clone() };
    let mut hue = [0f64; 12];
    let (mut sats, mut vals) = (Vec::new(), Vec::new());
    for p in &pool {
        let (h, s, v) = hsv(p[0], p[1], p[2]);
        hue[((h * 12.0) as usize).min(11)] += s;
        sats.push(s);
        vals.push(v);
    }
    let total: f64 = hue.iter().sum();
    if total > 1e-9 {
        for x in &mut hue {
            *x /= total;
        }
    }
    style.extend(hue.iter().map(|x| (x * 2.0) as f32));
    let g48 = grid(img, 48, 48);
    let grey = |p: &[f64; 4]| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
    let mut edges = 0.0;
    for y in 0..48 {
        for x in 0..47 {
            edges += (grey(&g48[y * 48 + x + 1]) - grey(&g48[y * 48 + x])).abs();
        }
    }
    edges /= 48.0 * 47.0;
    let (s_mean, s_std) = mean_std(&sats);
    let (v_mean, v_std) = mean_std(&vals);
    let opaque_share = opaque.len() as f64 / px.len() as f64;
    let grey_share = sats.iter().filter(|s| **s < 0.12).count() as f64 / sats.len().max(1) as f64;
    let aspect = f64::from(img.width) / f64::from(img.height.max(1));
    for x in [s_mean, s_std, v_mean, v_std, opaque_share, aspect, edges, grey_share] {
        style.push((x * 3.0) as f32);
    }
    // flatness on the image itself (sampled to at most about 40,000 pixels)
    let step = ((img.width as usize * img.height as usize) / 40_000).max(1);
    let mut colours: BTreeMap<u16, u32> = BTreeMap::new();
    let mut seen = 0u32;
    for p in img.data.chunks_exact(4).step_by(step) {
        if p[3] > 128 {
            *colours.entry((u16::from(p[0] >> 4) << 8) | (u16::from(p[1] >> 4) << 4) | u16::from(p[2] >> 4)).or_default() += 1;
            seen += 1;
        }
    }
    let mut counts: Vec<u32> = colours.into_values().collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    let flat_share = if seen > 0 { f64::from(counts.iter().take(6).sum::<u32>()) / f64::from(seen) } else { 0.0 };
    let stats = Stats { saturation: s_mean, value: v_mean, grey_share, opaque: opaque_share, edges, flat_share, hue };
    debug_assert_eq!(style.len(), STYLE_DIMS);
    Some((dhash(img), style, stats))
}

/// Whether an image is a blank or flat placeholder: almost nothing opaque, or one flat tone.
///
/// A flat glyph on transparency is a house style (TNO draws its counters in one teal), so only a filled canvas of one
/// tone counts as flat.
pub fn is_flat(style: &[f32], stats: &Stats) -> bool {
    // saturation and value spreads sit at offsets 1 and 3 of the figures, scaled by 3
    let fig = &style[STYLE_DIMS - 8..];
    stats.opaque < 0.02 || (stats.opaque > 0.95 && fig[1] / 3.0 < 0.01 && fig[3] / 3.0 < 0.01)
}

/// Whether a family's art is mostly vanilla's ([`COPY_SHARE_MAX`], [`PALETTE_MIN`]), with the reason.
pub fn vanilla_like(copies: usize, measured: usize, palette_distinctness: Option<f64>) -> Option<String> {
    let share = copies as f64 / measured.max(1) as f64;
    let p = palette_distinctness?;
    (share >= COPY_SHARE_MAX && p < PALETTE_MIN).then(|| {
        format!(
            "mostly vanilla's art: {copies} of {measured} measured files are near-copies of vanilla, and the rest keep vanilla's palette (palette distance {p:.2} times vanilla's own spread, a house style measures at least {PALETTE_MIN:.2})"
        )
    })
}

/// Encode measured files for the cache (a small binary format, version 1).
pub fn encode_cache(rows: &[Measured]) -> Vec<u8> {
    let mut out = b"HDV1".to_vec();
    out.extend((rows.len() as u32).to_le_bytes());
    for m in rows {
        let rel = m.rel.as_bytes();
        out.extend((rel.len() as u32).to_le_bytes());
        out.extend(rel);
        out.extend(m.size[0].to_le_bytes());
        out.extend(m.size[1].to_le_bytes());
        out.extend(m.sha256.as_bytes().get(..64).unwrap_or(&[b'0'; 64]));
        out.extend(m.dhash.to_le_bytes());
        out.push(u8::from(!m.style.is_empty()));
        for x in &m.style {
            out.extend(x.to_le_bytes());
        }
        let st = &m.stats;
        for x in [st.saturation, st.value, st.grey_share, st.opaque, st.edges, st.flat_share].iter().chain(st.hue.iter()) {
            out.extend(x.to_le_bytes());
        }
    }
    out
}

/// Decode [`encode_cache`]. `None` for anything malformed.
pub fn decode_cache(bytes: &[u8]) -> Option<Vec<Measured>> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = bytes.get(at..at + n)?;
        at += n;
        Some(s)
    };
    if take(4)? != b"HDV1" {
        return None;
    }
    let u32_of = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap_or([0; 4]));
    let n = u32_of(take(4)?) as usize;
    let mut out = Vec::with_capacity(n.min(100_000));
    for _ in 0..n {
        let len = u32_of(take(4)?) as usize;
        let rel = String::from_utf8(take(len)?.to_vec()).ok()?;
        let size = [u32_of(take(4)?), u32_of(take(4)?)];
        let sha256 = String::from_utf8(take(64)?.to_vec()).ok()?;
        let dhash = u64::from_le_bytes(take(8)?.try_into().ok()?);
        let styled = take(1)?[0] == 1;
        let mut style = Vec::new();
        if styled {
            for _ in 0..STYLE_DIMS {
                style.push(f32::from_le_bytes(take(4)?.try_into().ok()?));
            }
        }
        let mut f = [0f64; 18];
        for x in &mut f {
            *x = f64::from_le_bytes(take(8)?.try_into().ok()?);
        }
        let mut hue = [0f64; 12];
        hue.copy_from_slice(&f[6..]);
        let stats = Stats { saturation: f[0], value: f[1], grey_share: f[2], opaque: f[3], edges: f[4], flat_share: f[5], hue };
        out.push(Measured { rel, size, sha256, dhash, style, stats });
    }
    (at == bytes.len()).then_some(out)
}

/// Decode and measure files on `threads` threads. Unreadable files are left out. Order follows `files`.
pub fn measure_files(files: &[FileInfo], threads: usize) -> Vec<Measured> {
    let threads = threads.clamp(1, 32);
    let chunk = files.len().div_ceil(threads).max(1);
    let mut out: Vec<Option<Measured>> = Vec::with_capacity(files.len());
    std::thread::scope(|s| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|f| {
                            let bytes = crate::fsio::read_limited(&f.abs, crate::refs::MAX_REFERENCE_BYTES).ok()?;
                            let name = f.abs.file_name().map(|n| n.to_string_lossy().to_string());
                            let (img, _) = decode(&bytes, name.as_deref(), DecodeLimits::default()).ok()?;
                            let (dhash, style, stats) = measure(&img)?;
                            Some(Measured {
                                rel: f.rel.clone(),
                                size: [img.width, img.height],
                                sha256: crate::hashing::sha256_hex(&bytes),
                                dhash,
                                style,
                                stats,
                            })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for h in handles {
            out.extend(h.join().unwrap_or_default());
        }
    });
    out.into_iter().flatten().collect()
}

fn sq_dist(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// Mean distance from each row of `a` to its `k` nearest rows of `b` (leaving out the row itself when `same`).
pub fn knn_mean(a: &[Vec<f32>], b: &[Vec<f32>], k: usize, same: bool, threads: usize) -> Vec<f64> {
    let k = k.max(1).min(b.len().saturating_sub(usize::from(same))).max(1);
    let chunk = a.len().div_ceil(threads.clamp(1, 32)).max(1);
    let mut out = Vec::with_capacity(a.len());
    std::thread::scope(|s| {
        let handles: Vec<_> = a
            .chunks(chunk)
            .enumerate()
            .map(|(ci, part)| {
                s.spawn(move || {
                    part.iter()
                        .enumerate()
                        .map(|(i, x)| {
                            let me = ci * chunk + i;
                            let mut d: Vec<f32> = b.iter().enumerate().filter(|(j, _)| !(same && *j == me)).map(|(_, y)| sq_dist(x, y)).collect();
                            if d.is_empty() {
                                return 0.0;
                            }
                            let k = k.min(d.len());
                            d.select_nth_unstable_by(k - 1, |p, q| p.total_cmp(q));
                            d[..k].iter().map(|v| f64::from(v.max(0.0)).sqrt()).sum::<f64>() / k as f64
                        })
                        .collect::<Vec<f64>>()
                })
            })
            .collect();
        for h in handles {
            out.extend(h.join().unwrap_or_default());
        }
    });
    out
}

/// Median of a list (0 for none).
pub fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// The ranking of a family's mod files.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranking {
    /// Z-normalised style vectors of the mod files (same order as the input).
    pub z: Vec<Vec<f32>>,
    /// Score of each file (higher: more the mod's own style).
    pub score: Vec<f64>,
    /// Bits to the nearest vanilla file (64 without vanilla).
    pub hamming: Vec<u32>,
    /// How far the mod's files are from vanilla, relative to how far vanilla's files are from each other (median
    /// over files that are not near-copies). `None` without vanilla files.
    pub distinctness: Option<f64>,
    /// The same over the palette part of the style vector alone (hue histogram and figures).
    pub palette_distinctness: Option<f64>,
}

/// Rank mod files against vanilla files ([`Ranking`]).
pub fn rank(modf: &[Measured], van: &[Measured], typical: bool, threads: usize) -> Ranking {
    let hamming: Vec<u32> = modf.iter().map(|m| van.iter().map(|v| hamming(m.dhash, v.dhash)).min().unwrap_or(64)).collect();
    // every vanilla file counts for the near-copy check; the style ranking uses the sampled ones that carry a vector
    let van: Vec<&Measured> = van.iter().filter(|m| m.style.len() == STYLE_DIMS).collect();
    let all: Vec<&Vec<f32>> = van.iter().copied().chain(modf).map(|m| &m.style).collect();
    let n = all.len().max(1) as f64;
    let mut mu = vec![0f64; STYLE_DIMS];
    let mut sd = vec![0f64; STYLE_DIMS];
    for v in &all {
        for (d, x) in v.iter().enumerate() {
            mu[d] += f64::from(*x) / n;
        }
    }
    for v in &all {
        for (d, x) in v.iter().enumerate() {
            sd[d] += (f64::from(*x) - mu[d]).powi(2) / n;
        }
    }
    let norm = |v: &Vec<f32>| -> Vec<f32> { v.iter().enumerate().map(|(d, x)| ((f64::from(*x) - mu[d]) / (sd[d].sqrt() + 1e-6)) as f32).collect() };
    let z: Vec<Vec<f32>> = modf.iter().map(|m| norm(&m.style)).collect();
    let zv: Vec<Vec<f32>> = van.iter().map(|m| norm(&m.style)).collect();
    let d_mod = knn_mean(&z, &z, KNN, true, threads);
    let m_mod = median(&d_mod).max(1e-9);
    let (score, distinctness) = if zv.is_empty() {
        (d_mod.iter().map(|d| -d / m_mod).collect::<Vec<f64>>(), None)
    } else {
        let d_van = knn_mean(&z, &zv, KNN, false, threads);
        let d_vv = knn_mean(&zv, &zv, KNN, true, threads);
        let m_van = median(&d_van).max(1e-9);
        let ratio = |d_van: &[f64], d_vv: &[f64]| {
            let own: Vec<f64> = d_van.iter().zip(&hamming).filter(|(_, h)| **h > NEAR_COPY_BITS).map(|(d, _)| *d).collect();
            if own.is_empty() {
                0.0
            } else {
                median(&own) / median(d_vv).max(1e-9)
            }
        };
        let distinct = ratio(&d_van, &d_vv);
        let palette = |v: &[Vec<f32>]| -> Vec<Vec<f32>> { v.iter().map(|r| r[PALETTE_START..].to_vec()).collect() };
        let (pz, pv) = (palette(&z), palette(&zv));
        let palette_distinct = ratio(&knn_mean(&pz, &pv, KNN, false, threads), &knn_mean(&pv, &pv, KNN, true, threads));
        let score =
            if typical { d_mod.iter().map(|d| -d / m_mod).collect() } else { d_van.iter().zip(&d_mod).map(|(a, b)| a / m_van - b / m_mod).collect() };
        (score, Some((distinct, palette_distinct)))
    };
    Ranking { z, score, hamming, distinctness: distinctness.map(|d| d.0), palette_distinctness: distinctness.map(|d| d.1) }
}

/// The sub-folder that tags a file (often a country tag): its parent folder name below the family folder.
pub fn folder_tag(rel: &str, dir: &str) -> String {
    let rest = rel.strip_prefix(dir).unwrap_or(rel).trim_start_matches('/');
    match rest.rsplit_once('/') {
        Some((parent, _)) => parent.rsplit('/').next().unwrap_or(parent).to_ascii_lowercase(),
        None => "-".into(),
    }
}

/// k-means with k-means++ seeding from a fixed seed. Returns each row's cluster.
pub fn kmeans(rows: &[Vec<f32>], k: usize, seed: u64) -> Vec<usize> {
    if rows.is_empty() || k == 0 {
        return vec![0; rows.len()];
    }
    let k = k.min(rows.len());
    let mut rng = Rng::new(seed);
    let mut centres: Vec<Vec<f32>> = vec![rows[(rng.next_u64() % rows.len() as u64) as usize].clone()];
    while centres.len() < k {
        let d: Vec<f64> = rows.iter().map(|r| centres.iter().map(|c| f64::from(sq_dist(r, c))).fold(f64::MAX, f64::min)).collect();
        let total: f64 = d.iter().sum();
        if total <= 0.0 {
            break;
        }
        let mut t = (rng.next_u64() as f64 / u64::MAX as f64) * total;
        let mut pick = rows.len() - 1;
        for (i, x) in d.iter().enumerate() {
            if t < *x {
                pick = i;
                break;
            }
            t -= x;
        }
        centres.push(rows[pick].clone());
    }
    let mut assign = vec![0usize; rows.len()];
    for _ in 0..30 {
        let mut changed = false;
        for (i, r) in rows.iter().enumerate() {
            let best = (0..centres.len()).min_by(|a, b| sq_dist(r, &centres[*a]).total_cmp(&sq_dist(r, &centres[*b]))).unwrap_or(0);
            if assign[i] != best {
                assign[i] = best;
                changed = true;
            }
        }
        for (c, centre) in centres.iter_mut().enumerate() {
            let members: Vec<&Vec<f32>> = rows.iter().zip(&assign).filter(|(_, a)| **a == c).map(|(r, _)| r).collect();
            if members.is_empty() {
                continue;
            }
            for (d, x) in centre.iter_mut().enumerate() {
                *x = members.iter().map(|m| m[d]).sum::<f32>() / members.len() as f32;
            }
        }
        if !changed {
            break;
        }
    }
    assign
}

/// One picked or alternate file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pick {
    /// Index into the family's measured files.
    pub index: usize,
    /// Path relative to the mod.
    pub rel: String,
    /// Score.
    pub score: f64,
    /// Bits to the nearest vanilla file.
    pub vanilla_hamming: u32,
    /// Style cluster.
    pub cluster: usize,
}

/// Pick `n` diverse references and up to [`MAX_ALTERNATES`] alternates from the eligible files: k-means over the
/// top [`POOL`] of the ranking, then one pick per cluster in the order of each cluster's best score, the best
/// eligible member of each, repeated until `n` are picked. A pick is never within [`NEAR_COPY_BITS`] of another and
/// at most [`PER_FOLDER`] come from one sub-folder. The alternates follow the same rules.
pub fn pick_diverse(measured: &[Measured], ranking: &Ranking, eligible: &[usize], spec_dir: &str, n: usize, seed: u64) -> (Vec<Pick>, Vec<Pick>) {
    let mut order: Vec<usize> = eligible.to_vec();
    order.sort_by(|a, b| ranking.score[*b].total_cmp(&ranking.score[*a]).then(measured[*a].rel.cmp(&measured[*b].rel)));
    order.truncate(POOL);
    if order.is_empty() || n == 0 {
        return (Vec::new(), Vec::new());
    }
    let rows: Vec<Vec<f32>> = order.iter().map(|i| ranking.z[*i].clone()).collect();
    let assign = kmeans(&rows, n, seed);
    let k = assign.iter().max().map_or(0, |m| m + 1);
    // members of each cluster, best score first (the pool is already in score order)
    let mut clusters: Vec<Vec<usize>> = vec![Vec::new(); k];
    for (pos, c) in assign.iter().enumerate() {
        clusters[*c].push(pos);
    }
    let mut cluster_order: Vec<usize> = (0..k).filter(|c| !clusters[*c].is_empty()).collect();
    cluster_order.sort_by_key(|c| clusters[*c][0]);
    let mut taken = vec![false; order.len()];
    let mut per_folder: BTreeMap<String, usize> = BTreeMap::new();
    let mut chosen_hashes: Vec<u64> = Vec::new();
    let mut take = |want: usize, limit_folder: bool, out: &mut Vec<Pick>, taken: &mut Vec<bool>| {
        let mut cursor = vec![0usize; k];
        loop {
            let before = out.len();
            for c in &cluster_order {
                if out.len() >= want {
                    return;
                }
                while cursor[*c] < clusters[*c].len() {
                    let pos = clusters[*c][cursor[*c]];
                    cursor[*c] += 1;
                    let i = order[pos];
                    let m = &measured[i];
                    let tag = folder_tag(&m.rel, spec_dir);
                    if taken[pos]
                        || (limit_folder && per_folder.get(&tag).copied().unwrap_or(0) >= PER_FOLDER)
                        || chosen_hashes.iter().any(|h| hamming(*h, m.dhash) <= NEAR_COPY_BITS)
                    {
                        continue;
                    }
                    taken[pos] = true;
                    *per_folder.entry(tag).or_default() += 1;
                    chosen_hashes.push(m.dhash);
                    out.push(Pick { index: i, rel: m.rel.clone(), score: ranking.score[i], vanilla_hamming: ranking.hamming[i], cluster: *c });
                    break;
                }
            }
            if out.len() == before {
                return;
            }
        }
    };
    let mut picks = Vec::new();
    take(n, true, &mut picks, &mut taken);
    if picks.len() < n {
        // a mod with one or two sub-folders: lift the folder cap rather than pick fewer
        take(n, false, &mut picks, &mut taken);
    }
    let mut alternates = Vec::new();
    take(MAX_ALTERNATES, false, &mut alternates, &mut taken);
    (picks, alternates)
}

/// Medians of the figures over a set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Figures {
    /// Files measured.
    pub files: usize,
    /// Median saturation over opaque pixels.
    pub saturation: f64,
    /// Median value (brightness).
    pub value: f64,
    /// Median grey share.
    pub grey_share: f64,
    /// Median opaque share.
    pub opaque: f64,
    /// Median edge density.
    pub edges: f64,
    /// Median flat share.
    pub flat_share: f64,
    /// Strongest hue bin of the summed hue histogram (0 to 11), and its share.
    pub hue: Option<(usize, f64)>,
    /// The second strongest hue bin that is not next to the first, and its share.
    pub hue2: Option<(usize, f64)>,
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// The figures of a set of measured files.
pub fn figures(rows: &[&Stats]) -> Figures {
    let med = |f: fn(&Stats) -> f64| round3(median(&rows.iter().map(|s| f(s)).collect::<Vec<f64>>()));
    let mut hue = [0f64; 12];
    for s in rows {
        for (h, x) in hue.iter_mut().zip(s.hue) {
            *h += x * s.saturation;
        }
    }
    let total: f64 = hue.iter().sum();
    let top = (0..12).max_by(|a, b| hue[*a].total_cmp(&hue[*b])).filter(|_| total > 1e-9).map(|i| (i, round3(hue[i] / total)));
    let apart = |i: usize| top.is_some_and(|(t, _)| (i + 12 - t) % 12 > 1 && (t + 12 - i) % 12 > 1);
    let hue2 = (0..12).filter(|i| apart(*i)).max_by(|a, b| hue[*a].total_cmp(&hue[*b])).filter(|_| total > 1e-9).map(|i| (i, round3(hue[i] / total)));
    Figures {
        files: rows.len(),
        saturation: med(|s| s.saturation),
        value: med(|s| s.value),
        grey_share: med(|s| s.grey_share),
        opaque: med(|s| s.opaque),
        edges: med(|s| s.edges),
        flat_share: med(|s| s.flat_share),
        hue: top,
        hue2,
    }
}

/// Name of a hue bin.
pub fn hue_name(bin: usize) -> &'static str {
    ["red", "orange", "yellow", "yellow-green", "green", "green-teal", "teal", "azure", "blue", "violet", "magenta", "crimson"][bin.min(11)]
}

/// Processing switches suggested from the figures, with the reason for each. Only switches that apply to the family
/// (`pack::switch_applies`) are suggested.
pub fn suggest(
    family: Family,
    profile: &crate::profile::Profile,
    canvas: Option<[u32; 2]>,
    m: &Figures,
    v: Option<&Figures>,
) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let applies = |s: &str| crate::pack::switch_applies(s, profile);
    let own_canvas = canvas.is_some_and(|c| c != profile.canvas);
    if applies("upright_print") {
        let differs = v.is_none_or(|v| (m.opaque - v.opaque).abs() > 0.1);
        if own_canvas || differs {
            out.push((
                "upright_print",
                format!(
                    "the mod's pictures are {} with {} opaque, vanilla's card is tilted and masked{}",
                    canvas.map_or("their own size".into(), |c| format!("{}x{}", c[0], c[1])),
                    pct(m.opaque),
                    v.map_or(String::new(), |v| format!(" ({} opaque)", pct(v.opaque)))
                ),
            ));
        }
    }
    if applies("keep_colours") {
        if let Some(v) = v {
            if (m.saturation - v.saturation).abs() >= 0.2 {
                out.push(("keep_colours", format!("the mod's unit art has saturation {:.2}, vanilla's material {:.2}", m.saturation, v.saturation)));
            }
        }
    }
    if applies("fill_canvas") && m.opaque >= 0.9 && v.is_none_or(|v| v.opaque < 0.9) {
        out.push((
            "fill_canvas",
            format!(
                "the mod's art fills the canvas ({} opaque){}",
                pct(m.opaque),
                v.map_or(String::new(), |v| format!(", vanilla's is {} opaque", pct(v.opaque)))
            ),
        ));
    }
    if applies("in_colour") && m.saturation >= 0.15 && m.grey_share < 0.5 {
        out.push((
            "in_colour",
            format!(
                "the mod's art is in colour (saturation {:.2}, {} grey){}",
                m.saturation,
                pct(m.grey_share),
                v.map_or(String::new(), |v| format!(", the vanilla chain greys it (vanilla saturation {:.2})", v.saturation))
            ),
        ));
    }
    if applies("crisp") && family != Family::GuiTexture && m.flat_share >= 0.8 && v.is_none_or(|v| v.flat_share < m.flat_share - 0.15) {
        out.push((
            "crisp",
            format!(
                "flat art: 6 colours cover {} of the mod's opaque pixels{}",
                pct(m.flat_share),
                v.map_or(String::new(), |v| format!(", {} of vanilla's", pct(v.flat_share)))
            ),
        ));
    }
    out
}

fn pct(x: f64) -> String {
    format!("{:.0} percent", x * 100.0)
}

/// A draft look line from the figures: the medium (grey, muted, saturated), the main hue, the brightness, whether the
/// art fills its canvas or is a cut-out, flat or detailed, and the canvas where it differs from vanilla's. A draft to
/// read against the contact sheet: a measurement cannot name the subject matter.
pub fn draft_look(m: &Figures, v: Option<&Figures>, canvas: Option<[u32; 2]>, vanilla_canvas: [u32; 2]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let hue = m.hue.filter(|(_, share)| *share >= 0.25).map(|(b, _)| match m.hue2.filter(|(_, share)| *share >= 0.15) {
        Some((b2, _)) => format!("{} and {}", hue_name(b), hue_name(b2)),
        None => hue_name(b).to_string(),
    });
    if m.grey_share >= 0.6 || m.saturation < 0.08 {
        parts.push(match hue {
            Some(h) if m.grey_share < 0.9 => format!("black and white or greyscale with a faint {h} tint"),
            _ => "black and white or greyscale".into(),
        });
    } else if m.saturation >= 0.4 {
        parts.push(match hue {
            Some(h) => format!("saturated colour led by {h}"),
            None => "saturated mixed colour".into(),
        });
    } else if m.saturation < 0.2 {
        parts.push(match hue {
            Some(h) => format!("muted, nearly monochrome {h} tones"),
            None => "muted, nearly monochrome tones".into(),
        });
    } else {
        parts.push(match hue {
            Some(h) => format!("moderate colour leaning {h}"),
            None => "moderate mixed colour".into(),
        });
    }
    if m.value <= 0.35 {
        parts.push("dark".into());
    } else if m.value >= 0.7 {
        parts.push("light".into());
    }
    if m.opaque >= 0.95 {
        parts.push("fills the whole canvas edge to edge".into());
    } else if m.opaque <= 0.6 {
        parts.push("a cut-out subject on a transparent background".into());
    }
    if m.flat_share >= 0.8 {
        parts.push("flat colour areas with crisp edges and no painted shading".into());
    } else if let Some(v) = v {
        if m.edges >= v.edges * 1.4 {
            parts.push("busy, detailed photographic texture".into());
        } else if m.edges <= v.edges * 0.6 {
            parts.push("smooth, soft texture".into());
        }
    }
    if let Some(c) = canvas.filter(|c| *c != vanilla_canvas) {
        let shape = if c[0] * 10 > c[1] * 12 {
            "wide"
        } else if c[1] * 10 > c[0] * 12 {
            "upright"
        } else {
            "square"
        };
        parts.push(format!("{shape} {}x{} shape", c[0], c[1]));
    }
    if let Some(v) = v {
        if m.saturation >= v.saturation + 0.1 {
            parts.push("more saturated than vanilla".into());
        } else if m.saturation + 0.1 <= v.saturation {
            parts.push("less saturated than vanilla".into());
        }
    }
    parts.join(", ")
}

/// Draft form lines for a switch that needs one (the vanilla rules would still ask for vanilla's form).
pub fn draft_form(switches: &[&str]) -> Vec<String> {
    if switches.contains(&"keep_colours") {
        return vec![
            "One flat unit glyph in the reference glyphs' own colours on a transparent background, seen from the side, with no box and no shading."
                .into(),
        ];
    }
    if switches.contains(&"upright_print") {
        return vec![
            "A picture in the reference pictures' style that fills the {w}x{h} frame as they do, upright, with no tilted card, no mask and no sepia."
                .into(),
        ];
    }
    if switches.contains(&"fill_canvas") {
        return vec!["A picture that fills the whole {w}x{h} canvas edge to edge, as the reference panels do, with no transparent margin.".into()];
    }
    if switches.contains(&"in_colour") {
        return vec!["A full-colour photograph in the reference pictures' style that fills the whole frame, composed for a {w}x{h} crop.".into()];
    }
    Vec::new()
}

/// Texture kinds have their own vanilla folders; `pack derive` keys a GUI texture pack entry by family only.
pub fn vanilla_kinds() -> [GuiTextureKind; 4] {
    [GuiTextureKind::Button, GuiTextureKind::Frame, GuiTextureKind::Panel, GuiTextureKind::Background]
}

// ---- contact sheet ----

/// Digits 0 to 9 in a 3x5 pixel font, one row of 3 bits per line.
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

fn draw_number(img: &mut Rgba, n: usize, x0: u32, y0: u32, scale: u32, colour: [u8; 4]) {
    for (i, ch) in n.to_string().bytes().enumerate() {
        let glyph = DIGITS[usize::from(ch - b'0')];
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..3u32 {
                if bits & (4 >> col) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let (x, y) = (x0 + (i as u32 * 4 + col) * scale + dx, y0 + row as u32 * scale + dy);
                            if x < img.width && y < img.height {
                                img.set_px(x, y, colour);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Cell size of the contact sheet.
pub const CELL: u32 = 160;

/// A contact sheet: a row of vanilla images (grey band), the picks (yellow band, numbered from 1) and the alternates
/// (blue band, numbered from 1, eight to a row). Each image is fitted into a cell on a dark ground, small images
/// enlarged by whole steps.
pub fn contact_sheet(vanilla: &[Rgba], picks: &[Rgba], alternates: &[Rgba]) -> crate::Result<Rgba> {
    const BAND: u32 = 6;
    const LABEL: u32 = 14;
    let cols = 8usize.max(picks.len()).max(vanilla.len().min(8));
    let mut rows: Vec<(&[Rgba], [u8; 4], bool)> = Vec::new();
    if !vanilla.is_empty() {
        rows.push((&vanilla[..vanilla.len().min(cols)], [150, 150, 150, 255], false));
    }
    rows.push((picks, [232, 200, 90, 255], true));
    for chunk in alternates.chunks(cols) {
        rows.push((chunk, [90, 150, 232, 255], true));
    }
    let row_h = BAND + CELL + LABEL;
    let mut sheet = Rgba::filled(cols as u32 * CELL, rows.len() as u32 * row_h, [40, 40, 44, 255]);
    let mut alt_no = 0usize;
    for (r, (imgs, colour, numbered)) in rows.iter().enumerate() {
        let y0 = r as u32 * row_h;
        for x in 0..sheet.width {
            for y in y0..y0 + BAND - 2 {
                sheet.set_px(x, y, *colour);
            }
        }
        let is_alt = r > usize::from(!vanilla.is_empty());
        for (c, img) in imgs.iter().enumerate() {
            let (iw, ih) = (img.width.max(1), img.height.max(1));
            let room = CELL - 8;
            let up = (room / iw.max(ih)).clamp(1, 4);
            let fitted = if iw * up > room || ih * up > room {
                let s = f64::from(room) / f64::from(iw.max(ih));
                crate::resize::resize_area(img, ((f64::from(iw) * s) as u32).max(1), ((f64::from(ih) * s) as u32).max(1))?
            } else if up > 1 {
                let mut big = Rgba::new(iw * up, ih * up);
                for y in 0..big.height {
                    for x in 0..big.width {
                        big.set_px(x, y, img.px(x / up, y / up));
                    }
                }
                big
            } else {
                img.clone()
            };
            let x = c as u32 * CELL + (CELL - fitted.width) / 2;
            let y = y0 + BAND + (CELL - fitted.height) / 2;
            sheet.paste_over(&fitted, i64::from(x), i64::from(y));
            if *numbered {
                let n = if is_alt {
                    alt_no += 1;
                    alt_no
                } else {
                    c + 1
                };
                draw_number(&mut sheet, n, c as u32 * CELL + 4, y0 + BAND + CELL + 2, 2, *colour);
            }
        }
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: [u32; 2], colour: [u8; 4], inset: u32) -> Rgba {
        let mut img = Rgba::new(size[0], size[1]);
        for y in inset..size[1] - inset {
            for x in inset..size[0] - inset {
                img.set_px(x, y, colour);
            }
        }
        img
    }

    #[test]
    fn dominant_cluster_merges_near_sizes_and_takes_the_common_one() {
        let mut sizes = vec![[170, 150]; 6];
        sizes.extend([[175, 150]; 4]);
        sizes.extend([[160, 140]; 3]);
        sizes.extend([[100, 88]; 9]);
        let (canvas, member) = dominant_cluster(&sizes).unwrap();
        assert_eq!(canvas, [170, 150], "13 files near 170x150 outnumber 9 at 100x88");
        assert_eq!(member.iter().filter(|m| **m).count(), 13);
        assert!(dominant_cluster(&[]).is_none());
    }

    #[test]
    fn event_pictures_split_into_report_and_news_by_count_and_name() {
        let mut all = Vec::new();
        for i in 0..30 {
            all.push(FileInfo { rel: format!("gfx/event_pictures/a{i}.png"), abs: PathBuf::new(), size: [500, 250] });
        }
        for i in 0..10 {
            all.push(FileInfo { rel: format!("gfx/event_pictures/b{i}.png"), abs: PathBuf::new(), size: [400, 150] });
        }
        all.push(FileInfo { rel: "gfx/event_pictures/placeholder.png".into(), abs: PathBuf::new(), size: [500, 250] });
        let report = spec_of(Family::ReportEvent).unwrap();
        let news = spec_of(Family::NewsEvent).unwrap();
        let (r, rc, dropped) = family_files(&report, &all);
        assert_eq!((r.len(), rc), (30, Some([500, 250])));
        assert_eq!(dropped["placeholder_name"], 1);
        let (n, nc, _) = family_files(&news, &all);
        assert_eq!((n.len(), nc), (10, Some([400, 150])));
        // a mod whose larger cluster is named as news pictures swaps the two
        for f in all.iter_mut().filter(|f| f.size == [500, 250]) {
            f.rel = f.rel.replace("/a", "/news_");
        }
        assert_eq!(family_files(&news, &all).1, Some([500, 250]));
        assert_eq!(family_files(&report, &all).1, Some([400, 150]));
    }

    #[test]
    fn name_filters_drop_placeholders_and_other_families() {
        assert!(is_placeholder_name("gfx/interface/goals/focus_placeholder.dds"));
        assert!(is_placeholder_name("gfx/leaders/Portrait_Unknown.png"));
        assert!(!is_placeholder_name("gfx/interface/goals/GER_attempt_two.dds"), "a word that merely contains a token");
        let spirit = spec_of(Family::IdeaSpirit).unwrap();
        let all = vec![
            FileInfo { rel: "gfx/interface/ideas/SOV/advisors/a.png".into(), abs: PathBuf::new(), size: [62, 67] },
            FileInfo { rel: "gfx/interface/ideas/SOV/b.png".into(), abs: PathBuf::new(), size: [68, 68] },
        ];
        let (files, _, dropped) = family_files(&spirit, &all);
        assert_eq!(files.len(), 1);
        assert_eq!(dropped["other_family_name"], 1);
    }

    #[test]
    fn dhash_tells_near_copies_from_other_art() {
        let a = square([64, 64], [200, 40, 40, 255], 8);
        let mut b = a.clone();
        b.set_px(20, 20, [0, 0, 0, 255]);
        let c = square([64, 64], [200, 40, 40, 255], 24);
        assert!(hamming(dhash(&a), dhash(&b)) <= NEAR_COPY_BITS);
        let mut stripes = Rgba::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let v = if (x / 8 + y / 8) % 2 == 0 { 255 } else { 0 };
                stripes.set_px(x, y, [v, v, v, 255]);
            }
        }
        assert!(hamming(dhash(&a), dhash(&stripes)) > NEAR_COPY_BITS);
        let (_, style, stats) = measure(&c).unwrap();
        assert_eq!(style.len(), STYLE_DIMS);
        assert!(stats.flat_share > 0.99 && stats.grey_share < 0.01);
        assert!(!is_flat(&style, &stats), "a flat shape on transparency is not");
        let (_, style, stats) = measure(&Rgba::filled(64, 64, [90, 90, 90, 255])).unwrap();
        assert!(is_flat(&style, &stats), "a filled canvas of one tone is a placeholder");
        assert!(is_flat(&measure(&Rgba::new(8, 8)).unwrap().1, &measure(&Rgba::new(8, 8)).unwrap().2), "an empty image too");
    }

    #[test]
    fn a_scan_reuses_known_sizes_and_rereads_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        let goals = dir.path().join("gfx/interface/goals/GER");
        std::fs::create_dir_all(&goals).unwrap();
        std::fs::write(goals.join("a.png"), crate::pixels::encode_png(&square([20, 10], [9, 9, 9, 255], 1)).unwrap()).unwrap();
        std::fs::write(goals.join("notes.txt"), b"not an image").unwrap();
        let (files, known) = scan(dir.path(), "gfx/interface/goals", true, &KnownSizes::new());
        assert_eq!(files.len(), 1, "only image files are listed");
        assert_eq!((files[0].rel.as_str(), files[0].size), ("gfx/interface/goals/GER/a.png", [20, 10]));
        // a known size with the same stamp is taken as it is, without opening the file
        let mut fake = known.clone();
        fake.get_mut("gfx/interface/goals/GER/a.png").unwrap().1 = [7, 7];
        assert_eq!(scan(dir.path(), "gfx/interface/goals", true, &fake).0[0].size, [7, 7]);
        // a changed file is read again
        std::fs::write(goals.join("a.png"), crate::pixels::encode_png(&square([30, 12], [9, 9, 9, 255], 1)).unwrap()).unwrap();
        assert_eq!(scan(dir.path(), "gfx/interface/goals", true, &fake).0[0].size, [30, 12]);
    }

    #[test]
    fn the_cache_round_trips_and_refuses_damage() {
        let (dhash, style, stats) = measure(&square([40, 30], [10, 200, 190, 255], 4)).unwrap();
        let rows = vec![
            Measured { rel: "gfx/a.dds".into(), size: [40, 30], sha256: "ab".repeat(32), dhash, style, stats },
            Measured { rel: "gfx/b.png".into(), size: [8, 8], sha256: "cd".repeat(32), dhash: 7, style: Vec::new(), stats },
        ];
        let bytes = encode_cache(&rows);
        assert_eq!(decode_cache(&bytes).unwrap(), rows);
        assert!(decode_cache(&bytes[..bytes.len() - 1]).is_none());
        assert!(decode_cache(b"nope").is_none());
    }

    #[test]
    fn mostly_vanilla_needs_many_near_copies_and_a_vanilla_palette() {
        assert!(vanilla_like(35, 71, Some(1.38)).unwrap().contains("35 of 71"), "TFR's large counters");
        assert!(vanilla_like(1831, 2456, Some(1.48)).is_none(), "TNO's leader portraits keep their own palette");
        assert!(vanilla_like(10, 757, Some(1.1)).is_none(), "few near-copies");
        assert!(vanilla_like(50, 60, None).is_none(), "no vanilla to compare with");
        let glyph = square([60, 40], [76, 198, 192, 255], 10);
        let (_, style, stats) = measure(&glyph).unwrap();
        assert!(!is_flat(&style, &stats), "a flat glyph on transparency is a style, not a placeholder");
    }

    #[test]
    fn kmeans_separates_groups_and_repeats() {
        let mut rows = Vec::new();
        for i in 0..30 {
            let g = (i % 3) as f32 * 10.0;
            rows.push(vec![g + (i as f32) * 0.01, g]);
        }
        let a = kmeans(&rows, 3, 7);
        assert_eq!(a, kmeans(&rows, 3, 7), "seeded: the same split every run");
        for i in 0..30 {
            assert_eq!(a[i], a[i % 3], "row {i} is with its group");
        }
        assert_ne!(a[0], a[1]);
        assert_ne!(a[1], a[2]);
    }

    #[test]
    fn the_look_draft_reads_the_figures() {
        let m = Figures {
            files: 20,
            saturation: 0.55,
            value: 0.3,
            grey_share: 0.05,
            opaque: 0.98,
            edges: 0.2,
            flat_share: 0.4,
            hue: Some((6, 0.5)),
            hue2: None,
        };
        let v = Figures { files: 20, saturation: 0.3, value: 0.5, grey_share: 0.2, opaque: 0.7, edges: 0.1, flat_share: 0.3, hue: None, hue2: None };
        let look = draft_look(&m, Some(&v), Some([500, 250]), [210, 176]);
        assert_eq!(look, "saturated colour led by teal, dark, fills the whole canvas edge to edge, busy, detailed photographic texture, wide 500x250 shape, more saturated than vanilla");
        let sheet = contact_sheet(
            &[square([40, 40], [9, 9, 9, 255], 2)],
            &vec![square([40, 40], [200, 9, 9, 255], 2); 3],
            &vec![square([400, 100], [9, 9, 200, 255], 0); 9],
        )
        .unwrap();
        assert_eq!((sheet.width, sheet.height), (8 * CELL, 4 * (CELL + 20)), "a vanilla row, a pick row and two alternate rows");
    }
}
