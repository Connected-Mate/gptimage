//! `pack derive`: a style pack made from a mod's own art in one step (see `imagegen_core::derive` for the method).
//!
//! The command reads the mod's `gfx/` folders and the installed game, picks references per family, copies them into
//! this project's pack folder as PNG, writes the canvas, a draft look, draft form lines and the processing switches
//! into `pack.toml`, and records every figure and pick in `derive.json` beside it. The contact sheets show vanilla
//! art, so they go to the user's cache folder, never into the mod.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use imagegen_core::derive::{self, FileInfo, Measured, Pick, Spec};
use imagegen_core::error::RetryClass;
use imagegen_core::family::Family;
use imagegen_core::fsio::display_path;
use imagegen_core::hashing::sha256_hex;
use imagegen_core::pack::{self, PackEntry, ProcessingSwitches, StylePack, NO_PACK};
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba};
use imagegen_core::profile::ProfileSet;
use imagegen_core::style::StyleRef;
use serde_json::{json, Value};

use super::{EResult, Engine, EngineError};

/// Version of the derive method, recorded in `derive.json`.
pub const DERIVE_VERSION: &str = "pack_derive_v1";
/// File beside `pack.toml` with the figures and picks of the last derive.
pub const DERIVE_FILE: &str = "derive.json";

/// What `pack derive` is asked to do.
#[derive(Debug, Clone, Default)]
pub struct DeriveOptions {
    /// The mod folder to read (the project itself when `None`).
    pub from_mod: Option<PathBuf>,
    /// Only these families (all that `pack derive` covers when empty). Other families of an existing pack are kept.
    pub families: Vec<Family>,
    /// References per family ([`derive::DEFAULT_PICKS`] when `None`).
    pub picks: Option<usize>,
    /// Swaps: `focus:3=a2` puts alternate 2 in place of pick 3, `focus:3=gfx/interface/goals/x.dds` a file of the
    /// family by its path in the mod.
    pub swap: Vec<String>,
    /// Picks to leave out: `focus:3`.
    pub drop: Vec<String>,
    /// Replace an existing pack (its file is backed up).
    pub replace: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum With {
    Alternate(usize),
    Path(String),
}

/// Parse `family:n` (and `=x` for a swap).
fn parse_edit(s: &str, swap: bool) -> EResult<(Family, usize, Option<With>)> {
    let bad = || {
        EngineError::invalid(format!(
            "'{s}': write {}",
            if swap {
                "family:pick=aN (an alternate) or family:pick=<path in the mod>, for example focus:3=a2"
            } else {
                "family:pick, for example focus:3"
            }
        ))
    };
    let (left, right) = match s.split_once('=') {
        Some((l, r)) if swap => (l, Some(r.trim())),
        None if !swap => (s, None),
        _ => return Err(bad()),
    };
    let (fam, n) = left.trim().split_once(':').ok_or_else(bad)?;
    let family: Family = fam.trim().parse().map_err(|_| bad())?;
    let n: usize = n.trim().parse().map_err(|_| bad())?;
    if n == 0 {
        return Err(bad());
    }
    let with = match right {
        None => None,
        Some(r) => match r.strip_prefix('a').and_then(|x| x.parse::<usize>().ok()) {
            Some(a) if a > 0 => Some(With::Alternate(a)),
            _ if !r.is_empty() => Some(With::Path(r.replace('\\', "/"))),
            _ => return Err(bad()),
        },
    };
    Ok((family, n, with))
}

fn seed_of(family: Family) -> u64 {
    family.as_str().bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16)
}

/// The mod's name from `descriptor.mod`, else its folder name. Never the full path, which may be private.
fn mod_label(root: &Path) -> String {
    let named = std::fs::read_to_string(root.join("descriptor.mod")).ok().and_then(|t| {
        t.lines().find_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("name")?.trim_start().strip_prefix('=')?.trim();
            Some(rest.trim_matches('"').to_string())
        })
    });
    named.filter(|n| !n.is_empty()).unwrap_or_else(|| root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "mod".into()))
}

fn load_image(path: &Path) -> Option<Rgba> {
    let bytes = imagegen_core::fsio::read_limited(path, imagegen_core::refs::MAX_REFERENCE_BYTES).ok()?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string());
    decode(&bytes, name.as_deref(), DecodeLimits::default()).ok().map(|(img, _)| img)
}

/// Write measurements to the cache. A cache that cannot be written only costs time on the next run.
fn write_cache(path: Option<&Path>, rows: &[Measured]) {
    if let Some(dir) = path.and_then(Path::parent) {
        let _ = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(path.expect("a path with a parent"), derive::encode_cache(rows)));
    }
}

/// One family's result before it is written.
struct Derived {
    spec: Spec,
    report: Value,
    picks: Vec<Pick>,
    alternates: Vec<Pick>,
    measured: Vec<Measured>,
    entry: Option<PackEntry>,
    vanilla_sample: Vec<PathBuf>,
}

impl Engine {
    /// The family's vanilla files from the installed game, measured: every file carries its dHash for the near-copy
    /// check, and an even sample of [`derive::MAX_FILES`] its style vector. Kept in the user's cache folder, keyed by
    /// the files' paths, sizes and times, so a second run reads no vanilla file.
    fn derive_vanilla(&self, game: &Path, family: Family) -> Vec<Measured> {
        let mut paths = if family == Family::GuiTexture {
            derive::vanilla_kinds().iter().flat_map(|k| imagegen_core::stylesrc::vanilla_kind_files(game, *k)).collect::<Vec<_>>()
        } else {
            imagegen_core::stylesrc::vanilla_files(game, family, &[])
        };
        paths.sort();
        paths.dedup();
        let cache = self.measure_cache("vanilla", family, paths.iter().map(PathBuf::as_path));
        if let Some(rows) = cache.as_ref().and_then(|c| std::fs::read(c).ok()).and_then(|b| derive::decode_cache(&b)) {
            return rows;
        }
        let files: Vec<FileInfo> = paths
            .into_iter()
            .filter_map(|abs| {
                let size = imagegen_core::stylesrc::header_size(&abs)?;
                Some(FileInfo { rel: display_path(&abs).replace('\\', "/"), abs, size })
            })
            .collect();
        let mut rows = derive::measure_files(&files, threads());
        let keep: std::collections::HashSet<String> =
            derive::sample(&rows.iter().map(|m| m.rel.clone()).collect::<Vec<_>>(), derive::MAX_FILES).into_iter().collect();
        for m in &mut rows {
            if !keep.contains(&m.rel) {
                m.style = Vec::new();
            }
        }
        write_cache(cache.as_deref(), &rows);
        rows
    }

    /// Where the measurements of a set of files are cached: the user's cache folder, keyed by the method's version and
    /// each file's path, size and time, so a changed file is measured again.
    fn measure_cache<'a>(&self, side: &str, family: Family, paths: impl Iterator<Item = &'a Path>) -> Option<PathBuf> {
        let mut key = format!("{DERIVE_VERSION}|{}|{family}", derive::MAX_FILES);
        for p in paths {
            let meta = std::fs::metadata(p).ok();
            let time =
                meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            key.push_str(&format!("|{}:{}:{time}", display_path(p), meta.map_or(0, |m| m.len())));
        }
        self.user_cache_dir().map(|d| d.join("pack-derive").join(side).join(format!("{family}-{}.bin", &sha256_hex(key.as_bytes())[..16])))
    }

    /// The mod's files of a family, measured, from the cache when none changed.
    fn derive_measure_mod(&self, family: Family, files: &[FileInfo]) -> Vec<Measured> {
        let cache = self.measure_cache("mod", family, files.iter().map(|f| f.abs.as_path()));
        if let Some(rows) = cache.as_ref().and_then(|c| std::fs::read(c).ok()).and_then(|b| derive::decode_cache(&b)) {
            return rows;
        }
        let rows = derive::measure_files(files, threads());
        write_cache(cache.as_deref(), &rows);
        rows
    }

    fn derive_family(&self, spec: Spec, all: &[FileInfo], game: &Path, picks_n: usize) -> Derived {
        let family = spec.family;
        let (files, canvas, dropped) = derive::family_files(&spec, all);
        let mut report = json!({"family": family, "folder": spec.dir, "files_in_folder": all.len(), "filtered_out": dropped, "files_of_family": files.len(), "canvas": canvas});
        let skip = |mut report: Value, why: String, measured: Vec<Measured>| {
            report["status"] = json!("skipped");
            report["reason"] = json!(why);
            Derived { spec: spec.clone(), report, picks: Vec::new(), alternates: Vec::new(), measured, entry: None, vanilla_sample: Vec::new() }
        };
        if files.len() < derive::MIN_FILES {
            let why = if all.is_empty() {
                format!("the mod has no art in {}", spec.dir)
            } else {
                format!("only {} files of the family in {} (at least {} are needed)", files.len(), spec.dir, derive::MIN_FILES)
            };
            return skip(report, why, Vec::new());
        }
        let sampled = derive::sample(&files, derive::MAX_FILES);
        let raw = self.derive_measure_mod(family, &sampled);
        let mut seen = std::collections::HashSet::new();
        let (mut duplicates, mut flat) = (0usize, 0usize);
        let mut measured = Vec::new();
        for m in raw.iter() {
            if !seen.insert(m.sha256.clone()) {
                duplicates += 1;
            } else if derive::is_flat(&m.style, &m.stats) {
                flat += 1;
            } else {
                measured.push(m.clone());
            }
        }
        report["sampled"] = json!(sampled.len());
        report["unreadable"] = json!(sampled.len() - raw.len());
        report["exact_duplicates"] = json!(duplicates);
        report["flat_or_empty"] = json!(flat);
        let van = self.derive_vanilla(game, family);
        report["vanilla_files"] = json!(van.len());
        let ranking = derive::rank(&measured, &van, spec.typical, threads());
        let eligible: Vec<usize> = (0..measured.len()).filter(|i| ranking.hamming[*i] > derive::NEAR_COPY_BITS).collect();
        let copies = measured.len() - eligible.len();
        report["near_copies_of_vanilla"] = json!(copies);
        report["usable"] = json!(eligible.len());
        report["distinctness"] = json!(ranking.distinctness.map(|d| (d * 1000.0).round() / 1000.0));
        report["palette_distinctness"] = json!(ranking.palette_distinctness.map(|d| (d * 1000.0).round() / 1000.0));
        let profile = ProfileSet::builtin().for_family(family).cloned().expect("every family has a profile");
        let vanilla_canvas = [profile.canvas[0] * profile.frames.max(1), profile.canvas[1]];
        let mod_figures = derive::figures(&eligible.iter().map(|i| &measured[*i].stats).collect::<Vec<_>>());
        let van_figures = (!van.is_empty()).then(|| derive::figures(&van.iter().map(|m| &m.stats).collect::<Vec<_>>()));
        report["figures"] = json!({"mod": mod_figures, "vanilla": van_figures});
        if eligible.len() < derive::MIN_FILES {
            let why = format!(
                "{copies} of {} measured files are near-copies of vanilla (within {} dHash bits), and {} remain (at least {} are needed)",
                measured.len(),
                derive::NEAR_COPY_BITS,
                eligible.len(),
                derive::MIN_FILES
            );
            return skip(report, why, measured);
        }
        if let Some(why) = derive::vanilla_like(copies, measured.len(), ranking.palette_distinctness) {
            return skip(report, why, measured);
        }
        let (picks, alternates) = derive::pick_diverse(&measured, &ranking, &eligible, spec.dir, picks_n, seed_of(family));
        let pack_canvas = canvas.filter(|c| *c != vanilla_canvas && profile.frames <= 1 && !profile.needs_mod_canvas());
        if let Some(c) = canvas.filter(|c| *c != vanilla_canvas && profile.frames > 1) {
            report["note"] = json!(format!(
                "the mod's strips are {}x{}, the family's are {}x{}; a strip family keeps its canvas",
                c[0], c[1], vanilla_canvas[0], vanilla_canvas[1]
            ));
        }
        let switches = derive::suggest(family, &profile, canvas, &mod_figures, van_figures.as_ref());
        let names: Vec<&str> = switches.iter().map(|(s, _)| *s).collect();
        let look = derive::draft_look(&mod_figures, van_figures.as_ref(), canvas, vanilla_canvas);
        let processing = ProcessingSwitches {
            keep_colours: names.contains(&"keep_colours"),
            upright_print: names.contains(&"upright_print"),
            fill_canvas: names.contains(&"fill_canvas"),
            crisp: names.contains(&"crisp"),
            in_colour: names.contains(&"in_colour"),
            ..Default::default()
        };
        let form = derive::draft_form(&names);
        report["status"] = json!("derived");
        report["pack_canvas"] = json!(pack_canvas);
        report["vanilla_canvas"] = json!(vanilla_canvas);
        report["look"] = json!(look);
        report["form"] = json!(form);
        report["switches"] = json!(switches.iter().map(|(s, why)| json!({"switch": s, "why": why})).collect::<Vec<_>>());
        let entry = PackEntry { look: Some(look), form, canvas: pack_canvas, processing, ..Default::default() };
        // the vanilla row of the sheet: vanilla's most typical files for the family
        let typical: Vec<PathBuf> = {
            let styled: Vec<&Measured> = van.iter().filter(|m| !m.style.is_empty()).collect();
            let mut idx: Vec<usize> = (0..styled.len()).collect();
            let z: Vec<Vec<f32>> = styled.iter().map(|m| m.style.clone()).collect();
            let d = derive::knn_mean(&z, &z, derive::KNN, true, threads());
            idx.sort_by(|a, b| d[*a].total_cmp(&d[*b]));
            // the most typical files that are not near-copies of each other (vanilla repeats some art many times)
            let mut chosen: Vec<&Measured> = Vec::new();
            for i in idx {
                if chosen.len() == 8 {
                    break;
                }
                if chosen.iter().all(|c| derive::hamming(c.dhash, styled[i].dhash) > derive::NEAR_COPY_BITS) {
                    chosen.push(styled[i]);
                }
            }
            chosen.into_iter().map(|m| PathBuf::from(&m.rel)).collect()
        };
        Derived { spec, report, picks, alternates, measured, entry: Some(entry), vanilla_sample: typical }
    }

    /// Derive a style pack from a mod's art and save it ([`DeriveOptions`]). See `imagegen_core::derive`.
    pub fn derive_pack(&self, name: &str, o: &DeriveOptions) -> EResult<Value> {
        imagegen_core::naming::validate_snake("pack name", name)?;
        if name == NO_PACK {
            return Err(EngineError::invalid(format!("'{NO_PACK}' is reserved")));
        }
        let picks_n = o.picks.unwrap_or(derive::DEFAULT_PICKS);
        if !(1..=derive::MAX_PICKS).contains(&picks_n) {
            return Err(EngineError::invalid(format!("picks is 1 to {}", derive::MAX_PICKS)));
        }
        let swaps = o.swap.iter().map(|s| parse_edit(s, true)).collect::<EResult<Vec<_>>>()?;
        let drops = o.drop.iter().map(|s| parse_edit(s, false)).collect::<EResult<Vec<_>>>()?;
        let existing = match pack::load(&self.fs, name) {
            Ok(p) if o.replace => Some(p),
            Ok(_) => {
                return Err(EngineError::new(
                    "exists",
                    format!("style pack '{name}' already exists; pass replace to derive over it (the old file is backed up, and families not derived this time are kept)"),
                    RetryClass::Terminal,
                ))
            }
            Err(imagegen_core::Error::NotFound(_)) => None,
            Err(e) => return Err(e.into()),
        };
        let root = o.from_mod.clone().unwrap_or_else(|| self.fs.root().to_path_buf());
        if !root.join("gfx").is_dir() {
            return Err(EngineError::new(
                "not_found",
                format!("{} has no gfx folder: name the mod folder itself", display_path(&root)),
                RetryClass::Terminal,
            ));
        }
        let game = self.game_dir().ok_or_else(|| {
            EngineError::refused(
                "game_not_found",
                "the installed game was not found (set HOI4_GAME_DIR or game_dir in .hoi4-imagegen/config.toml): the near-copy filter and the ranking need vanilla's art",
                RetryClass::RefreshPreconditions,
            )
        })?;
        let specs: Vec<Spec> = derive::specs().into_iter().filter(|s| o.families.is_empty() || o.families.contains(&s.family)).collect();
        for f in &o.families {
            if derive::spec_of(*f).is_none() {
                let covered: Vec<&str> = derive::specs().iter().map(|s| s.family.as_str()).collect();
                return Err(EngineError::invalid(format!("pack derive does not cover {f}; it covers {}", covered.join(", "))));
            }
        }
        // header sizes read before, keyed by each file's length and time, so a second run opens only changed files
        let scan_cache = self
            .user_cache_dir()
            .map(|d| d.join("pack-derive").join("scan").join(format!("{}.json", &sha256_hex(display_path(&root).as_bytes())[..16])));
        let known: derive::KnownSizes =
            scan_cache.as_ref().and_then(|c| std::fs::read(c).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let mut seen = derive::KnownSizes::new();
        let mut scans: HashMap<(&str, bool), Vec<FileInfo>> = HashMap::new();
        let mut results = Vec::new();
        for spec in specs {
            let all = scans
                .entry((spec.dir, spec.recursive))
                .or_insert_with(|| {
                    let (files, now) = derive::scan(&root, spec.dir, spec.recursive, &known);
                    seen.extend(now);
                    files
                })
                .clone();
            results.push(self.derive_family(spec, &all, &game, picks_n));
        }
        if let (Some(c), Ok(body)) = (&scan_cache, serde_json::to_vec(&seen)) {
            if let Some(dir) = c.parent() {
                let _ = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(c, body));
            }
        }
        // the swaps and drops, by the numbers the last run showed
        for (family, n, with) in &swaps {
            let r = results
                .iter_mut()
                .find(|r| r.spec.family == *family && r.entry.is_some())
                .ok_or_else(|| EngineError::invalid(format!("{family} was not derived in this run, so nothing can be swapped")))?;
            if *n > r.picks.len() {
                return Err(EngineError::invalid(format!("{family} has {} picks; there is no pick {n}", r.picks.len())));
            }
            let replacement = match with.as_ref().expect("a swap has a replacement") {
                With::Alternate(a) => r
                    .alternates
                    .get(a - 1)
                    .cloned()
                    .ok_or_else(|| EngineError::invalid(format!("{family} has {} alternates; there is no a{a}", r.alternates.len())))?,
                With::Path(p) => {
                    let i = r.measured.iter().position(|m| m.rel == *p).ok_or_else(|| {
                        EngineError::invalid(format!(
                            "{p} is not among the measured {family} files of the mod (a path in the mod, as derive.json lists them)"
                        ))
                    })?;
                    Pick { index: i, rel: p.clone(), score: f64::NAN, vanilla_hamming: 64, cluster: usize::MAX }
                }
            };
            r.picks[n - 1] = replacement;
        }
        let mut drop_by: BTreeMap<Family, Vec<usize>> = BTreeMap::new();
        for (family, n, _) in &drops {
            drop_by.entry(*family).or_default().push(*n);
        }
        for (family, ns) in &drop_by {
            let r = results
                .iter_mut()
                .find(|r| r.spec.family == *family && r.entry.is_some())
                .ok_or_else(|| EngineError::invalid(format!("{family} was not derived in this run")))?;
            let keep: Vec<Pick> = r.picks.iter().enumerate().filter(|(i, _)| !ns.contains(&(i + 1))).map(|(_, p)| p.clone()).collect();
            r.picks = keep;
        }
        if results.iter().all(|r| r.entry.is_none()) {
            let why: Vec<String> =
                results.iter().map(|r| format!("{}: {}", r.spec.family, r.report["reason"].as_str().unwrap_or("skipped"))).collect();
            return Err(EngineError::new(
                "nothing_derived",
                format!("no family has a house style to derive: {}", why.join("; ")),
                RetryClass::Terminal,
            )
            .with_details(json!({"families": results.iter().map(|r| r.report.clone()).collect::<Vec<_>>()})));
        }
        let label = mod_label(&root);
        let dir = pack::pack_dir(name)?;
        let sheet_dir = self.project_cache_dir("pack-derive").map(|d| d.join(name));
        let mut p = existing.clone().unwrap_or(StylePack {
            schema: 1,
            name: name.to_string(),
            description: String::new(),
            approved_by: None,
            approved_on: None,
            families: BTreeMap::new(),
        });
        p.description =
            format!("{label} house style, derived from the mod's own art by pack derive ({DERIVE_VERSION}); review the contact sheets before use");
        let mut families = serde_json::Map::new();
        let mut sheets = serde_json::Map::new();
        for r in &mut results {
            let family = r.spec.family;
            let Some(mut entry) = r.entry.take() else {
                families.insert(family.as_str().to_string(), r.report.clone());
                continue;
            };
            let mut pick_images = Vec::new();
            let mut listed = Vec::new();
            for (n, pk) in r.picks.iter().enumerate() {
                let src = root.join(&pk.rel);
                let Some(img) = load_image(&src) else { continue };
                let png = encode_png(&img)?;
                let rel = format!("{dir}/{family}/{name}_{family}_ref_{}.png", n + 1);
                if self.fs.exists(&rel) {
                    if self.fs.read(&rel, imagegen_core::refs::MAX_REFERENCE_BYTES)? != png {
                        self.fs.replace(&rel, &png)?;
                    }
                } else {
                    self.fs.write_new(&rel, &png)?;
                }
                let source_sha = &r.measured[pk.index].sha256;
                entry.references.push(StyleRef {
                    path: rel.clone(),
                    sha256: sha256_hex(&png),
                    note: format!("derived by pack derive from {} in {label} (source sha256 {source_sha})", pk.rel),
                });
                listed.push(json!({"n": n + 1, "source": pk.rel, "reference": rel, "score": (pk.score.is_finite()).then(|| (pk.score * 1000.0).round() / 1000.0), "vanilla_hamming": pk.vanilla_hamming, "cluster": (pk.cluster != usize::MAX).then_some(pk.cluster), "source_sha256": source_sha}));
                pick_images.push(img);
            }
            let alt_listed: Vec<Value> = r
                .alternates
                .iter()
                .enumerate()
                .map(|(i, a)| json!({"n": format!("a{}", i + 1), "source": a.rel, "score": (a.score * 1000.0).round() / 1000.0, "vanilla_hamming": a.vanilla_hamming, "cluster": a.cluster}))
                .collect();
            let alt_images: Vec<Rgba> = r.alternates.iter().filter_map(|a| load_image(&root.join(&a.rel))).collect();
            let van_images: Vec<Rgba> = r.vanilla_sample.iter().filter_map(|p| load_image(p)).collect();
            let sheet = derive::contact_sheet(&van_images, &pick_images, &alt_images)?;
            let png = encode_png(&sheet)?;
            let sheet_path = sheet_dir.as_ref().and_then(|d| {
                let file = d.join(format!("{family}.png"));
                std::fs::create_dir_all(d).and_then(|_| std::fs::write(&file, &png)).ok().map(|_| file)
            });
            if let Some(s) = &sheet_path {
                sheets.insert(family.as_str().to_string(), json!(display_path(s)));
            }
            r.report["picks"] = json!(listed);
            r.report["alternates"] = json!(alt_listed);
            r.report["sheet"] = json!(format!("{family}.png"));
            families.insert(family.as_str().to_string(), r.report.clone());
            p.families.insert(family, entry);
        }
        let saved = self.save_pack(p, existing.is_some())?;
        let derived: Vec<&str> = results.iter().filter(|r| r.report["status"] == json!("derived")).map(|r| r.spec.family.as_str()).collect();
        let skipped: BTreeMap<&str, Value> =
            results.iter().filter(|r| r.report["status"] == json!("skipped")).map(|r| (r.spec.family.as_str(), r.report["reason"].clone())).collect();
        let review = json!([
            "Open each family's contact sheet: the grey row is vanilla, the yellow row the picks (numbered from 1), the blue rows the alternates (a1 onward).",
            "Swap out any pick that shows an extremist symbol or flag, a recognisable real person, a placeholder, a near-duplicate of another pick, or art that is not this family's: --swap family:N=aM (or swap: [\"family:N=aM\"]), or --drop family:N.",
            "Read each family's look line against its sheet and name the subject matter the measurement cannot see (photo collage, blueprint, wreath and so on) in pack.toml.",
            "Rerun with replace and the same swaps to rebuild the pack, then pack validate.",
        ]);
        let record = json!({
            "version": DERIVE_VERSION,
            "pack": name,
            "from_mod": label,
            "game_version": imagegen_core::config::game_version(&game),
            "rules": {"near_copy_bits": derive::NEAR_COPY_BITS, "max_files": derive::MAX_FILES, "pool": derive::POOL, "knn": derive::KNN, "per_folder": derive::PER_FOLDER, "copy_share_max": derive::COPY_SHARE_MAX, "palette_min": derive::PALETTE_MIN, "size_tolerance": derive::SIZE_TOLERANCE, "aspect_tolerance": derive::ASPECT_TOLERANCE, "picks": picks_n},
            "swaps": o.swap,
            "drops": o.drop,
            "families": families,
            "review": review,
        });
        let derive_rel = format!("{dir}/{DERIVE_FILE}");
        let body = serde_json::to_vec_pretty(&record).map_err(|e| EngineError::new("io", e.to_string(), RetryClass::Terminal))?;
        if self.fs.exists(&derive_rel) {
            self.fs.replace(&derive_rel, &body)?;
        } else {
            self.fs.write_new(&derive_rel, &body)?;
        }
        let check = self.pack_validate(name)?;
        let summary: BTreeMap<&str, Value> = results
            .iter()
            .filter(|r| r.report["status"] == json!("derived"))
            .map(|r| {
                (
                    r.spec.family.as_str(),
                    json!({"picks": r.report["picks"].as_array().map_or(0, Vec::len), "alternates": r.alternates.len(), "canvas": r.report["pack_canvas"], "look": r.report["look"], "switches": r.report["switches"], "near_copies_of_vanilla": r.report["near_copies_of_vanilla"], "usable": r.report["usable"], "sheet": sheets.get(r.spec.family.as_str())}),
                )
            })
            .collect();
        Ok(json!({
            "derived": name,
            "from_mod": label,
            "families": summary,
            "skipped": skipped,
            "saved": saved["saved"],
            "sha256": saved["sha256"],
            "record": derive_rel,
            "sheets_folder": sheet_dir.map(|d| display_path(&d)),
            "validation": check,
            "review": review,
            "count": {"derived": derived.len(), "skipped": skipped.len()},
            "next": "Review the contact sheets (review), swap or drop picks and rerun with replace, then plan with style_pack or set [defaults] style_pack in .hoi4-imagegen/config.toml.",
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_and_drop_syntax() {
        assert_eq!(parse_edit("focus:3=a2", true).unwrap(), (Family::Focus, 3, Some(With::Alternate(2))));
        assert_eq!(
            parse_edit("decision:1=gfx\\interface\\decisions\\x.dds", true).unwrap(),
            (Family::Decision, 1, Some(With::Path("gfx/interface/decisions/x.dds".into())))
        );
        assert_eq!(parse_edit("focus:8", false).unwrap(), (Family::Focus, 8, None));
        for bad in ["focus:0=a1", "focus=a1", "nope:1=a1", "focus:1=", "focus:x=a1"] {
            assert!(parse_edit(bad, true).is_err(), "{bad}");
        }
        assert!(parse_edit("focus:1=a2", false).is_err(), "a drop has no replacement");
    }
}
