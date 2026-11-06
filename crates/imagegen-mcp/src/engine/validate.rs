//! `hoi4.image_validate`: deterministic checks on artifacts, files and folders.
//!
//! The report keeps `deterministic` results apart from the visual review. A passing report says the geometry,
//! alpha, margins and file layout are right. It never says the art is good.

use imagegen_core::error::RetryClass;
use imagegen_core::hashing::sha256_hex;
use imagegen_core::naming::ArtifactId;
use imagegen_core::pixels::{decode, DecodeLimits};
use imagegen_core::profile::Profile;
use imagegen_core::records::{kind, ValidatedRecord};
use imagegen_core::validate::{check_set_hash, style_check, validate_container, validate_image, Finding, Level, Options, Status, StyleModel};
use std::collections::HashMap;

use imagegen_core::family::Family;
use serde_json::{json, Value};

use crate::api::ValidateParams;

use super::diversity::Subject;
use super::{looks_like_artifact, EResult, Engine, EngineError};

const MAX_FOLDER_FILES: usize = 500;

impl Engine {
    /// Validate each target.
    pub fn validate(&self, p: ValidateParams) -> EResult<Value> {
        if p.targets.is_empty() || p.targets.len() > 200 {
            return Err(EngineError::invalid("give 1 to 200 targets"));
        }
        let mut results = Vec::new();
        let mut styles = StyleModels::default();
        for target in &p.targets {
            if target.ends_with('/') || target.ends_with('\\') {
                for rel in self.list_folder(target)? {
                    results.push(self.validate_path(&rel, &p, &mut styles)?);
                }
            } else if self.fs.exists(target) && !looks_like_artifact(target) {
                results.push(self.validate_path(target, &p, &mut styles)?);
            } else if looks_like_artifact(target) {
                results.push(self.validate_artifact(target, &p, &mut styles)?);
            } else {
                return Err(EngineError::new(
                    "not_found",
                    format!("'{target}' is neither a file in the project nor an artifact id"),
                    RetryClass::Terminal,
                ));
            }
        }
        let failed = results.iter().filter(|r| r["status"] == "fail").count();
        Ok(json!({
            "status": if failed == 0 { "pass" } else { "fail" },
            "checked": results.len(),
            "failed": failed,
            "deterministic_only": "A pass here means geometry, alpha, margins and file layout are right. Composition, style and readability at game size go to hoi4.image_review.",
            "results": results,
        }))
    }

    fn list_folder(&self, dir: &str) -> EResult<Vec<String>> {
        let dir = dir.trim_end_matches(['/', '\\']);
        let mut names = self.fs.list_names(dir)?;
        names.sort();
        let files: Vec<String> = names
            .into_iter()
            .filter(|n| {
                let l = n.to_ascii_lowercase();
                l.ends_with(".dds") || l.ends_with(".tga") || l.ends_with(".png")
            })
            .map(|n| format!("{dir}/{n}"))
            .collect();
        if files.len() > MAX_FOLDER_FILES {
            return Err(EngineError::invalid(format!("{dir} holds {} images; validate at most {MAX_FOLDER_FILES} at a time", files.len())));
        }
        Ok(files)
    }

    fn family_for_path(&self, p: &ValidateParams) -> EResult<&Profile> {
        let family = p.family.ok_or_else(|| EngineError::invalid("validating a file needs family (for example focus); artifacts use their own"))?;
        self.family_profile(family)
    }

    fn validate_path(&self, rel: &str, p: &ValidateParams, styles: &mut StyleModels) -> EResult<Value> {
        let profile = self.family_for_path(p)?;
        let bytes = self.fs.read(rel, 64 * 1024 * 1024)?;
        let checks = profile.imagegen.checks.clone();
        let canvas = p.canvas.filter(|c| profile.free_canvas() || profile.canvas_choices().contains(c));
        let style = styles.get(self, profile, &checks);
        let (status, mut findings, extra) = run_checks(&bytes, rel, profile, &checks, canvas, style);
        // installed art should be in the ledger
        let installed_by_us = self.ledger.entries()?.iter().any(|e| {
            e.kind == kind::INSTALLED
                && e.data
                    .get("files")
                    .and_then(Value::as_array)
                    .map(|f| f.iter().any(|x| x.get("path").and_then(Value::as_str) == Some(rel)))
                    .unwrap_or(false)
        });
        if !installed_by_us && rel.starts_with("gfx/") {
            findings.push(Finding {
                code: "GFX-PROVENANCE-MISSING".into(),
                level: Level::Advisory,
                message: "this file has no install record in the provenance ledger (it may be hand-made, made by another tool, or installed before the ledger existed)".into(),
                frame: None,
                evidence: json!({}),
            });
        }
        Ok(result_json(rel, rel, &sha256_hex(&bytes), status, &findings, &checks, profile, &extra))
    }

    fn validate_artifact(&self, id: &str, p: &ValidateParams, styles: &mut StyleModels) -> EResult<Value> {
        ArtifactId::parse(id)?;
        let artifact = self.artifact(id)?;
        if let Some(f) = p.family {
            if f != artifact.family {
                return Err(EngineError::refused(
                    "GFX-FAMILY-MISMATCH",
                    format!("{id} is a {} and can only be validated as a {}, not as a {f}", artifact.family, artifact.family),
                    RetryClass::Terminal,
                ));
            }
        }
        let profile = &self.profile_of(&artifact)?;
        let (bytes, sha) = self.read_artifact(&artifact)?;
        // the full check set of the family always runs: there is no way to ask for fewer
        let checks = profile.imagegen.checks.clone();
        let canvas = profile.canvas_overridden(artifact.canvas).then_some(artifact.canvas);
        let style = styles.get(self, profile, &checks);
        let near_bits = match style {
            Some(model) => model.and_then(|m| m.near_duplicate_bits),
            None => self.style_model(profile).and_then(|m| m.near_duplicate_bits),
        };
        let (_, mut findings, extra) = run_checks(&bytes, &artifact.path, profile, &checks, canvas, style);
        let mut notes = extra;
        // a frame and composition that nearly repeat an earlier output of the family warn, never fail
        if let Ok((img, _)) = decode(&bytes, Some(&artifact.path), DecodeLimits::default()) {
            let subject =
                Subject { family: artifact.family, asset_id: &artifact.asset_id, processed: artifact.processed, root_id: &artifact.root_id };
            findings.extend(self.near_duplicate_findings(&subject, &img, near_bits));
        }
        // the family's processing must be on record: a processed artifact whose chain lacks a required stage fails
        if let Some(rec) = &artifact.record {
            for problem in imagegen_core::chain::audit(rec, profile) {
                findings.push(Finding {
                    code: "GFX-CHAIN-INCOMPLETE".into(),
                    level: Level::Error,
                    message: problem,
                    frame: None,
                    evidence: json!({"recipe": rec.recipe.as_ref().map(|r| json!({"id": r.id, "version": r.version}))}),
                });
            }
            // every companion image (achievement states, flag sizes) is checked as the family, at its own size; a
            // companion the game looks up (small emblem, text icon, miniature, small strip) at its own frames and margin
            for c in &rec.companions {
                let data = self.fs.read(&c.path, 64 * 1024 * 1024)?;
                let (cprofile, csize) = match imagegen_core::lookup::derived_companion(profile.family, &c.key) {
                    Some(d) => {
                        let mut p2 = profile.clone();
                        p2.frames = d.frames;
                        p2.margin = d.margin;
                        p2.canvas = d.frame;
                        p2.accepted = vec![d.frame];
                        (p2, d.frame)
                    }
                    None => (profile.clone(), c.size),
                };
                // the style model describes the main image only: companion states (grey, smaller sizes) skip it
                let cchecks: Vec<String> =
                    checks.iter().filter(|k| !imagegen_core::validate::MAIN_IMAGE_ONLY_CHECKS.contains(&k.as_str())).cloned().collect();
                let (_, more, _) = run_checks(&data, &c.path, &cprofile, &cchecks, Some(csize), None);
                findings.extend(more.into_iter().map(|mut f| {
                    f.message = format!("[{}] {}", c.key, f.message);
                    f
                }));
            }
            notes["companions"] = json!(rec.companions.iter().map(|c| json!({"key": c.key, "path": c.path, "size": c.size})).collect::<Vec<_>>());
        }
        let status = if findings.iter().any(|f| f.level == Level::Error) { Status::Fail } else { Status::Pass };
        if sha != artifact.recorded_sha256 {
            notes["file_changed_since_recorded"] = json!(true);
        }
        if !artifact.processed {
            notes["note"] =
                json!("This is a raw candidate. It is expected to fail the size check until hoi4.image_process has fitted it to the canvas.");
        }
        if self.profile_ref(profile).hash != artifact.profile.hash {
            notes["profile_changed_since_made"] = json!(true);
        }
        let errors = findings.iter().filter(|f| f.level == Level::Error).count() as u32;
        let warnings = findings.iter().filter(|f| f.level == Level::Warning).count() as u32;
        self.ledger.append(
            kind::VALIDATED,
            Some(&artifact.asset_id),
            Some(&artifact.id),
            ValidatedRecord {
                target: artifact.id.clone(),
                family: artifact.family,
                check_set_hash: check_set_hash(&checks),
                sha256: sha.clone(),
                status,
                errors,
                warnings,
                findings: findings.clone(),
                checks: checks.clone(),
                profile: self.profile_ref(profile),
            },
        )?;
        Ok(result_json(&artifact.id, &artifact.path, &sha, status, &findings, &checks, profile, &notes))
    }
}

/// Decode and check one file's bytes.
fn run_checks(
    bytes: &[u8],
    name: &str,
    profile: &Profile,
    checks: &[String],
    canvas: Option<[u32; 2]>,
    style: Option<Option<&StyleModel>>,
) -> (Status, Vec<Finding>, Value) {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut findings = validate_container(bytes, file_name, profile, checks);
    let mut extra = json!({});
    let blocked = findings.iter().any(|f| f.code == "GFX-LFS-POINTER" || f.code == "GFX-TRUNCATED");
    if !blocked {
        match decode(bytes, Some(file_name), DecodeLimits::default()) {
            Ok((img, _)) => {
                let report = validate_image(&img, profile, checks, &Options { canvas });
                extra["size"] = json!(report.size);
                extra["pixels_sha256"] = json!(report.pixels_sha256);
                extra["frames"] = json!(report.frames);
                findings.extend(report.findings);
                // the style check compares with the family's own style distribution; it can warn, never fail
                match style {
                    Some(Some(model)) => {
                        let f = style_check(&img, model);
                        extra["style"] = match &f {
                            Some(f) => {
                                json!({"available": true, "tier": model.tier, "tier_name": model.tier_name, "verdict": f.code, "numbers": f.evidence})
                            }
                            None => json!({"available": true, "verdict": "image has no visible pixel"}),
                        };
                        findings.extend(f);
                    }
                    Some(None) => {
                        extra["style"] = json!({"available": false, "reason": "no source tier (pinned style, the mod's shelf copy, the mod's own assets, vanilla) has 3 usable images for this family"});
                    }
                    None => {}
                }
            }
            Err(e) => findings.push(Finding {
                code: "GFX-UNREADABLE".into(),
                level: Level::Error,
                message: format!("the image cannot be decoded: {e}"),
                frame: None,
                evidence: json!({}),
            }),
        }
    }
    let status = if findings.iter().any(|f| f.level == Level::Error) { Status::Fail } else { Status::Pass };
    (status, findings, extra)
}

#[allow(clippy::too_many_arguments)]
fn result_json(
    target: &str,
    path: &str,
    sha: &str,
    status: Status,
    findings: &[Finding],
    checks: &[String],
    profile: &Profile,
    extra: &Value,
) -> Value {
    let errors = findings.iter().filter(|f| f.level == Level::Error).count();
    let warnings = findings.iter().filter(|f| f.level == Level::Warning).count();
    let mut v = json!({
        "target": target,
        "path": path,
        "sha256": sha,
        "status": status,
        "errors": errors,
        "warnings": warnings,
        "findings": findings,
        "checks": checks,
        "family": profile.family,
        "group": profile.family.group(),
        "check_set_hash": check_set_hash(checks),
        "profile": {"id": profile.id, "canvas": profile.canvas, "frames": profile.frames, "rubric": profile.imagegen.rubric},
        "visual_review": "pending: run hoi4.image_preview and hoi4.image_review",
    });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in e {
            o.insert(k.clone(), val.clone());
        }
    }
    v
}

/// Style models per family for one validation call: each family's sources are scanned once.
#[derive(Default)]
struct StyleModels {
    by_family: HashMap<(Family, Option<String>), Option<StyleModel>>,
}

impl StyleModels {
    /// `None` when the family's check set has no `style` check; `Some(None)` when no tier could supply a model.
    fn get(&mut self, engine: &Engine, profile: &Profile, checks: &[String]) -> Option<Option<&StyleModel>> {
        if !checks.iter().any(|c| c == "style") {
            return None;
        }
        let key = (profile.family, profile.pack.as_ref().map(|p| p.name.clone()));
        Some(self.by_family.entry(key).or_insert_with(|| engine.style_model(profile)).as_ref())
    }
}
