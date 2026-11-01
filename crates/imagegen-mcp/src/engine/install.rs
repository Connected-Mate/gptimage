//! `hoi4.image_install`: the only operation that writes game files.
//!
//! It refuses unless validation passed, a review approved, and every recorded hash still matches (the exact
//! file, its raw candidate and its plan). It never overwrites a file it did not install, writes all of an
//! asset's files or none, and leaves a provenance record next to the ledger.

use std::collections::HashMap;

use imagegen_core::error::RetryClass;
use imagegen_core::export::{self, FinalFile};
use imagegen_core::fsio::PendingWrite;
use imagegen_core::gate::{self, HashCheck};
use imagegen_core::hashing::sha256_hex;
use imagegen_core::pixels::{decode, DecodeLimits, Rgba};
use imagegen_core::plan::Plan;
use imagegen_core::records::{kind, CandidateRecord, InstalledFile, InstalledRecord, ReplacedFile, ValidatedRecord};
use imagegen_core::sprite;
use serde_json::{json, Value};

use crate::api::{InstallMode, InstallParams, OnExists};

use super::{EResult, Engine, EngineError};

impl Engine {
    /// Install an approved artifact.
    pub fn install(&self, p: InstallParams) -> EResult<Value> {
        let artifact = self.artifact(&p.target)?;
        let profile = self.profile_of(&artifact)?.clone();
        let (bytes, sha) = self.read_artifact(&artifact)?;
        let entries = self.ledger.entries()?;

        // ---- the gate: validation, review, and every hash along the lineage
        let mut checks: Vec<HashCheck> = Vec::new();
        let current_profile_hash = self.profile_ref(&profile).hash;
        if let Some(v) = entries
            .iter()
            .filter(|e| e.kind == kind::VALIDATED && e.artifact_id.as_deref() == Some(artifact.id.as_str()))
            .filter_map(|e| serde_json::from_value::<ValidatedRecord>(e.data.clone()).ok())
            .last()
        {
            checks.push(HashCheck {
                what: "profile the validation used".into(),
                code: "install_gate_validation_stale".into(),
                recorded: v.profile.hash,
                found: Some(current_profile_hash),
            });
        }
        let root: Option<CandidateRecord> = entries
            .iter()
            .filter(|e| matches!(e.kind.as_str(), kind::GENERATED | kind::EDITED | kind::IMPORTED))
            .find(|e| e.artifact_id.as_deref() == Some(artifact.root_id.as_str()))
            .and_then(|e| serde_json::from_value(e.data.clone()).ok());
        if let Some(root) = &root {
            checks.push(HashCheck {
                what: format!("raw candidate {}", root.candidate_id),
                code: "install_gate_hash_changed".into(),
                recorded: root.sha256.clone(),
                found: self.fs.read(&root.path, 64 * 1024 * 1024).ok().map(|b| sha256_hex(&b)),
            });
            if let (Some(plan_id), Some(plan_hash)) = (&root.plan_id, &root.plan_hash) {
                let found = self
                    .fs
                    .read(&imagegen_core::plan::plan_path(plan_id), 4 * 1024 * 1024)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Plan>(&b).ok())
                    .map(|pl| pl.compute_hash());
                checks.push(HashCheck {
                    what: format!("plan {plan_id}"),
                    code: "install_gate_plan_changed".into(),
                    recorded: plan_hash.clone(),
                    found,
                });
            }
        }
        let report = gate::evaluate(&entries, &artifact.id, &sha, &p.approval_id, &checks, &self.gate_expect(artifact.family)?);
        if !report.passed {
            let details = json!({"failures": report.failures});
            let first = report.failures[0].code.clone();
            let all: Vec<String> = report.failures.iter().map(|f| format!("{}: {}", f.code, f.message)).collect();
            let mut err = EngineError::refused(&first, format!("install is refused. {}", all.join(" | ")), RetryClass::RefreshPreconditions);
            err.details = Some(details);
            return Err(err);
        }

        // ---- what will be written
        let (img, _) = decode(&bytes, Some(&artifact.path), DecodeLimits::default())?;
        imagegen_core::reuse::ensure_no_cross_family_reuse(
            &self.fs,
            &self.profiles,
            artifact.family,
            &imagegen_core::reuse::pixel_hash(&img),
            &artifact.id,
        )?;
        // the images each final file is made from: the main art, and the companions (achievement states, flag sizes)
        let mut companion_imgs: HashMap<String, Rgba> = HashMap::new();
        for (c, data) in self.read_companions(&artifact)? {
            let (ci, _) = decode(&data, Some(&c.path), DecodeLimits::default())?;
            companion_imgs.insert(c.key.clone(), ci);
        }
        let files = export::final_files(&profile, artifact.canvas, &artifact.feature_slug, &artifact.asset)?;
        let mut encoded: Vec<(FinalFile, Vec<u8>)> = Vec::new();
        let mut written: Vec<InstalledFile> = Vec::new();
        for f in &files {
            let source = match f.variant.as_deref() {
                None | Some("normal") => &img,
                Some(key) => companion_imgs.get(key).ok_or_else(|| {
                    EngineError::refused(
                        "companion_missing",
                        format!(
                            "{} needs the {key} image of {}, and the processed record has none. Process the source candidate again",
                            f.rel, artifact.id
                        ),
                        RetryClass::RefreshPreconditions,
                    )
                })?,
            };
            let data = export::export_bytes(source, f, [source.width, source.height])?;
            let back = export::decode_exported(&data, f.container)?;
            if back.pixels_sha256() != source.pixels_sha256() {
                return Err(EngineError::new(
                    "export_round_trip_failed",
                    format!("{} did not decode to the same pixels it was made from; nothing was written", f.rel),
                    RetryClass::Terminal,
                ));
            }
            written.push(InstalledFile {
                path: f.rel.clone(),
                sha256: sha256_hex(&data),
                pixels_sha256: back.pixels_sha256(),
                container: f.container.ext().to_string(),
            });
            encoded.push((f.clone(), data));
        }

        // ---- existing files: only ones this tool installed, and only on request
        let mut owned: HashMap<String, String> = HashMap::new();
        for e in entries.iter().filter(|e| e.kind == kind::INSTALLED) {
            if let Ok(rec) = serde_json::from_value::<InstalledRecord>(e.data.clone()) {
                for f in rec.files {
                    owned.insert(f.path, f.sha256);
                }
            }
        }
        let mut replaced: Vec<ReplacedFile> = Vec::new();
        let mut conflicts = Vec::new();
        for (f, _) in &encoded {
            if !self.fs.exists(&f.rel) {
                continue;
            }
            let current = sha256_hex(&self.fs.read(&f.rel, 64 * 1024 * 1024)?);
            match p.on_exists {
                OnExists::Fail => conflicts.push(f.rel.clone()),
                OnExists::Replace => {
                    if owned.get(&f.rel) == Some(&current) {
                        replaced.push(ReplacedFile { path: f.rel.clone(), previous_sha256: current });
                    } else {
                        return Err(EngineError::refused(
                            "final_path_not_owned",
                            format!("{} exists and was not installed by this tool (or was edited since). It is never replaced; choose another asset name or move the file yourself", f.rel),
                            RetryClass::Terminal,
                        ));
                    }
                }
            }
        }
        if !conflicts.is_empty() {
            return Err(EngineError::new(
                "final_path_exists",
                format!(
                    "these files already exist: {}. Choose another asset name, or pass on_exists = replace for files this tool installed earlier",
                    conflicts.join(", ")
                ),
                RetryClass::RefreshPreconditions,
            ));
        }

        // ---- sprites
        let sprites = export::sprite_entries(&profile, &artifact.feature_slug, &artifact.asset, &files)?;
        // the mod's own trace: a sprite name or texture the mod already uses for another family is refused (GFX-FAMILY-REUSE)
        let allowlist = self.audit_allowlist(None)?;
        let new_names: Vec<String> = sprites.as_ref().map(|(_, e)| e.iter().map(|x| x.name.clone()).collect()).unwrap_or_default();
        let new_textures: Vec<String> = files.iter().map(|f| f.rel.clone()).collect();
        let trace_guard =
            imagegen_core::audit::ensure_no_trace_reuse(self.fs.root(), allowlist.as_deref(), artifact.family, &new_names, &new_textures)?;
        // every file of the install goes into one batch: the images, and the `.gfx` text or the handoff note
        let mut batch: Vec<PendingWrite> = encoded
            .iter()
            .map(|(f, data)| PendingWrite { rel: f.rel.clone(), bytes: data.clone(), replace: replaced.iter().any(|r| r.path == f.rel) })
            .collect();
        let mut sprite_names = Vec::new();
        let mut gfx_file_out = None;
        if let Some((gfx_file, entries_)) = &sprites {
            sprite_names = entries_.iter().map(|e| e.name.clone()).collect();
            gfx_file_out = Some(gfx_file.clone());
            if p.mode == InstallMode::Register {
                // the file is edited as bytes with the lossless parser: UTF-8, UTF-8 with a BOM and Windows-1252 all keep their encoding
                let existing = if self.fs.exists(gfx_file) { Some(self.fs.read(gfx_file, 8 * 1024 * 1024)?) } else { None };
                let new_bytes = sprite::register_bytes(existing.as_deref(), entries_)?;
                if let Some(prev) = &existing {
                    replaced.push(ReplacedFile { path: gfx_file.clone(), previous_sha256: sha256_hex(prev) });
                }
                batch.push(PendingWrite { rel: gfx_file.clone(), bytes: new_bytes, replace: existing.is_some() });
            } else {
                let handoff = format!("docs/assets/{}/gfx_handoff.md", artifact.feature_slug);
                let mut notes = vec![format!("approved under {} (sha256 {})", p.approval_id, &sha[..12])];
                if profile.differs_from_vanilla() {
                    notes.push("the canvas differs from the vanilla-measured default; see hoi4.image_families".into());
                }
                let block = sprite::handoff_block(
                    &artifact.asset_id,
                    &written.iter().map(|w| w.path.clone()).collect::<Vec<_>>(),
                    gfx_file,
                    entries_,
                    &notes,
                );
                let handoff_exists = self.fs.exists(&handoff);
                let existing = if handoff_exists {
                    String::from_utf8_lossy(&self.fs.read(&handoff, 4 * 1024 * 1024)?).to_string()
                } else {
                    String::from("# GFX handoff\n\nSprite definitions for the main agent to add. Asset workers do not edit .gfx files.\n\n")
                };
                batch.push(PendingWrite { rel: handoff, bytes: format!("{existing}{block}").into_bytes(), replace: handoff_exists });
            }
        }
        // a family without a sprite (achievements, flags) is loaded by the game from its file name, so both modes
        // install the files alone

        // ---- write everything or nothing: temp files first, then all renames, undone together if anything fails
        let committed = self.fs.write_batch(&batch).map_err(|e| {
            EngineError::new("install_write_failed", format!("{e}. Nothing was left written by this call"), RetryClass::SafeSameInputs)
        })?;

        let record = InstalledRecord {
            target: artifact.id.clone(),
            family: artifact.family,
            mode: match p.mode {
                InstallMode::Handoff => "handoff".into(),
                InstallMode::Register => "register".into(),
            },
            files: written.clone(),
            sprites: sprite_names.clone(),
            gfx_file: gfx_file_out.clone(),
            replaced: replaced.clone(),
            approval_id: p.approval_id.clone(),
        };
        if let Err(e) = self.ledger.append(kind::INSTALLED, Some(&artifact.asset_id), Some(&artifact.id), &record) {
            committed.rollback();
            return Err(EngineError::from(e));
        }
        committed.finish();
        let record_path = self.write_asset_record(&artifact.asset_id, &artifact.family.to_string(), &record)?;

        let snippet = sprites.as_ref().map(|(_, e)| sprite::new_file(e));
        Ok(json!({
            "target": artifact.id,
            "final_files": written,
            "pixels_sha256": img.pixels_sha256(),
            "sprite": {"names": sprite_names, "gfx_file": gfx_file_out, "snippet": snippet, "mode": record.mode,
                       "caveats": sprites.as_ref().map(|(_, e)| e.iter().map(|x| x.caveat.clone()).filter(|c| !c.is_empty()).collect::<Vec<_>>()).unwrap_or_default()},
            "replaced": replaced,
            "reuse_trace": match &trace_guard {
                imagegen_core::audit::TraceGuard::Checked => json!("checked"),
                imagegen_core::audit::TraceGuard::Skipped(why) => json!(format!("skipped: {why}")),
            },
            "record_path": record_path,
            "next": "Wire the sprite name where the game uses it (the handoff note has the snippet), then verify in game. In-game behaviour is user-operated evidence this tool cannot produce."
        }))
    }

    /// The per-asset provenance record, a readable view of the ledger entries for the asset.
    fn write_asset_record(&self, asset_id: &str, family: &str, installed: &InstalledRecord) -> EResult<String> {
        let rel = format!(".hoi4-imagegen/records/{asset_id}.json");
        let entries = self.ledger.for_asset(asset_id)?;
        let body = json!({
            "schema": 1,
            "asset_id": asset_id,
            "family": family,
            "generated_at": self.now(),
            "last_install": installed,
            "ledger_entries": entries,
            "note": "A view of .hoi4-imagegen/ledger.jsonl for this asset; the ledger is the source of truth and carries the hash chain."
        });
        let mut bytes = serde_json::to_vec_pretty(&body).map_err(|e| EngineError::new("internal", e.to_string(), RetryClass::Terminal))?;
        bytes.push(b'\n');
        self.fs.replace(&rel, &bytes)?;
        Ok(rel)
    }
}

#[allow(dead_code)]
fn _value(_: Value) {}
