//! `hoi4.image_review`: the visual review and the approval that `hoi4.image_install` needs.
//!
//! The caller's rubric answers alone never approve. Since 0.3.2 the review measures the file against the family's
//! vanilla art itself (`imagegen_core::vanillamatch`) and an approval needs that check to pass, and a preview of the
//! exact file that put the vanilla references beside it. A deliberate mod style can pass the vanilla check only with
//! an explicit override and a reason, both recorded in the ledger and in the reply.

use imagegen_core::error::RetryClass;
use imagegen_core::hashing::sha256_hex;
use imagegen_core::pixels::{decode, DecodeLimits};
use imagegen_core::records::{kind, ReviewedRecord, ValidatedRecord, Verdict};
use imagegen_core::rubric;
use imagegen_core::validate::Status;
use serde_json::{json, Value};

use crate::api::ReviewParams;

use super::{EResult, Engine, EngineError};

impl Engine {
    /// Record a review. An approval returns the `approval_id` install requires.
    pub fn review(&self, p: ReviewParams) -> EResult<Value> {
        let reviewer = p.reviewer.trim().to_string();
        if reviewer.is_empty() || reviewer.chars().count() > 80 {
            return Err(EngineError::invalid("reviewer must be 1 to 80 characters"));
        }
        let notes = p.notes.clone().unwrap_or_default();
        if notes.chars().count() > 4000 {
            return Err(EngineError::invalid("notes must be at most 4000 characters"));
        }
        let override_on = p.override_vanilla_check.unwrap_or(false);
        let override_reason = p.override_reason.as_deref().map(str::trim).unwrap_or("").to_string();
        if override_on && !(10..=1000).contains(&override_reason.chars().count()) {
            return Err(EngineError::invalid(
                "override_vanilla_check needs an override_reason of 10 to 1000 characters saying why the art may differ from vanilla",
            ));
        }
        if !override_on && !override_reason.is_empty() {
            return Err(EngineError::invalid("override_reason is only used with override_vanilla_check: true"));
        }
        let artifact = self.artifact(&p.target)?;
        let profile = &self.profile_of(&artifact)?;
        let (bytes, sha) = self.read_artifact(&artifact)?;
        let (img, _) = decode(&bytes, Some(&artifact.path), DecodeLimits::default())?;
        let vanilla_check = self.reference_check(&artifact, &img);
        // a style pack's art is measured against the pack's references, any other against vanilla's
        let against = match vanilla_check.get("reference_set").and_then(|s| s.get("pack")).and_then(Value::as_str) {
            Some(name) => format!("the style pack '{name}' references of its family"),
            None => "the family's vanilla art".to_string(),
        };
        let vanilla_status = vanilla_check.get("status").and_then(Value::as_str).unwrap_or("unavailable").to_string();
        let vanilla_override = override_on.then(|| json!({"reason": override_reason, "status_overridden": vanilla_status}));
        let rubric_id = profile.imagegen.rubric.clone();
        let answers = p.answers.clone().unwrap_or_else(|| json!({}));
        if !answers.is_object() {
            return Err(EngineError::invalid("answers must be an object of {question: true|false}"));
        }
        let questions: Vec<Value> = rubric::questions(&rubric_id).iter().map(|(k, q)| json!({"key": k, "question": q})).collect();

        let mut approval_id = None;
        if p.verdict == Verdict::Approved {
            let entries = self.ledger.entries()?;
            let latest_validation = entries
                .iter()
                .filter(|e| e.kind == kind::VALIDATED && e.artifact_id.as_deref() == Some(artifact.id.as_str()))
                .filter_map(|e| serde_json::from_value::<ValidatedRecord>(e.data.clone()).ok())
                .next_back();
            match latest_validation {
                Some(v) if v.family != artifact.family || v.check_set_hash != self.gate_expect(profile).check_set_hash => {
                    return Err(EngineError::refused(
                        "review_needs_validation",
                        "the last validation did not run the full check set of this artifact's family; validate it again before approving",
                        RetryClass::RefreshPreconditions,
                    ))
                }
                Some(v) if v.sha256 == sha && v.status == Status::Pass => {}
                Some(v) if v.sha256 != sha => {
                    return Err(EngineError::refused(
                        "review_needs_validation",
                        "the file changed since it was last validated; validate it again before approving",
                        RetryClass::RefreshPreconditions,
                    ))
                }
                Some(_) => {
                    return Err(EngineError::refused(
                        "review_needs_validation",
                        "the last validation failed; fix the findings and validate again before approving",
                        RetryClass::RefreshPreconditions,
                    ))
                }
                None => {
                    return Err(EngineError::refused(
                        "review_needs_validation",
                        "this artifact has not been validated; run hoi4.image_validate first",
                        RetryClass::RefreshPreconditions,
                    ))
                }
            }
            let previews: Vec<&Value> = entries
                .iter()
                .filter(|e| {
                    e.kind == kind::PREVIEWED
                        && e.artifact_id.as_deref() == Some(artifact.id.as_str())
                        && e.data.get("sha256").and_then(Value::as_str) == Some(sha.as_str())
                })
                .map(|e| &e.data)
                .collect();
            if previews.is_empty() {
                return Err(EngineError::refused(
                    "review_needs_preview",
                    "no contact sheet was made for this exact file. Run hoi4.image_preview and look at the native-size panels and the vanilla side-by-side sheet before approving",
                    RetryClass::RefreshPreconditions,
                ));
            }
            let vanilla_previewed = previews.iter().any(|d| {
                d.get("vanilla_sheet").and_then(|v| v.get("sha256")).and_then(Value::as_str).is_some()
                    && d.get("vanilla_sheet").and_then(|v| v.get("references")).and_then(Value::as_array).is_some_and(|r| !r.is_empty())
            });
            if !override_on {
                if !vanilla_previewed {
                    let why = vanilla_check.get("reason").and_then(Value::as_str).map(|r| format!(" ({r})")).unwrap_or_default();
                    return Err(EngineError::refused(
                        "review_needs_vanilla_preview",
                        format!(
                            "no preview of this exact file showed it beside {against}{why}. Run hoi4.image_preview again and compare it with the vanilla side-by-side sheet before approving, or approve a deliberate mod style with override_vanilla_check and an override_reason"
                        ),
                        RetryClass::RefreshPreconditions,
                    )
                    .with_details(json!({"vanilla_check": vanilla_check})));
                }
                match vanilla_status.as_str() {
                    "pass" => {}
                    "fail" => {
                        let outside: Vec<imagegen_core::vanillamatch::Outside> =
                            serde_json::from_value(vanilla_check.get("outside").cloned().unwrap_or(Value::Null)).unwrap_or_default();
                        let what = if outside.is_empty() {
                            vanilla_check.get("reason").and_then(Value::as_str).unwrap_or("it could not be measured").to_string()
                        } else {
                            imagegen_core::vanillamatch::describe(&outside)
                        };
                        return Err(EngineError::refused(
                            "review_vanilla_mismatch",
                            format!(
                                "the file does not measure like {against}, so it cannot be approved: {what}. Fix the art or its processing (see the vanilla side-by-side sheet), or approve a deliberate mod style with override_vanilla_check and an override_reason. Use verdict changes_requested if the art needs another round"
                            ),
                            RetryClass::RefreshPreconditions,
                        )
                        .with_details(json!({"vanilla_check": vanilla_check})));
                    }
                    _ => {
                        let why = vanilla_check.get("reason").and_then(Value::as_str).unwrap_or("unknown");
                        return Err(EngineError::refused(
                            "review_needs_vanilla_check",
                            format!(
                                "the vanilla-match check cannot run: {why}. An approval needs it. Point HOI4_GAME_DIR at the installed game, or approve a deliberate mod style with override_vanilla_check and an override_reason"
                            ),
                            RetryClass::RefreshPreconditions,
                        )
                        .with_details(json!({"vanilla_check": vanilla_check})));
                    }
                }
            }
            let check = rubric::check_answers(&rubric_id, &answers);
            if !check.all_true() {
                return Err(EngineError::refused(
                    "review_incomplete",
                    format!(
                        "an approval needs every rubric answer true. Missing: [{}]. Not true: [{}]. Use verdict changes_requested if the art needs another round",
                        check.missing.join(", "),
                        check.failed.join(", ")
                    ),
                    RetryClass::RefreshPreconditions,
                )
                .with_details(json!({"rubric": rubric_id, "questions": questions})));
            }
            let next_seq = entries.len() + 1;
            approval_id =
                Some(format!("apr-{}", &sha256_hex(format!("{}|{}|{}|{}|{}", artifact.id, sha, reviewer, self.now(), next_seq).as_bytes())[..16]));
        }

        let record = ReviewedRecord {
            target: artifact.id.clone(),
            family: artifact.family,
            sha256: sha.clone(),
            verdict: p.verdict,
            reviewer,
            rubric: rubric_id.clone(),
            answers,
            notes,
            approval_id: approval_id.clone(),
            vanilla_check: Some(vanilla_check.clone()),
            vanilla_override: vanilla_override.clone(),
        };
        self.ledger.append(kind::REVIEWED, Some(&artifact.asset_id), Some(&artifact.id), &record)?;
        Ok(json!({
            "target": artifact.id,
            "family": artifact.family,
            "sha256": sha,
            "verdict": p.verdict,
            "approval_id": approval_id,
            "rubric": rubric_id,
            "questions": questions,
            "vanilla_check": vanilla_check,
            "vanilla_override": vanilla_override,
            "note": if approval_id.is_some() {
                "Install with this approval_id. Changing the file, validating it into a failure or recording a later rejection withdraws the approval."
            } else {
                "No approval was issued. Edit or regenerate, process, validate, preview and review again."
            }
        }))
    }
}
