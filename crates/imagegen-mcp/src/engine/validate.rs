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
use imagegen_core::validate::{validate_container, validate_image, Finding, Level, Options, Status};
use serde_json::{json, Value};

use crate::api::ValidateParams;

use super::{looks_like_artifact, EResult, Engine, EngineError};

const MAX_FOLDER_FILES: usize = 500;

impl Engine {
    /// Validate each target.
    pub fn validate(&self, p: ValidateParams) -> EResult<Value> {
        if p.targets.is_empty() || p.targets.len() > 200 {
            return Err(EngineError::invalid("give 1 to 200 targets"));
        }
        let mut results = Vec::new();
        for target in &p.targets {
            if target.ends_with('/') || target.ends_with('\\') {
                for rel in self.list_folder(target)? {
                    results.push(self.validate_path(&rel, &p)?);
                }
            } else if self.fs.exists(target) && !looks_like_artifact(target) {
                results.push(self.validate_path(target, &p)?);
            } else if looks_like_artifact(target) {
                results.push(self.validate_artifact(target, &p)?);
            } else {
                return Err(EngineError::new("not_found", format!("'{target}' is neither a file in the project nor an artifact id"), RetryClass::Terminal));
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

    fn profile_for_path(&self, p: &ValidateParams) -> EResult<&Profile> {
        let id = p.profile.as_deref().ok_or_else(|| EngineError::invalid("validating a file needs profile (a profile id or category such as focus_icon)"))?;
        self.profiles
            .get(id)
            .or_else(|| id.parse().ok().and_then(|c| self.profiles.for_category(c)))
            .ok_or_else(|| EngineError::new("not_found", format!("no profile '{id}'"), RetryClass::Terminal))
    }

    fn validate_path(&self, rel: &str, p: &ValidateParams) -> EResult<Value> {
        let profile = self.profile_for_path(p)?;
        let bytes = self.fs.read(rel, 64 * 1024 * 1024)?;
        let checks = p.checks.clone().unwrap_or_else(|| profile.imagegen.checks.clone());
        let canvas = if profile.canvas == [0, 0] { p.canvas } else { None };
        let (status, mut findings, extra) = run_checks(&bytes, rel, profile, &checks, canvas);
        // installed art should be in the ledger
        let installed_by_us = self.ledger.entries()?.iter().any(|e| e.kind == kind::INSTALLED && e.data.get("files").and_then(Value::as_array).map(|f| f.iter().any(|x| x.get("path").and_then(Value::as_str) == Some(rel))).unwrap_or(false));
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

    fn validate_artifact(&self, id: &str, p: &ValidateParams) -> EResult<Value> {
        ArtifactId::parse(id)?;
        let artifact = self.artifact(id)?;
        let profile = self.profile(&artifact.profile.id)?;
        let (bytes, sha) = self.read_artifact(&artifact)?;
        let checks = p.checks.clone().unwrap_or_else(|| profile.imagegen.checks.clone());
        let canvas = if profile.canvas == [0, 0] { Some(artifact.canvas) } else { None };
        let (status, findings, extra) = run_checks(&bytes, &artifact.path, profile, &checks, canvas);
        let mut notes = extra;
        if sha != artifact.recorded_sha256 {
            notes["file_changed_since_recorded"] = json!(true);
        }
        if !artifact.processed {
            notes["note"] = json!("This is a raw candidate. It is expected to fail the size check until hoi4.image_process has fitted it to the canvas.");
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
fn run_checks(bytes: &[u8], name: &str, profile: &Profile, checks: &[String], canvas: Option<[u32; 2]>) -> (Status, Vec<Finding>, Value) {
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
fn result_json(target: &str, path: &str, sha: &str, status: Status, findings: &[Finding], checks: &[String], profile: &Profile, extra: &Value) -> Value {
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
