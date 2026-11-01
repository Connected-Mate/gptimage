//! The install gate.
//!
//! Install refuses unless all of these hold, read from the ledger and from the files as they are now:
//! the latest validation of this exact file passed, a visual review approved this exact file under the
//! approval id the caller presents, and the hashes recorded along the way (the raw candidate and the plan)
//! still match what is on disk. A changed pixel after approval invalidates the approval.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, RetryClass};
use crate::family::Family;
use crate::ledger::Entry;
use crate::profile::Profile;
use crate::records::{kind, ProcessedRecord, ReviewedRecord, ValidatedRecord, Verdict};
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
            "install_gate_checks_incomplete" => "install_gate_checks_incomplete",
            "install_gate_family_mismatch" => "install_gate_family_mismatch",
            "install_gate_chain_incomplete" => "install_gate_chain_incomplete",
            _ => "install_gate_plan_changed",
        };
        let all: Vec<String> = self.failures.iter().map(|f| format!("{}: {}", f.code, f.message)).collect();
        Err(Error::refused(code, format!("install is refused. {}", all.join(" | ")), RetryClass::RefreshPreconditions))
    }
}

fn payload<T: serde::de::DeserializeOwned>(e: &Entry) -> Option<T> {
    serde_json::from_value(e.data.clone()).ok()
}

/// What the gate expects the records to say about the artifact.
#[derive(Debug, Clone, PartialEq)]
pub struct GateExpect {
    /// The artifact's own family. Every validation and review must have been made as this family.
    pub family: Family,
    /// `validate::check_set_hash` of the family's full check set now.
    pub check_set_hash: String,
    /// The family's profile now. Its recipe says which processing stages the record must show.
    pub profile: Profile,
}

impl GateExpect {
    /// Expectations for a profile.
    pub fn for_profile(profile: &Profile) -> Self {
        GateExpect { family: profile.family, check_set_hash: crate::validate::check_set_hash(&profile.imagegen.checks), profile: profile.clone() }
    }
}

/// Evaluate the gate for one artifact.
///
/// * `entries` are all ledger entries.
/// * `artifact_id` is the processed artifact (or candidate) to install.
/// * `current_sha256` is the hash of that artifact's file now.
/// * `approval_id` is what the caller presents.
/// * `checks` are the lineage hash comparisons the caller computed from the files on disk.
pub fn evaluate(
    entries: &[Entry],
    artifact_id: &str,
    current_sha256: &str,
    approval_id: &str,
    checks: &[HashCheck],
    expect: &GateExpect,
) -> GateReport {
    let mut failures = Vec::new();
    let mut fail = |code: &str, message: String| failures.push(GateFailure { code: code.to_string(), message });

    // 1. validation of this exact file
    let validations: Vec<ValidatedRecord> =
        entries.iter().filter(|e| e.kind == kind::VALIDATED && e.artifact_id.as_deref() == Some(artifact_id)).filter_map(payload).collect();
    match validations.last() {
        None => fail("install_gate_validation_missing", format!("{artifact_id} has not been validated; run hoi4.image_validate on it")),
        Some(v) if v.sha256 != current_sha256 => fail(
            "install_gate_validation_stale",
            format!("the last validation was of a different file ({}), the file now hashes to {}", short(&v.sha256), short(current_sha256)),
        ),
        Some(v) if v.family != expect.family => fail(
            "install_gate_family_mismatch",
            format!(
                "the last validation ran as {} but this artifact is a {}; an asset can only be validated as its own family",
                v.family, expect.family
            ),
        ),
        Some(v) if v.check_set_hash != expect.check_set_hash => fail(
            "install_gate_checks_incomplete",
            format!(
                "the last validation did not run the full check set of {} (recorded {}, expected {}); validate again",
                expect.family,
                short(&v.check_set_hash),
                short(&expect.check_set_hash)
            ),
        ),
        Some(v) if v.status != Status::Pass => {
            fail("install_gate_validation_failed", format!("the last validation failed with {} error(s); fix them and validate again", v.errors))
        }
        Some(_) => {}
    }

    // 1b. the processing chain: only an artifact that went through its family's whole recipe can be installed
    match entries.iter().find(|e| e.kind == kind::PROCESSED && e.artifact_id.as_deref() == Some(artifact_id)).and_then(payload::<ProcessedRecord>) {
        None => fail(
            "install_gate_chain_incomplete",
            format!("{artifact_id} is not a processed artifact; run hoi4.image_process, which runs the family's processing, and install its result"),
        ),
        Some(rec) => {
            for problem in crate::chain::audit(&rec, &expect.profile) {
                fail("install_gate_chain_incomplete", problem);
            }
        }
    }

    // 2. review under the presented approval id
    let reviews: Vec<ReviewedRecord> =
        entries.iter().filter(|e| e.kind == kind::REVIEWED && e.artifact_id.as_deref() == Some(artifact_id)).filter_map(payload).collect();
    if reviews.is_empty() {
        fail("install_gate_review_missing", format!("{artifact_id} has no recorded visual review; run hoi4.image_review"));
    } else if let Some(r) = reviews.iter().rev().find(|r| r.approval_id.as_deref() == Some(approval_id)) {
        if r.family != expect.family {
            fail(
                "install_gate_family_mismatch",
                format!("the review under approval {approval_id} was made as {} but this artifact is a {}", r.family, expect.family),
            );
        } else if r.verdict != Verdict::Approved {
            fail("install_gate_review_not_approved", format!("the review under approval {approval_id} is {:?}, not approved", r.verdict));
        } else if r.sha256 != current_sha256 {
            fail(
                "install_gate_hash_changed",
                format!(
                    "the approved file hashed to {}, it now hashes to {}; the approval no longer applies",
                    short(&r.sha256),
                    short(current_sha256)
                ),
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
    use crate::fsio::ProjectFs;
    use crate::hashing::FixedClock;
    use crate::ledger::Ledger;
    use crate::records::{ProfileRef, ValidatedRecord};
    use serde_json::json;
    use std::sync::Arc;

    const ART: &str = "f.a-v1-p1";

    fn profile() -> ProfileRef {
        ProfileRef { id: "focus".into(), hash: "h".into(), kit_sha256: None }
    }

    fn full_set() -> Vec<String> {
        crate::profile::ProfileSet::builtin().for_family(Family::Focus).unwrap().imagegen.checks.clone()
    }

    fn focus_profile() -> Profile {
        crate::profile::ProfileSet::builtin().for_family(Family::Focus).unwrap().clone()
    }

    fn gate_expect() -> GateExpect {
        GateExpect::for_profile(&focus_profile())
    }

    /// A processed record with the focus recipe's whole chain.
    fn processed(complete: bool) -> ProcessedRecord {
        use crate::chain::Recipe;
        use crate::process::{Op, StepRecord};
        let step =
            |op: Op, i: &str, o: &str| StepRecord { op, input_sha256: i.into(), output_sha256: o.into(), output_size: [100, 88], engine: "t".into() };
        let mut steps = vec![step(Op::TrimToSubject { min_alpha: 16 }, "src", "t")];
        if complete {
            steps.push(step(Op::FitSubject { width: 100, height: 88, margin: 2, min_alpha: 16 }, "t", "out"));
        }
        ProcessedRecord {
            artifact_id: ART.into(),
            source_id: "f.a-v1".into(),
            asset_id: "f.a".into(),
            family: Family::Focus,
            run: 1,
            canvas: [100, 88],
            path: "p.png".into(),
            sha256: "aaa".into(),
            pixels_sha256: if complete { "out".into() } else { "t".into() },
            size: [100, 88],
            steps,
            profile: profile(),
            recipe: Some(crate::records::RecipeRecord { id: Recipe::Icon.id().into(), version: Recipe::Icon.version(), finishers: vec![] }),
            companions: vec![],
        }
    }

    fn validated(sha: &str, status: Status) -> ValidatedRecord {
        ValidatedRecord {
            target: ART.into(),
            family: Family::Focus,
            check_set_hash: crate::validate::check_set_hash(&full_set()),
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
            family: Family::Focus,
            sha256: sha.into(),
            verdict,
            reviewer: "tester".into(),
            rubric: "icon_v1".into(),
            answers: json!({}),
            notes: String::new(),
            approval_id: approval.map(str::to_string),
        }
    }

    fn entries(f: impl FnOnce(&Ledger)) -> Vec<Entry> {
        let dir = tempfile::tempdir().unwrap();
        let l = Ledger::new(ProjectFs::new(dir.path()).unwrap(), Arc::new(FixedClock(0)));
        l.append(kind::PROCESSED, Some("f.a"), Some(ART), processed(true)).unwrap();
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
        let r = evaluate(&e, ART, "aaa", "apr-1", &[ok], &gate_expect());
        assert!(r.passed, "{r:?}");
        assert!(r.into_result().is_ok());
    }

    #[test]
    fn each_missing_piece_closes_the_gate_with_its_own_code() {
        let empty = entries(|_| {});
        let r = evaluate(&empty, ART, "aaa", "apr-1", &[], &gate_expect());
        assert_eq!(codes(&r), vec!["install_gate_validation_missing", "install_gate_review_missing"]);

        // never processed at all, or processed without the family's fit step: the chain closes the gate
        let approved = |l: &Ledger| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        };
        let dir = tempfile::tempdir().unwrap();
        let raw = Ledger::new(ProjectFs::new(dir.path()).unwrap(), Arc::new(FixedClock(0)));
        approved(&raw);
        let r = evaluate(&raw.entries().unwrap(), ART, "aaa", "apr-1", &[], &gate_expect());
        assert_eq!(codes(&r), vec!["install_gate_chain_incomplete"]);
        assert!(r.failures[0].message.contains("not a processed artifact"));
        let dir = tempfile::tempdir().unwrap();
        let short = Ledger::new(ProjectFs::new(dir.path()).unwrap(), Arc::new(FixedClock(0)));
        short.append(kind::PROCESSED, Some("f.a"), Some(ART), processed(false)).unwrap();
        approved(&short);
        let r = evaluate(&short.entries().unwrap(), ART, "aaa", "apr-1", &[], &gate_expect());
        assert!(codes(&r).contains(&"install_gate_chain_incomplete"), "{r:?}");
        assert!(r.failures.iter().any(|f| f.message.contains("fit_subject")), "{r:?}");
        assert_eq!(r.into_result().unwrap_err().code(), "install_gate_chain_incomplete");

        let failed = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Fail)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        assert_eq!(codes(&evaluate(&failed, ART, "aaa", "apr-1", &[], &gate_expect())), vec!["install_gate_validation_failed"]);

        let no_review = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
        });
        assert_eq!(codes(&evaluate(&no_review, ART, "aaa", "apr-1", &[], &gate_expect())), vec!["install_gate_review_missing"]);

        let rejected = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Rejected, Some("apr-1"))).unwrap();
        });
        assert_eq!(codes(&evaluate(&rejected, ART, "aaa", "apr-1", &[], &gate_expect())), vec!["install_gate_review_not_approved"]);

        let wrong_id = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        assert_eq!(codes(&evaluate(&wrong_id, ART, "aaa", "apr-OTHER", &[], &gate_expect())), vec!["install_gate_approval_mismatch"]);
    }

    #[test]
    fn a_changed_file_invalidates_validation_and_approval() {
        let e = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        let r = evaluate(&e, ART, "bbb", "apr-1", &[], &gate_expect());
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
        let changed =
            HashCheck { what: "raw candidate".into(), code: "install_gate_hash_changed".into(), recorded: "r1".into(), found: Some("r2".into()) };
        let gone = HashCheck { what: "plan".into(), code: "install_gate_plan_changed".into(), recorded: "p1".into(), found: None };
        let r = evaluate(&e, ART, "aaa", "apr-1", &[changed, gone], &gate_expect());
        assert_eq!(codes(&r), vec!["install_gate_hash_changed", "install_gate_plan_changed"]);
    }

    #[test]
    fn a_later_rejection_of_the_same_file_withdraws_approval() {
        let e = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Rejected, None)).unwrap();
        });
        let r = evaluate(&e, ART, "aaa", "apr-1", &[], &gate_expect());
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
        let r = evaluate(&e, ART, "bbb", "apr-1", &[], &gate_expect());
        assert_eq!(codes(&r), vec!["install_gate_hash_changed"]);
    }

    #[test]
    fn a_validation_that_skipped_checks_cannot_open_the_gate() {
        let e = entries(|l| {
            let mut v = validated("aaa", Status::Pass);
            v.checks = vec!["size".into()];
            v.check_set_hash = crate::validate::check_set_hash(&["size".to_string()]);
            l.append(kind::VALIDATED, None, Some(ART), v).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        let r = evaluate(&e, ART, "aaa", "apr-1", &[], &gate_expect());
        assert_eq!(codes(&r), vec!["install_gate_checks_incomplete"]);
        assert_eq!(r.into_result().unwrap_err().code(), "install_gate_checks_incomplete");
    }

    #[test]
    fn the_check_set_hash_ignores_order_and_duplicates_but_not_membership() {
        let a = crate::validate::check_set_hash(&["size".into(), "alpha".into()]);
        let b = crate::validate::check_set_hash(&["alpha".into(), "size".into(), "size".into()]);
        assert_eq!(a, b);
        assert_ne!(a, crate::validate::check_set_hash(&["size".into()]));
    }

    #[test]
    fn a_validation_or_review_made_as_another_family_is_refused() {
        let wrong_validation = entries(|l| {
            let mut v = validated("aaa", Status::Pass);
            v.family = Family::IdeaSpirit;
            l.append(kind::VALIDATED, None, Some(ART), v).unwrap();
            l.append(kind::REVIEWED, None, Some(ART), reviewed("aaa", Verdict::Approved, Some("apr-1"))).unwrap();
        });
        assert_eq!(codes(&evaluate(&wrong_validation, ART, "aaa", "apr-1", &[], &gate_expect())), vec!["install_gate_family_mismatch"]);
        let wrong_review = entries(|l| {
            l.append(kind::VALIDATED, None, Some(ART), validated("aaa", Status::Pass)).unwrap();
            let mut r = reviewed("aaa", Verdict::Approved, Some("apr-1"));
            r.family = Family::Decision;
            l.append(kind::REVIEWED, None, Some(ART), r).unwrap();
        });
        assert_eq!(codes(&evaluate(&wrong_review, ART, "aaa", "apr-1", &[], &gate_expect())), vec!["install_gate_family_mismatch"]);
    }
}
