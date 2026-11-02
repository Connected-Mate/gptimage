//! `hoi4.image_review`: the visual review and the approval that `hoi4.image_install` needs.

use imagegen_core::error::RetryClass;
use imagegen_core::hashing::sha256_hex;
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
        let artifact = self.artifact(&p.target)?;
        let profile = self.profile_of(&artifact)?;
        let (_, sha) = self.read_artifact(&artifact)?;
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
                Some(v) if v.family != artifact.family || v.check_set_hash != self.gate_expect(artifact.family)?.check_set_hash => {
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
            let previewed = entries.iter().any(|e| {
                e.kind == kind::PREVIEWED
                    && e.artifact_id.as_deref() == Some(artifact.id.as_str())
                    && e.data.get("sha256").and_then(Value::as_str) == Some(sha.as_str())
            });
            if !previewed {
                return Err(EngineError::refused(
                    "review_needs_preview",
                    "no contact sheet was made for this exact file. Run hoi4.image_preview and look at the native-size panels before approving",
                    RetryClass::RefreshPreconditions,
                ));
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
            "note": if approval_id.is_some() {
                "Install with this approval_id. Changing the file, validating it into a failure or recording a later rejection withdraws the approval."
            } else {
                "No approval was issued. Edit or regenerate, process, validate, preview and review again."
            }
        }))
    }
}
