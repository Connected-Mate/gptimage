//! `GFX-NEAR-DUPLICATE`: an output whose frame and composition nearly repeat an earlier output of the same family.
//!
//! The hash and the family threshold live in `imagegen_core::diversity`. This file compares an image with the
//! project's earlier outputs of the family, as the ledger records them: the latest processed artifact of each other
//! asset for a processed artifact, the latest raw candidate of each other asset for a raw one. The finding is a
//! warning, never an error.

use std::collections::BTreeSet;

use imagegen_core::diversity::{self, FALLBACK_THRESHOLD, HASH_BITS};
use imagegen_core::family::Family;
use imagegen_core::naming::ArtifactId;
use imagegen_core::pixels::{decode, DecodeLimits, Rgba};
use imagegen_core::records::{kind, CandidateRecord};
use imagegen_core::validate::{Finding, Level};
use serde_json::{json, Value};

use super::Engine;

/// Earlier assets compared at most, newest first.
const MAX_EARLIER: usize = 200;
/// Findings reported at most.
const MAX_FINDINGS: usize = 5;
/// Largest earlier output read.
const MAX_BYTES: usize = 64 * 1024 * 1024;

/// What an image is compared as.
pub(crate) struct Subject<'a> {
    pub family: Family,
    pub asset_id: &'a str,
    /// True for a processed artifact (compared with processed artifacts), false for a raw candidate.
    pub processed: bool,
    /// Id of the raw candidate the image comes from, for the references it used.
    pub root_id: &'a str,
}

impl Engine {
    /// Near-duplicate findings of `img` against the project's earlier outputs of the family. `threshold` is the
    /// family's measured threshold in dHash bits, when a reference set gave one.
    pub(crate) fn near_duplicate_findings(&self, s: &Subject, img: &Rgba, threshold: Option<u32>) -> Vec<Finding> {
        let Ok(entries) = self.ledger.entries() else { return vec![] };
        let (bits, basis) = match threshold {
            Some(b) => (b, "the 1st percentile of the distances between distinct images of the family's reference set"),
            None => (FALLBACK_THRESHOLD, "the fixed fallback, as the family has no reference set to measure"),
        };
        let candidates: Vec<CandidateRecord> = entries
            .iter()
            .filter(|e| matches!(e.kind.as_str(), kind::GENERATED | kind::EDITED | kind::IMPORTED))
            .filter_map(|e| serde_json::from_value(e.data.clone()).ok())
            .collect();
        let refs_of = |raw_id: &str| -> BTreeSet<String> {
            candidates.iter().find(|c| c.candidate_id == raw_id).map(|c| c.references.iter().map(|r| r.path.clone()).collect()).unwrap_or_default()
        };
        let own_refs = refs_of(s.root_id);
        let hash = diversity::dhash(img);
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut out = Vec::new();
        for e in entries.iter().rev() {
            if seen.len() >= MAX_EARLIER || out.len() >= MAX_FINDINGS {
                break;
            }
            let wanted =
                if s.processed { e.kind == kind::PROCESSED } else { matches!(e.kind.as_str(), kind::GENERATED | kind::EDITED | kind::IMPORTED) };
            let Some(asset) = e.asset_id.as_deref() else { continue };
            if !wanted || asset == s.asset_id || e.data.get("family").and_then(Value::as_str) != Some(s.family.as_str()) {
                continue;
            }
            if !seen.insert(asset.to_string()) {
                continue;
            }
            let (Some(id), Some(path)) = (e.artifact_id.as_deref(), e.data.get("path").and_then(Value::as_str)) else { continue };
            let Ok(bytes) = self.fs.read(path, MAX_BYTES) else { continue };
            let Ok((other, _)) = decode(&bytes, Some(path), DecodeLimits::default()) else { continue };
            let d = diversity::distance(hash, diversity::dhash(&other));
            if d > bits {
                continue;
            }
            let root = if s.processed {
                e.data.get("source_id").and_then(Value::as_str).and_then(|x| ArtifactId::parse(x).ok()).map(|a| a.raw_id()).unwrap_or_default()
            } else {
                id.to_string()
            };
            let theirs = refs_of(&root);
            let shared: Vec<&String> = own_refs.intersection(&theirs).collect();
            out.push(Finding {
                code: "GFX-NEAR-DUPLICATE".into(),
                level: Level::Warning,
                message: format!(
                    "the frame and composition nearly repeat the earlier {} output {id} ({path}): dHash distance {d} of {HASH_BITS} bits, at or below {bits} ({basis}). Vary the frame or composition{}",
                    s.family,
                    if shared.is_empty() { String::new() } else { format!("; both used {} of the same reference images", shared.len()) }
                ),
                frame: None,
                evidence: json!({
                    "earlier": id,
                    "earlier_path": path,
                    "distance_bits": d,
                    "threshold_bits": bits,
                    "threshold_measured": threshold.is_some(),
                    "shared_references": shared,
                }),
            });
        }
        out
    }
}
