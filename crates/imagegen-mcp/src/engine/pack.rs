//! Style packs in the engine: which pack a plan uses, the effective profile of an artifact made with a pack, the
//! review's pack reference set, the pack actions of `hoi4.image_references` and the import of a reference folder.
//!
//! See `imagegen_core::pack` for the format.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use imagegen_core::error::RetryClass;
use imagegen_core::family::{Family, GuiTextureKind};
use imagegen_core::hashing::sha256_hex;
use imagegen_core::pack::{self, KindEntry, PackEntry, StylePack, NO_PACK};
use imagegen_core::pixels::{decode, DecodeLimits};
use imagegen_core::profile::Profile;
use imagegen_core::records::{kind, CandidateRecord};
use imagegen_core::refs::{self, ReferenceInput};
use imagegen_core::style::StyleRef;
use imagegen_core::vanillamatch::{self, VanillaImage, VanillaSet};
use serde_json::{json, Value};

use super::{Artifact, EResult, Engine, EngineError};

/// The pack a plan uses and why.
pub(crate) struct PackChoice {
    /// The pack.
    pub pack: StylePack,
    /// SHA-256 of its file.
    pub sha256: String,
    /// `explicit` or `project_default`.
    pub why: &'static str,
}

/// Image files a folder import takes.
const IMAGE_EXTS: [&str; 6] = [".png", ".dds", ".tga", ".jpg", ".jpeg", ".webp"];

fn is_image(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    IMAGE_EXTS.iter().any(|e| l.ends_with(e)) && !l.starts_with("contact-sheet") && !l.starts_with("contact_sheet")
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 120 && !name.starts_with('.') && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn read_json(path: &Path) -> Option<Value> {
    let bytes = imagegen_core::fsio::read_limited(path, 4 * 1024 * 1024).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn first_line(path: &Path) -> Option<String> {
    let bytes = imagegen_core::fsio::read_limited(path, 64 * 1024).ok()?;
    String::from_utf8(bytes).ok()?.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

/// One reference file found by a folder import.
struct Found {
    file: String,
    source: Option<String>,
    kind: Option<GuiTextureKind>,
}

/// The references a family folder lists: `references.json` (or `manifest.json`) with a `references` list of `{ref}`
/// objects, else every image in the folder.
fn folder_references(dir: &Path, warnings: &mut Vec<String>, label: &str) -> Vec<Found> {
    let listed = ["references.json", "manifest.json"].iter().find_map(|n| read_json(&dir.join(n)));
    if let Some(list) = listed.as_ref().and_then(|v| v.get("references")).and_then(Value::as_array) {
        let mut out = Vec::new();
        for r in list {
            let Some(file) = r.get("ref").or_else(|| r.get("file")).and_then(Value::as_str) else { continue };
            let source = ["source_path", "tno_path", "source"].iter().find_map(|k| r.get(*k).and_then(Value::as_str)).map(str::to_string);
            let kind = ["texture_kind", "kind"].iter().find_map(|k| r.get(*k).and_then(Value::as_str)).and_then(|k| {
                let parsed = serde_json::from_value::<GuiTextureKind>(json!(k)).ok();
                if parsed.is_none() {
                    warnings.push(format!(
                        "{label}: '{k}' is not a texture kind (button, frame, panel, background); the file is kept for the whole family"
                    ));
                }
                parsed
            });
            out.push(Found { file: file.to_string(), source, kind });
        }
        return out;
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    names.retain(|n| is_image(n));
    names.sort();
    names.into_iter().map(|file| Found { file, source: None, kind: None }).collect()
}

impl Engine {
    /// The style pack a plan (or an import) uses: the named one, else the project default. `none` means no pack.
    pub(crate) fn choose_pack(&self, named: Option<&str>) -> EResult<Option<PackChoice>> {
        let (name, why) = match named.map(str::trim) {
            Some(NO_PACK) => return Ok(None),
            Some(n) => (n.to_string(), "explicit"),
            None => match self.config.defaults.style_pack.as_deref().map(str::trim) {
                Some(n) if n != NO_PACK && !n.is_empty() => (n.to_string(), "project_default"),
                _ => return Ok(None),
            },
        };
        let pack = pack::load(&self.fs, &name).map_err(|e| {
            let mut err = EngineError::from(e);
            if why == "project_default" {
                err.message = format!("{} (the project's [defaults] style_pack names it; pass style_pack: \"none\" to plan without it)", err.message);
            }
            err
        })?;
        let sha256 = pack::file_sha256(&self.fs, &name)?;
        Ok(Some(PackChoice { pack, sha256, why }))
    }

    /// The effective profile of a family with a pack applied. The pack must still have an entry for the family.
    pub(crate) fn profile_with_pack(&self, family: Family, pack_name: Option<&str>) -> EResult<Profile> {
        let base = self.family_profile(family)?.clone();
        let Some(name) = pack_name else { return Ok(base) };
        let p = pack::load(&self.fs, name)?;
        let entry = p.families.get(&family).ok_or_else(|| {
            EngineError::refused(
                "pack_entry_missing",
                format!("the style pack '{name}' has no entry for {family} any more, and this asset was made with it. Restore the entry or make a new plan"),
                RetryClass::RefreshPreconditions,
            )
        })?;
        Ok(pack::apply_profile(&base, name, entry))
    }

    /// The prompt rules and negatives an edit of an artifact restates: the style pack's form and negatives where the
    /// artifact was made with a pack that gives them, else the family's.
    pub(crate) fn artifact_prompt_rules(&self, a: &Artifact, profile: &Profile) -> (Vec<String>, Vec<String>) {
        let own = (profile.imagegen.prompt.rules.clone(), profile.imagegen.prompt.negative.clone());
        let Some(name) = a.profile.style_pack.as_deref() else { return own };
        match pack::load(&self.fs, name).ok().and_then(|p| p.resolve(a.family, self.artifact_texture_kind(a))) {
            Some(r) => pack::prompt_rules(profile, &r),
            None => own,
        }
    }

    /// The texture kind the plan of an artifact named, following edits back to the planned candidate.
    pub(crate) fn artifact_texture_kind(&self, a: &Artifact) -> Option<GuiTextureKind> {
        if a.family != Family::GuiTexture {
            return None;
        }
        let entries = self.ledger.entries().ok()?;
        let mut id = a.root_id.clone();
        for _ in 0..16 {
            let rec: CandidateRecord = entries
                .iter()
                .filter(|e| matches!(e.kind.as_str(), kind::GENERATED | kind::EDITED | kind::IMPORTED))
                .find(|e| e.artifact_id.as_deref() == Some(id.as_str()))
                .and_then(|e| serde_json::from_value(e.data.clone()).ok())?;
            if let Some(plan_id) = &rec.plan_id {
                return self.load_plan(plan_id).ok()?.request.texture_kind;
            }
            id = imagegen_core::naming::ArtifactId::parse(rec.parent.as_deref()?).ok()?.raw_id();
        }
        None
    }

    /// The images the review measures an artifact against, and a description of the set. An artifact made with a
    /// style pack is measured against the pack's references for its family (and texture kind), calibrated with the
    /// small-set rule (`pack_match_v2`); any other against the family's vanilla art.
    pub(crate) fn comparison_set(&self, a: &Artifact) -> (Result<Arc<VanillaSet>, String>, Value) {
        let Some(name) = a.profile.style_pack.clone() else {
            return (self.vanilla_set(a.family), json!({"kind": "vanilla", "rule": vanillamatch::VERSION}));
        };
        let kind = self.artifact_texture_kind(a);
        let mut described = json!({"kind": "style_pack", "pack": name, "rule": pack::MATCH_VERSION, "widening": pack::MATCH_WIDENING, "floor_factor": pack::MATCH_FLOOR_FACTOR, "shade_step": pack::SHADE_STEP, "shade_margin": pack::SHADE_MARGIN});
        let p = match pack::load(&self.fs, &name) {
            Ok(p) => p,
            Err(e) => return (Err(format!("the style pack '{name}' cannot be read: {e}")), described),
        };
        let Some(r) = p.resolve(a.family, kind) else {
            return (Err(format!("the style pack '{name}' has no entry for {}", a.family)), described);
        };
        described["entry"] = json!(r.entry);
        if let Err(e) = pack::verify_references(&self.fs, &name, &r.references) {
            return (Err(e.to_string()), described);
        }
        let mut images = Vec::new();
        let mut rows = Vec::new();
        for s in &r.references {
            let Ok(bytes) = self.fs.read(&s.path, refs::MAX_REFERENCE_BYTES) else { continue };
            let Ok((image, _)) = decode(&bytes, Some(&s.path), DecodeLimits::default()) else { continue };
            let Some(row) = pack::measure_reference(&image) else { continue };
            rows.push(row);
            images.push(VanillaImage { path: PathBuf::from(&s.path), frame: None, sha256: s.sha256.clone(), image, features: row.features });
        }
        described["files"] = json!(images.len());
        let unchecked = pack::unchecked(a.family);
        if !unchecked.is_empty() {
            described["unchecked"] = json!(unchecked);
        }
        match pack::calibrate(a.family, &rows) {
            Some(calibration) => (Ok(Arc::new(VanillaSet { images, calibration })), described),
            None => (
                Err(format!(
                    "the style pack '{name}' has {} measurable references for {}; the pack-match check needs at least {}",
                    images.len(),
                    r.entry,
                    vanillamatch::MIN_FILES
                )),
                described,
            ),
        }
    }

    /// The look of a style pack's references for kept-colours unit art: the glyph size, the glyph palette
    /// (`snap_palette`) and the edge glow per frame (`edge_glow`), measured on the family's references, with their
    /// hashes. `None` when the profile has no pack or no reference can be read. A changed or missing reference is
    /// refused, as at plan time.
    pub(crate) fn pack_finish(&self, profile: &Profile) -> EResult<Option<imagegen_core::chain::PackFinish>> {
        let Some(pp) = profile.pack.as_ref() else { return Ok(None) };
        let p = pack::load(&self.fs, &pp.name)?;
        let Some(r) = p.resolve(profile.family, None) else { return Ok(None) };
        pack::verify_references(&self.fs, &pp.name, &r.references)?;
        let mut images = Vec::new();
        let mut hashes = Vec::new();
        for s in &r.references {
            let bytes = self.fs.read(&s.path, refs::MAX_REFERENCE_BYTES)?;
            let Ok((image, _)) = decode(&bytes, Some(&s.path), DecodeLimits::default()) else { continue };
            images.push(image);
            hashes.push(s.sha256.clone());
        }
        if images.is_empty() {
            return Ok(None);
        }
        let look = imagegen_core::recipes::pack_look::measure(&images, profile.frames.max(1), profile.canvas[0]);
        Ok(Some(imagegen_core::chain::PackFinish { pack: pp.name.clone(), references: hashes, look }))
    }

    /// Pin every reference of a pack to the hash of its file now. Each must be an image inside the project.
    fn pin_pack(&self, p: &mut StylePack) -> EResult<()> {
        let pin = |r: &mut StyleRef| -> EResult<()> {
            refs::load_reference(&self.fs, None, &ReferenceInput { path: r.path.clone(), role: refs::RefRole::StyleAnchor })?;
            r.path = r.path.replace('\\', "/");
            r.sha256 = sha256_hex(&self.fs.read(&r.path, refs::MAX_REFERENCE_BYTES)?);
            Ok(())
        };
        for e in p.families.values_mut() {
            for r in &mut e.references {
                pin(r)?;
            }
            for k in e.kinds.values_mut() {
                for r in &mut k.references {
                    pin(r)?;
                }
            }
        }
        Ok(())
    }

    /// Save a pack (references pinned) and record it in the ledger.
    pub(crate) fn save_pack(&self, mut p: StylePack, replace: bool) -> EResult<Value> {
        p.validate()?;
        self.pin_pack(&mut p)?;
        if p.approved_by.is_some() && p.approved_on.is_none() {
            p.approved_on = Some(self.now());
        }
        let rel = pack::save(&self.fs, &p, replace)?;
        let sha = pack::file_sha256(&self.fs, &p.name)?;
        let families: Vec<&str> = p.families.keys().map(|f| f.as_str()).collect();
        self.ledger.append(
            kind::PACK,
            None,
            None,
            json!({"name": p.name, "path": rel, "sha256": sha, "families": families, "references": p.all_references().len(), "approved_by": p.approved_by}),
        )?;
        Ok(json!({"saved": rel, "sha256": sha, "pack": p}))
    }

    /// The pack actions of `hoi4.image_references`.
    pub(crate) fn pack_list(&self) -> EResult<Value> {
        let names = pack::list(&self.fs)?;
        Ok(json!({"packs": names, "default": self.config.defaults.style_pack, "folder": pack::PACK_DIR}))
    }

    /// Read one pack.
    pub(crate) fn pack_get(&self, name: &str) -> EResult<Value> {
        let p = pack::load(&self.fs, name)?;
        Ok(json!({"pack": p, "path": pack::pack_path(name)?, "sha256": pack::file_sha256(&self.fs, name)?}))
    }

    /// Check a saved pack: pinned hashes, switches, reference sizes against the canvas, and how well each family's
    /// references agree (the pack-match check of each reference against ranges fitted on the others).
    pub fn pack_validate(&self, name: &str) -> EResult<Value> {
        let p = pack::load(&self.fs, name)?;
        let mut problems: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut families = serde_json::Map::new();
        for (family, e) in &p.families {
            let base = self.family_profile(*family)?.clone();
            let profile = pack::apply_profile(&base, name, e);
            let mut sets: Vec<(String, Vec<StyleRef>)> = vec![(family.as_str().to_string(), e.references.clone())];
            for (k, x) in &e.kinds {
                if !x.references.is_empty() {
                    sets.push((format!("{family}.{k}"), x.references.clone()));
                }
            }
            let mut entries = Vec::new();
            for (entry, list) in sets {
                if let Err(err) = pack::verify_references(&self.fs, name, &list) {
                    problems.push(format!("{entry}: {err}"));
                }
                let mut sizes: BTreeMap<String, usize> = BTreeMap::new();
                let mut rows = Vec::new();
                for r in &list {
                    let Ok(bytes) = self.fs.read(&r.path, refs::MAX_REFERENCE_BYTES) else { continue };
                    match decode(&bytes, Some(&r.path), DecodeLimits::default()) {
                        Ok((img, _)) => {
                            *sizes.entry(format!("{}x{}", img.width, img.height)).or_default() += 1;
                            if let Some(f) = pack::measure_reference(&img) {
                                rows.push(f);
                            }
                        }
                        Err(err) => problems.push(format!("{entry}: {} is not a readable image: {err}", r.path)),
                    }
                }
                let canvas = if profile.needs_mod_canvas() {
                    None
                } else {
                    Some(format!("{}x{}", profile.canvas[0] * profile.frames.max(1), profile.canvas[1]))
                };
                let modal = sizes.iter().max_by_key(|(_, n)| **n).map(|(s, n)| (s.clone(), *n));
                let mut suggested = None;
                if let (Some((size, n)), Some(c)) = (&modal, &canvas) {
                    if size != c && *n * 4 >= list.len() * 3 && e.canvas.is_none() && profile.frames <= 1 {
                        suggested = Some(size.clone());
                        warnings.push(format!(
                            "{entry}: {n} of {} references are {size}, the family's canvas is {c}. If the mod's art really is {size}, set canvas in the pack",
                            list.len()
                        ));
                    }
                }
                let agreement = pack::leave_one_out(*family, &rows).map(|(pass, n)| json!({"pass": pass, "of": n}));
                if !list.is_empty() && rows.len() < vanillamatch::MIN_FILES {
                    warnings.push(format!(
                        "{entry}: {} measurable references; the review's pack-match check needs at least {}, so an approval would need an override",
                        rows.len(),
                        vanillamatch::MIN_FILES
                    ));
                }
                entries.push(json!({"entry": entry, "references": list.len(), "measured": rows.len(), "sizes": sizes, "family_canvas": canvas, "suggested_canvas": suggested, "self_check": agreement}));
            }
            let sw = e.processing;
            let needs_form = [
                (sw.keep_colours, "keep_colours", "the vanilla counter rules ask for a black silhouette that the tool recolours"),
                (sw.upright_print, "upright_print", "the vanilla report rules say the tool adds the tilt and frame"),
                (sw.fill_canvas, "fill_canvas", "the vanilla rules ask for a transparent background"),
            ];
            for (on, what, why) in needs_form {
                if on && e.form.is_empty() {
                    warnings.push(format!(
                        "{family}: {what} is on but the entry has no form override, and {why}. Add form lines that describe the mod's form"
                    ));
                }
            }
            if e.look.is_none() && e.kinds.values().all(|k| k.look.is_none()) {
                warnings.push(format!("{family}: no look line"));
            }
            families.insert(
                family.as_str().to_string(),
                json!({"look": e.look, "form_override": !e.form.is_empty(), "negative_override": !e.negative.is_empty(), "canvas": e.canvas, "processing": sw.names(), "kinds": e.kinds.keys().map(|k| k.as_str()).collect::<Vec<_>>(), "checks": profile.imagegen.checks, "recipe": imagegen_core::chain::recipe_of(&profile).id(), "sets": entries}),
            );
        }
        Ok(json!({"name": name, "ok": problems.is_empty(), "problems": problems, "warnings": warnings, "families": families}))
    }

    /// Build a pack from a folder laid out with one sub-folder per family id (`decision/`, `gui_texture/` and so
    /// on), each holding the reference images and optionally `references.json` (a `references` list of `{ref}`
    /// objects, with an optional `texture_kind` for GUI textures and a source path for the note) and `look.txt`.
    /// A `gallery-plan.json` at the top may give each family's look (`families.<family>.look`). With `top_family`,
    /// images listed by a top-level `manifest.json` or `references.json` belong to that family.
    ///
    /// The files are copied into the mod under `.hoi4-imagegen/packs/<name>/<family>/` and pinned. Replacing an
    /// existing pack keeps each entry's form, negatives, canvas and processing switches.
    pub fn import_pack_folder(&self, name: &str, from: &Path, top_family: Option<Family>, replace: bool) -> EResult<Value> {
        imagegen_core::naming::validate_snake("pack name", name)?;
        if name == NO_PACK {
            return Err(EngineError::invalid(format!("'{NO_PACK}' is reserved")));
        }
        if !from.is_dir() {
            return Err(EngineError::new("not_found", format!("{} is not a folder", from.display()), RetryClass::Terminal));
        }
        let existing = match pack::load(&self.fs, name) {
            Ok(p) if replace => Some(p),
            Ok(_) => {
                return Err(EngineError::new(
                    "exists",
                    format!("style pack '{name}' already exists; pass --replace to import over it (the old file is backed up)"),
                    RetryClass::Terminal,
                ))
            }
            Err(imagegen_core::Error::NotFound(_)) => None,
            Err(e) => return Err(e.into()),
        };
        let plan = read_json(&from.join("gallery-plan.json"));
        let plan_look = |f: Family| plan.as_ref().and_then(|v| v.get("families")?.get(f.as_str())?.get("look")?.as_str().map(str::to_string));
        let mut warnings: Vec<String> = Vec::new();
        let mut folders: Vec<(Family, PathBuf)> = Vec::new();
        let mut names: Vec<String> = std::fs::read_dir(from)
            .map_err(|e| EngineError::new("io", e.to_string(), RetryClass::Terminal))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        for n in names {
            match n.parse::<Family>() {
                Ok(f) => folders.push((f, from.join(&n))),
                Err(_) => warnings.push(format!("the folder {n} is not a family id and was skipped")),
            }
        }
        if let Some(f) = top_family {
            if folders.iter().any(|(g, _)| *g == f) {
                return Err(EngineError::invalid(format!("{f} has its own folder; leave out top_family")));
            }
            folders.push((f, from.to_path_buf()));
        }
        let mut p = existing.clone().unwrap_or(StylePack {
            schema: 1,
            name: name.to_string(),
            description: String::new(),
            approved_by: None,
            approved_on: None,
            families: BTreeMap::new(),
        });
        let mut report = serde_json::Map::new();
        let mut copied = 0usize;
        for (family, dir) in folders {
            let label = family.as_str().to_string();
            let found = folder_references(&dir, &mut warnings, &label);
            let look = first_line(&dir.join("look.txt"))
                .or_else(|| {
                    ["references.json", "manifest.json"].iter().find_map(|n| read_json(&dir.join(n))?.get("look")?.as_str().map(str::to_string))
                })
                .or_else(|| plan_look(family));
            let old = existing.as_ref().and_then(|x| x.families.get(&family)).cloned().unwrap_or_default();
            let mut entry = PackEntry {
                look: look.clone().or(old.look.clone()),
                form: old.form,
                negative: old.negative,
                canvas: old.canvas,
                processing: old.processing,
                references: Vec::new(),
                kinds: BTreeMap::new(),
            };
            for (k, x) in &old.kinds {
                entry.kinds.insert(*k, KindEntry { references: Vec::new(), ..x.clone() });
            }
            let mut taken = 0usize;
            for f in found {
                if !safe_file_name(&f.file) {
                    warnings.push(format!("{label}: '{}' is not a plain file name and was skipped", f.file));
                    continue;
                }
                let src = dir.join(&f.file);
                let bytes = match imagegen_core::fsio::read_limited(&src, refs::MAX_REFERENCE_BYTES) {
                    Ok(b) => b,
                    Err(e) => {
                        warnings.push(format!("{label}/{}: {e}", f.file));
                        continue;
                    }
                };
                if let Err(e) = decode(&bytes, Some(&f.file), DecodeLimits::default()) {
                    warnings.push(format!("{label}/{}: not a readable image ({e}); skipped", f.file));
                    continue;
                }
                let list_len = match f.kind {
                    Some(k) => entry.kinds.get(&k).map(|x| x.references.len()).unwrap_or(0),
                    None => entry.references.len(),
                };
                if list_len >= pack::MAX_REFS {
                    warnings.push(format!("{label}: more than {} references; {} was left out", pack::MAX_REFS, f.file));
                    continue;
                }
                let rel = format!("{}/{label}/{}", pack::pack_dir(name)?, f.file);
                if self.fs.exists(&rel) {
                    if self.fs.read(&rel, refs::MAX_REFERENCE_BYTES)? != bytes {
                        if !replace {
                            return Err(EngineError::new("exists", format!("{rel} exists with other content; pass --replace"), RetryClass::Terminal));
                        }
                        self.fs.replace(&rel, &bytes)?;
                    }
                } else {
                    self.fs.write_new(&rel, &bytes)?;
                }
                copied += 1;
                taken += 1;
                let note = match &f.source {
                    Some(s) => format!("imported with the pack from {label}/{}; source path in the mod it comes from: {s}", f.file),
                    None => format!("imported with the pack from {label}/{}", f.file),
                };
                let r = StyleRef { path: rel, sha256: sha256_hex(&bytes), note };
                match f.kind {
                    Some(k) => entry.kinds.entry(k).or_default().references.push(r),
                    None => entry.references.push(r),
                }
            }
            if taken == 0 && entry.look.is_none() {
                warnings.push(format!("{label}: no reference images and no look; skipped"));
                continue;
            }
            report.insert(label, json!({"references": taken, "kinds": entry.kinds.iter().map(|(k, x)| (k.as_str().to_string(), x.references.len())).collect::<BTreeMap<_, _>>(), "look": entry.look}));
            p.families.insert(family, entry);
        }
        if p.families.is_empty() {
            return Err(EngineError::invalid(format!(
                "{} holds no family folders with references (folders named by family id, such as decision/)",
                from.display()
            )));
        }
        let saved = self.save_pack(p, existing.is_some())?;
        let check = self.pack_validate(name)?;
        Ok(json!({
            "imported": name,
            "files_copied": copied,
            "families": report,
            "saved": saved["saved"],
            "sha256": saved["sha256"],
            "warnings": warnings,
            "validation": check,
            "next": "Read the validation: set canvas, form lines and processing switches where the mod's art differs from vanilla (docs/how-to.md, making a style pack), save with hoi4.image_references pack_save or edit pack.toml, then run pack validate. Plan with style_pack, or set [defaults] style_pack in .hoi4-imagegen/config.toml."
        }))
    }
}
