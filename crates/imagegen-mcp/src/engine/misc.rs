//! Status, profiles, references and ledger queries: the read-mostly tools.

use imagegen_core::calibrate::calibrate;
use imagegen_core::config::game_version;
use imagegen_core::export;
use imagegen_core::fsio::display_path;
use imagegen_core::hashing::sha256_hex;
use imagegen_core::records::kind;
use imagegen_core::refs::{self, ReferenceInput};
use imagegen_core::rubric;
use imagegen_core::style;
use imagegen_providers::registry::key_env_names;
use imagegen_providers::types::ModelStatus;
use serde_json::{json, Value};

use crate::api::{FamiliesParams, LedgerParams, ReferencesAction, ReferencesParams};

use super::{EResult, Engine, EngineError};

impl Engine {
    /// `hoi4.image_status`
    pub fn status(&self) -> Value {
        let mut providers = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        for id in self.registry.ids() {
            let Some(p) = self.registry.get(&id) else { continue };
            let desc = p.descriptor();
            let cfg = self.config.provider(&id);
            let configured_model = cfg.model.clone();
            if let Some(m) = &configured_model {
                match desc.model(m).map(|x| &x.status) {
                    Some(ModelStatus::Retired { on }) => warnings.push(format!("providers.{id}.model is {m}, which was shut down on {on}")),
                    Some(ModelStatus::Deprecated { shutdown }) => {
                        warnings.push(format!("providers.{id}.model is {m}, which shuts down on {shutdown}"))
                    }
                    None if id != "comfyui" => warnings.push(format!("providers.{id}.model '{m}' is not in the reviewed model table")),
                    _ => {}
                }
            }
            providers.push(json!({
                "id": id,
                "configured": p.is_configured(),
                "key_env": key_env_names(&id, &cfg),
                "default_model": p.default_model(false),
                "default_edit_model": p.default_model(true),
                "models": desc.models.iter().map(|m| json!({"id": m.id, "status": m.status, "recommended": m.recommended, "alpha": m.alpha, "max_refs": m.max_refs})).collect::<Vec<_>>(),
                "tables_retrieved": desc.retrieved,
                "doc_url": desc.doc_url,
                "terms_note": desc.terms_note,
            }));
        }
        let game = self.game_dir();
        let ledger = self.ledger.verify().ok();
        let kit_path = self.config.kit_profiles_file(self.fs.root());
        json!({
            "server": {"name": "hoi4-imagegen-mcp", "version": env!("CARGO_PKG_VERSION"), "mode": format!("{:?}", self.mode)},
            "project_root": display_path(self.fs.root()),
            "config_file_present": self.fs.exists(imagegen_core::config::CONFIG_PATH),
            "providers": providers,
            "provider_problems": self.registry.problems(),
            "warnings": warnings,
            "game": {"found": game.is_some(), "dir": game.as_ref().map(|g| display_path(g)), "version": game.as_deref().and_then(game_version)},
            "profiles": {
                "count": self.profiles.all().count(),
                "effective_hash": self.profiles.effective_hash(),
                "kit_file": display_path(&kit_path),
                "kit_file_present": self.profiles.kit_path.is_some(),
                "kit_sha256": self.profiles.kit_sha256,
                "notes": self.profiles.notes,
            },
            "ledger": {"exists": self.ledger.exists(), "entries": ledger.as_ref().map(|l| l.entries), "chain_ok": ledger.as_ref().map(|l| l.ok), "first_bad": ledger.and_then(|l| l.first_bad)},
            "policy": {
                "allow_vanilla_references": self.config.policy.allow_vanilla_references,
                "forbidden_families": self.config.policy.forbidden_families,
                "max_usd_per_call": self.config.policy.max_usd_per_call,
            },
            "chatgpt_login": self.plugin,
            "jobs_running": self.jobs.len(),
        })
    }

    /// The style sources that exist today for a family: the style profiles that apply to it and the reference
    /// folders of the family. Another packet adds automatic selection.
    fn style_sources(&self, prof: &imagegen_core::profile::Profile) -> Value {
        let styles: Vec<Value> = style::list(&self.fs)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|name| style::load(&self.fs, &name).ok())
            .filter(|s| s.applies_to(prof.family))
            .map(|s| json!({"name": s.name, "pinned_references": s.references.len(), "approved_by": s.approved_by, "provider": s.provider, "model": s.model}))
            .collect();
        let dirs: Vec<Value> = prof
            .imagegen
            .reference_dirs
            .iter()
            .map(|d| {
                let mut files = Vec::new();
                let _ = self.collect_reference_files(d, 0, &mut files);
                json!({"dir": d, "exists": self.fs.exists(d), "files": files.len()})
            })
            .collect();
        json!({"style_profiles": styles, "reference_dirs": dirs, "selection": "explicit: name a style and references in the request; automatic selection is not built yet"})
    }

    /// `hoi4.image_families`
    pub fn families_info(&self, p: FamiliesParams) -> EResult<Value> {
        let measured = imagegen_core::measurement::data();
        let describe = |prof: &imagegen_core::profile::Profile, full: bool| -> Value {
            let entries = imagegen_core::measurement::entries_of(prof.family);
            let mut v = json!({
                "family": prof.family,
                "group": prof.family.group(),
                "tool": prof.family.group().tool_name(),
                "canvas": prof.canvas,
                "frames": prof.frames,
                "strip": [prof.canvas[0] * prof.frames.max(1), prof.canvas[1]],
                "accepted": prof.accepted,
                "margin": prof.margin,
                "automation": prof.imagegen.automation,
                "background": prof.imagegen.background,
                "container": prof.imagegen.container,
                "needs_mod_canvas": prof.needs_mod_canvas(),
                "mod_override": {"origin": prof.origin, "overridden": prof.overridden, "differs_from_vanilla": prof.differs_from_vanilla(), "vanilla_canvas": prof.vanilla.as_ref().map(|x| x.canvas)},
                "evidence": prof.source,
            });
            if full {
                v["measured"] = json!({
                    "game_version": measured.game_version(),
                    "entries": entries,
                    "distribution": entries.first().map(|id| measured.distribution(id).into_iter().map(|(s, n)| json!({"size": s, "files": n})).collect::<Vec<_>>()),
                });
                v["checks"] = json!(prof.imagegen.checks);
                v["check_set_hash"] = json!(imagegen_core::validate::check_set_hash(&prof.imagegen.checks));
                v["rubric"] = json!(prof.imagegen.rubric);
                v["rubric_questions"] =
                    json!(rubric::questions(&prof.imagegen.rubric).iter().map(|(k, q)| json!({"key": k, "question": q})).collect::<Vec<_>>());
                v["prompt_rules"] = json!(prof.imagegen.prompt);
                v["folder"] = json!(prof.imagegen.folder);
                v["prefix"] = json!(prof.imagegen.prefix);
                v["alt_prefixes"] = json!(prof.imagegen.alt_prefixes);
                v["sprite"] = json!(prof.imagegen.sprite);
                v["companion_files"] = json!(prof.imagegen.companion_files);
                v["variants"] = json!(prof.imagegen.variants);
                v["grayscale"] = json!(prof.imagegen.grayscale);
                v["style_sources"] = self.style_sources(prof);
                v["example_final_files"] = export::final_files(
                    prof,
                    if prof.needs_mod_canvas() { [64, 64] } else { prof.canvas },
                    "example_feature",
                    if prof.family == imagegen_core::Family::Flag { "EXA" } else { "example_asset" },
                )
                .map(|f| f.into_iter().map(|x| x.rel).collect::<Vec<_>>())
                .map(Value::from)
                .unwrap_or(Value::Null);
            }
            v
        };
        let mut out = match p.family {
            Some(f) => json!({"family": describe(self.family_profile(f)?, true)}),
            None => {
                json!({"families": self.profiles.all().map(|x| describe(x, false)).collect::<Vec<_>>(), "groups": imagegen_core::Group::ALL.iter().map(|g| json!({"group": g, "tool": g.tool_name(), "families": g.family_ids()})).collect::<Vec<_>>()})
            }
        };
        out["kit_file"] = json!(self.profiles.kit_path);
        out["kit_sha256"] = json!(self.profiles.kit_sha256);
        out["notes"] = json!(self.profiles.notes);
        if p.calibrate.unwrap_or(false) {
            out["calibration"] = match self.game_dir() {
                Some(dir) => {
                    let report = calibrate(&dir, &self.profiles)?;
                    let mut v = serde_json::to_value(&report).unwrap_or(Value::Null);
                    v["available"] = json!(true);
                    v["note"] = json!("Header-only survey of the installed game (DDS and TGA headers; no pixels, nothing copied). proposed_canvas is a proposal: nothing is written, edit the kit's profiles.json or the project config yourself.");
                    v
                }
                None => {
                    json!({"available": false, "reason": "the installed game folder was not found; set HOI4_GAME_DIR or game_dir in .hoi4-imagegen/config.toml"})
                }
            };
        }
        Ok(out)
    }

    /// `hoi4.image_references`
    pub fn references(&self, p: ReferencesParams) -> EResult<Value> {
        let need = |v: &Option<String>, what: &str| -> EResult<String> {
            v.clone().filter(|s| !s.trim().is_empty()).ok_or_else(|| EngineError::invalid(format!("{what} is required for this action")))
        };
        match p.action {
            ReferencesAction::List => {
                let mut files = Vec::new();
                self.collect_reference_files("references", 0, &mut files)?;
                let recorded: Vec<Value> = self.ledger.entries()?.into_iter().filter(|e| e.kind == kind::REFERENCE).map(|e| e.data).collect();
                Ok(
                    json!({"files": files, "recorded": recorded, "styles": style::list(&self.fs)?, "note": "Put approved reference images under references/. Paradox art is refused unless policy.allow_vanilla_references is set for the project."}),
                )
            }
            ReferencesAction::Add | ReferencesAction::Approve => {
                let path = need(&p.path, "path")?;
                let note = need(&p.note, "note (rights note: where the image comes from and what you may do with it)")?;
                let role = p.role.unwrap_or(refs::RefRole::StyleAnchor);
                let loaded =
                    refs::load_reference(&self.fs, self.game_dir().as_deref(), &self.config.policy, &ReferenceInput { path: path.clone(), role })?;
                let approving = p.action == ReferencesAction::Approve;
                if approving {
                    need(&p.approver, "approver")?;
                }
                self.ledger.append(
                    kind::REFERENCE,
                    None,
                    None,
                    json!({"action": if approving { "approve" } else { "add" }, "path": loaded.path, "role": loaded.role, "origin": loaded.origin,
                           "source_sha256": loaded.source_sha256, "sha256": loaded.sha256, "size": [loaded.width, loaded.height],
                           "note": note, "approver": p.approver}),
                )?;
                Ok(
                    json!({"path": loaded.path, "role": loaded.role, "origin": loaded.origin, "source_sha256": loaded.source_sha256, "clean_png_sha256": loaded.sha256, "size": [loaded.width, loaded.height], "notes": loaded.notes, "recorded": true, "approved": approving}),
                )
            }
            ReferencesAction::StyleList => Ok(json!({"styles": style::list(&self.fs)?})),
            ReferencesAction::StyleGet => {
                let name = need(&p.name, "name")?;
                Ok(json!({"style": style::load(&self.fs, &name)?}))
            }
            ReferencesAction::StyleSave => {
                let mut s = p.style.clone().ok_or_else(|| EngineError::invalid("style is required for style_save"))?;
                for r in &mut s.references {
                    // pin the hash of the file as it is now; a changed file later invalidates plans that use the style
                    let loaded = refs::load_reference(
                        &self.fs,
                        self.game_dir().as_deref(),
                        &self.config.policy,
                        &ReferenceInput { path: r.path.clone(), role: refs::RefRole::StyleAnchor },
                    )?;
                    r.sha256 = sha256_hex(&self.fs.read(&r.path, refs::MAX_REFERENCE_BYTES)?);
                    let _ = loaded;
                }
                if s.approved_by.is_some() && s.approved_on.is_none() {
                    s.approved_on = Some(self.now());
                }
                let rel = style::save(&self.fs, &s, p.replace.unwrap_or(false))?;
                self.ledger.append(
                    kind::STYLE,
                    None,
                    None,
                    json!({"name": s.name, "path": rel, "references": s.references, "approved_by": s.approved_by}),
                )?;
                Ok(json!({"saved": rel, "style": s}))
            }
        }
    }

    fn collect_reference_files(&self, dir: &str, depth: u32, out: &mut Vec<Value>) -> EResult<()> {
        if depth > 3 || out.len() >= 300 {
            return Ok(());
        }
        let mut names = self.fs.list_names(dir)?;
        names.sort();
        for n in names {
            let rel = format!("{dir}/{n}");
            let lower = n.to_ascii_lowercase();
            if [".png", ".jpg", ".jpeg", ".webp", ".dds", ".tga"].iter().any(|e| lower.ends_with(e)) {
                if let Ok(bytes) = self.fs.read(&rel, refs::MAX_REFERENCE_BYTES) {
                    out.push(json!({"path": rel, "bytes": bytes.len(), "sha256": sha256_hex(&bytes), "origin": refs::classify_project_path(&rel)}));
                }
            } else if !n.contains('.') {
                self.collect_reference_files(&rel, depth + 1, out)?;
            }
        }
        Ok(())
    }

    /// `hoi4.image_ledger`
    pub fn ledger_query(&self, p: LedgerParams) -> EResult<Value> {
        let limit = p.limit.unwrap_or(50).clamp(1, 500) as usize;
        let verify = self.ledger.verify()?;
        let mut entries = self.ledger.entries()?;
        if let Some(a) = &p.asset_id {
            entries.retain(|e| e.asset_id.as_deref() == Some(a.as_str()));
        }
        if let Some(a) = &p.artifact_id {
            entries.retain(|e| e.artifact_id.as_deref() == Some(a.as_str()));
        }
        if let Some(k) = &p.kind {
            entries.retain(|e| &e.kind == k);
        }
        let total = entries.len();
        let skip = total.saturating_sub(limit);
        let shown: Vec<_> = entries.into_iter().skip(skip).collect();
        let mut out = json!({
            "chain": {"ok": verify.ok, "entries": verify.entries, "first_bad": verify.first_bad},
            "matched": total,
            "returned": shown.len(),
            "entries": shown,
        });
        if p.lineage.unwrap_or(false) {
            let id = p.artifact_id.as_deref().ok_or_else(|| EngineError::invalid("lineage needs artifact_id"))?;
            out["lineage"] = json!(self.ledger.lineage(id)?);
        }
        Ok(out)
    }
}
