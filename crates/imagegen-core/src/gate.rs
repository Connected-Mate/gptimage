//! The install gate.
//!
//! Install refuses unless all of these hold, read from the ledger and from the files as they are now:
//! the latest validation of this exact file passed, a visual review approved this exact file under the
//! approval id the caller presents, and the hashes recorded along the way (the raw candidate and the plan)
//! still match what is on disk. A changed pixel after approval invalidates the approval.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, RetryClass};
use crate::ledger::Entry;
use crate::records::{kind, ReviewedRecord, ValidatedRecord, Verdict};
use crate::validate::Status;

/// A recorded hash compared with the hash found now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HashCheck {
    /// What was hashed (`raw candidate`, `plan`).
    pub what: String,
    /// Code to report if it differs.
    pub code: String,
    /// Hash recorded in the ledger.
    pub recorded: String,
    /// Hash found now; `None` when the file is gone.
    pub found: Option<String>,
}

/// One reason the gate is closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateFailure {
    /// Stable code.
    pub code: String,
    /// Explanation.
    pub message: String,
}

/// Outcome of the gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateReport {
    /// True when install may proceed.
    pub passed: bool,
    /// Everything that failed, not just the first.
    pub failures: Vec<GateFailure>,
}

impl GateReport {
    /// Turn a closed gate into an error carrying every reason.
    pub fn into_result(self) -> Result<(), Error> {
        if self.passed {
            return Ok(());
        }
        let first = self.failures.first().expect("closed gate has a failure");
        let code: &'static str = match first.code.as_str() {
            "install_gate_validation_missing" => "install_gate_validation_missing",
            "install_gate_validation_stale" => "install_gate_validation_stale",
            "install_gate_validation_failed" => "install_gate_validation_failed",
            "install_gate_review_missing" => "install_gate_review_missing",
            "install_gate_review_not_approved" => "install_gate_review_not_approved",
            "install_gate_approval_mismatch" => "install_gate_approval_mismatch",
            "install_gate_hash_changed" => "install_gate_hash_changed",
            _ => "install_gate_plan_changed",
        };
        let all: Vec<String> = self.failures.iter().map(|f| format!("{}: {}", f.code, f.message)).collect();
        Err(Error::refused(code, format!("install is refused. {}", all.join(" | ")), RetryClass::RefreshPreconditions))
    }
}

fn payload<T: serde::de::DeserializeOwned>(e: &Entry) -> Option<T> {
    serde_json::from_value(e.data.clone()).ok()
}

/// Evaluate the gate for one artifact.
///
/// * `entries` are all ledger entries.
/// * `artifact_id` is the processed artifact (or candidate) to install.
/// * `current_sha256` is the hash of that artifact's file now.
/// * `approval_id` is what the caller presents.
/// * `checks` are the lineage hash comparisons the caller computed from the files on disk.
pub fn evaluate(entries: &[Entry], artifact_id: &str, current_sha256: &str, approval_id: &str, checks: &[HashCheck]) -> GateReport {
    let mut failures = Vec::new();
    let mut fail = |code: &str, message: String| failures.push(GateFailure { code: code.to_string(), message });

    // 1. validation of this exact file
    let validations: Vec<ValidatedRecord> = entries
        .iter()
        .filter(|e| e.kind == kind::VALIDATED && e.artifact_id.as_deref() == Some(artifact_id))
        .filter_map(payload)
        .collect();
    match validations.last() {
        None => fail("install_gate_validation_missing", format!("{artifact_id} has not been validated; run hoi4.image_validate on it")),
        Some(v) if v.sha256 != current_sha256 => fail(
            "install_gate_validation_stale",
            format!("the last validation was of a different file ({}), the file now hashes to {}", short(&v.sha256), short(current_sha256)),
        ),
        Some(v) if v.status != Status::Pass => fail(
            "install_gate_validation_failed",
            format!("the last validation failed with {} error(s); fix them and validate again", v.errors),
        ),
        Some(_) => {}
    }

    // 2. review under the presented approval id
    let reviews: Vec<ReviewedRecord> = entries
        .iter()
        .filter(|e| e.kind == kind::REVIEWED && e.artifact_id.as_deref() == Some(artifact_id))
        .filter_map(payload)
        .collect();
    if reviews.is_empty() {
        fail("install_gate_review_missing", format!("{artifact_id} has no recorded visual review; run hoi4.image_review"));
    } else if let Some(r) = reviews.iter().rev().find(|r| r.approval_id.as_deref() == Some(approval_id)) {
        if r.verdict != Verdict::Approved {
            fail("install_gate_review_not_approved", format!("the review under approval {approval_id} is {:?}, not approved", r.verdict));
        } else if r.sha256 != current_sha256 {
            fail(
                "install_gate_hash_changed",
                format!("the approved file hashed to {}, it now hashes to {}; the approval no longer applies", short(&r.sha256), short(current_sha256)),
            );
        } else if let Some(latest) = reviews.last() {
            // a later rejection of the same file withdraws the approval
            if latest.sha256 == current_sha256 && latest.verdict != Verdict::Approved && latest.approval_id.as_deref() != Some(approval_id) {
                fail("install_gate_review_not_approved", format!("a later review of this file is {:?}", latest.verdict));
            }
        }
    } else {
        fail(
            "install_gate_approval_mismatch",
            format!("approval id '{approval_id}' does not belong to {artifact_id}; use the id returned by an approving review"),
        );
    }

    // 3. lineage hashes
    for c in checks {
        match &c.found {
            None => fail(&c.code, format!("the {} is missing now (it was {})", c.what, short(&c.recorded))),
            Some(found) if found != &c.recorded => {
                fail(&c.code, format!("the {} changed since it was recorded ({} became {})", c.what, short(&c.recorded), short(found)))
            }
            Some(_) => {}
        }
    }
    GateReport { passed: failures.is_empty(), failures }
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::Ledger;
    use crate::fsio::ProjectFs;
    use crate::hashing::FixedClock;
    use crate::records::{ProfileRef, ValidatedRecord};
    use serde_json::json;
    use std::sync::Arc;

    const ART: &str = "f.a-v1-p1";

    fn profile() -> ProfileRef {
        ProfileRef { id: "focus_icon".into(), hash: "h".into(), kit_sha256: None }
    }

    fn validated(sha: &str, status: Status) -> ValidatedRecord {
        ValidatedRecord {
            target: ART.into(),
            sha256: sha.into(),
            status,
            errors: u32::from(status == Status::Fail),
            warnings: 0,
            findings: vec![],
            checks: vec![],
            profile: profile(),
        }
    }

    fn reviewed(sha: &str, verdict: Verdict, approval: Option<&str>) -> ReviewedRecord {
        ReviewedRecord {
            target: ART.into(),
            sha256: sha.into(),
            verdict,
            reviewer: "tester".into(),
            rubric: "focus_icon_v1".into(),
            answers: json!({}),
            notes: String::new(),
            approval_id: approval.map(str::to_string),
        }
    }

    fn entries(f: impl FnOnce(&Ledger)) -> Vec<Entry> {
        let dir = tempfile::tempdir().unwrap();
        let l = Ledger::new(ProjectFs::new(dir.path()).unwrap(), Arc::new(FixedClock(0)));
        f(&l);
        l.entries().unwrap()
    }

    fn codes(r: &GateReport) -> Vec<&str> {
        r.failures.iter().map(|f| f.code.as_str()).collect()
    }

    #[test]
    fn open_when_validated_reviewed_and_unchanged() {
        let e = entries(|l| {
            l.append(kind::VALIDATED, Some("f.a"), Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, Some("f.a"), Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        let ok = HashCheck { what: "plan".into(), code: "install_gate_plan_changed".into(), recorded: "p".into(), found: Some("p".into()) };
        let r = evaluate(&e, ART, "aaa", "apr-1", &[ok]);
        assert!(r.passed, "{r:?}");
        assert!(r.into_result().is_ok());
    }

    #[test]
    fn each_missing_piece_closes_the_gate_with_its_own_code() {
        let empty = entries(|_| {});
        let r = evaluate(&empty, ART, "aaa", "apr-1", &[]);
        assert_eq!(codes(&r), vec!["install_gate_validation_missing", "install_gate_review_missing"]);

        let failed = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Fail)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        assert_eq!(codes(&evaluate(&failed, ART, "aaa", "apr-1", &[])), vec!["install_gate_validation_failed"]);

        let no_review = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
        });
        assert_eq!(codes(&evaluate(&no_review, ART, "aaa", "apr-1", &[])), vec!["install_gate_review_missing"]);

        let rejected = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Rejected, Some("apr-1"))).unwrap();
        });
        assert_eq!(codes(&evaluate(&rejected, ART, "aaa", "apr-1", &[])), vec!["install_gate_review_not_approved"]);

        let wrong_id = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        assert_eq!(codes(&evaluate(&wrong_id, ART, "aaa", "apr-OTHER", &[])), vec!["install_gate_approval_mismatch"]);
    }

    #[test]
    fn a_changed_file_invalidates_validation_and_approval() {
        let e = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        let r = evaluate(&e, ART, "bbb", "apr-1", &[]);
        assert_eq!(codes(&r), vec!["install_gate_validation_stale", "install_gate_hash_changed"]);
        let err = r.into_result().unwrap_err();
        assert_eq!(err.code(), "install_gate_validation_stale");
        assert!(err.to_string().contains("install_gate_hash_changed"), "all reasons are listed: {err}");
    }

    #[test]
    fn lineage_hash_changes_and_missing_files_close_the_gate() {
        let e = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        let changed = HashCheck { what: "raw candidate".into(), code: "install_gate_hash_changed".into(), recorded: "r1".into(), found: Some("r2".into()) };
        let gone = HashCheck { what: "plan".into(), code: "install_gate_plan_changed".into(), recorded: "p1".into(), found: None };
        let r = evaluate(&e, ART, "aaa", "apr-1", &[changed, gone]);
        assert_eq!(codes(&r), vec!["install_gate_hash_changed", "install_gate_plan_changed"]);
    }

    #[test]
    fn a_later_rejection_of_the_same_file_withdraws_approval() {
        let e = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Rejected, None)).unwrap();
        });
        let r = evaluate(&e, ART, "aaa", "apr-1", &[]);
        assert_eq!(codes(&r), vec!["install_gate_review_not_approved"]);
    }

    #[test]
    fn a_new_validation_after_a_change_must_pass_again() {
        let e = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
            l.append(kind::VALIDATED, None, Some(ART), validated("bbb", Status::Pass)).unwrap();
        });
        // file is now "bbb": validated, but the approval was for "aaa"
        let r = evaluate(&e, ART, "bbb", "apr-1", &[]);
        assert_eq!(codes(&r), vec!["install_gate_hash_changed"]);
    }
}
