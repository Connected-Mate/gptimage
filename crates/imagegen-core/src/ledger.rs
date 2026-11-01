//! The provenance ledger: an append-only, hash-chained JSON Lines file.
//!
//! Each entry holds the SHA-256 of the previous one, so a deleted, edited or reordered line is detected by
//! [`Ledger::verify`]. The ledger answers "which references, settings and outputs belong together" and is the
//! evidence the install gate reads. It lives at `.hoi4-imagegen/ledger.jsonl`, next to the project's styles
//! and config, and is meant to be committed with the mod.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::fsio::ProjectFs;
use crate::hashing::{canonical_hash, Clock};

/// Project-relative path of the ledger.
pub const LEDGER_PATH: &str = ".hoi4-imagegen/ledger.jsonl";
const LOCK_PATH: &str = ".hoi4-imagegen/ledger.lock";
const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One ledger line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Entry {
    /// 1-based sequence number.
    pub seq: u64,
    /// RFC 3339 UTC time.
    pub ts: String,
    /// Event kind, see [`crate::records::kind`].
    pub kind: String,
    /// `feature.asset` the event belongs to.
    pub asset_id: Option<String>,
    /// Candidate or processed artifact id.
    pub artifact_id: Option<String>,
    /// Event payload.
    pub data: Value,
    /// Hash of the previous entry (zeros for the first).
    pub prev: String,
    /// Hash of this entry's other fields.
    pub hash: String,
}

fn entry_hash(e: &Entry) -> String {
    canonical_hash(&json!({
        "seq": e.seq, "ts": e.ts, "kind": e.kind, "asset_id": e.asset_id,
        "artifact_id": e.artifact_id, "data": e.data, "prev": e.prev,
    }))
}

/// Result of checking the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VerifyReport {
    /// True when every entry checks out.
    pub ok: bool,
    /// Entries read.
    pub entries: u64,
    /// First bad entry and why.
    pub first_bad: Option<(u64, String)>,
}

/// Handle to the ledger of one project.
#[derive(Clone)]
pub struct Ledger {
    fs: ProjectFs,
    clock: Arc<dyn Clock>,
}

struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        // On Windows the removal can fail for a moment (another thread's failed open still holds a handle). A lock
        // that stays behind blocks every later append for a minute, so the removal is retried.
        for _ in 0..200 {
            match fs::remove_file(&self.0) {
                Ok(()) => return,
                Err(e) if e.kind() == ErrorKind::NotFound => return,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }
}

impl Ledger {
    /// Open the ledger of a project (nothing is created until the first append).
    pub fn new(fs: ProjectFs, clock: Arc<dyn Clock>) -> Self {
        Ledger { fs, clock }
    }

    fn lock(&self) -> Result<LockGuard> {
        let path = self.fs.resolve(LOCK_PATH)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent.display(), e))?;
        }
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(LockGuard(path));
                }
                Err(e) if matches!(e.kind(), ErrorKind::AlreadyExists | ErrorKind::PermissionDenied | ErrorKind::NotFound) => {
                    if e.kind() == ErrorKind::NotFound {
                        // Windows briefly reports a lock that another thread is deleting as "not found".
                        if let Some(parent) = path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                    }
                    // On Windows a lock that is being deleted reports "access denied" instead of "exists".
                    // A lock older than a minute belongs to a crashed process.
                    let stale = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .map(|t| SystemTime::now().duration_since(t).map(|d| d > Duration::from_secs(60)).unwrap_or(false))
                        .unwrap_or(false);
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > Duration::from_secs(10) {
                        return Err(Error::Ledger(format!("timed out waiting for the ledger lock ({e})")));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(Error::io(path.display(), e)),
            }
        }
    }

    /// Append one event. Returns the stored entry.
    pub fn append(&self, kind: &str, asset_id: Option<&str>, artifact_id: Option<&str>, data: impl Serialize) -> Result<Entry> {
        let data = serde_json::to_value(data).map_err(|e| Error::Ledger(format!("cannot serialize event: {e}")))?;
        let _guard = self.lock()?;
        let path = self.fs.resolve(LEDGER_PATH)?;
        let existing = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
            Err(e) => return Err(Error::io(path.display(), e)),
        };
        if !existing.is_empty() && !existing.ends_with('\n') {
            return Err(Error::Ledger(
                "the ledger ends with a partial line (an interrupted write). Inspect .hoi4-imagegen/ledger.jsonl before adding to it".into(),
            ));
        }
        let (seq, prev) = match existing.lines().last() {
            Some(line) => {
                let last: Entry = serde_json::from_str(line).map_err(|e| Error::Ledger(format!("last ledger line is unreadable: {e}")))?;
                (last.seq + 1, last.hash)
            }
            None => (1, GENESIS.to_string()),
        };
        let mut entry = Entry {
            seq,
            ts: self.clock.now_rfc3339(),
            kind: kind.to_string(),
            asset_id: asset_id.map(str::to_string),
            artifact_id: artifact_id.map(str::to_string),
            data,
            prev,
            hash: String::new(),
        };
        entry.hash = entry_hash(&entry);
        let mut line = serde_json::to_string(&entry).map_err(|e| Error::Ledger(e.to_string()))?;
        line.push('\n');
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent.display(), e))?;
        }
        let mut f = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| Error::io(path.display(), e))?;
        f.write_all(line.as_bytes()).map_err(|e| Error::io(path.display(), e))?;
        f.sync_all().map_err(|e| Error::io(path.display(), e))?;
        Ok(entry)
    }

    /// Every entry, in order. Fails if a line cannot be parsed.
    pub fn entries(&self) -> Result<Vec<Entry>> {
        let path = self.fs.resolve(LEDGER_PATH)?;
        let text = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(path.display(), e)),
        };
        text.lines()
            .enumerate()
            .map(|(i, l)| serde_json::from_str::<Entry>(l).map_err(|e| Error::Ledger(format!("ledger line {} is unreadable: {e}", i + 1))))
            .collect()
    }

    /// Check the whole chain.
    pub fn verify(&self) -> Result<VerifyReport> {
        let entries = match self.entries() {
            Ok(e) => e,
            Err(Error::Ledger(msg)) => return Ok(VerifyReport { ok: false, entries: 0, first_bad: Some((0, msg)) }),
            Err(e) => return Err(e),
        };
        let mut prev = GENESIS.to_string();
        for (i, e) in entries.iter().enumerate() {
            let bad = |why: &str| VerifyReport { ok: false, entries: entries.len() as u64, first_bad: Some((e.seq, why.to_string())) };
            if e.seq != i as u64 + 1 {
                return Ok(bad("sequence number is not contiguous"));
            }
            if e.prev != prev {
                return Ok(bad("previous-hash link is broken"));
            }
            if entry_hash(e) != e.hash {
                return Ok(bad("entry content does not match its hash"));
            }
            prev = e.hash.clone();
        }
        Ok(VerifyReport { ok: true, entries: entries.len() as u64, first_bad: None })
    }

    /// Entries for one asset.
    pub fn for_asset(&self, asset_id: &str) -> Result<Vec<Entry>> {
        Ok(self.entries()?.into_iter().filter(|e| e.asset_id.as_deref() == Some(asset_id)).collect())
    }

    /// Entries for one artifact.
    pub fn for_artifact(&self, artifact_id: &str) -> Result<Vec<Entry>> {
        Ok(self.entries()?.into_iter().filter(|e| e.artifact_id.as_deref() == Some(artifact_id)).collect())
    }

    /// The generate, edit or import entry that used an idempotency key.
    pub fn find_idempotency(&self, key: &str) -> Result<Option<Entry>> {
        Ok(self.entries()?.into_iter().find(|e| {
            matches!(e.kind.as_str(), "generated" | "edited") && e.data.get("idempotency_key").and_then(Value::as_str) == Some(key)
        }))
    }

    /// Highest version number ever allocated for an asset (so a deleted file's number is not reused).
    pub fn highest_version(&self, asset_id: &str) -> Result<u32> {
        Ok(self
            .for_asset(asset_id)?
            .iter()
            .filter(|e| matches!(e.kind.as_str(), "generated" | "edited" | "imported"))
            .filter_map(|e| e.data.get("version").and_then(Value::as_u64))
            .max()
            .unwrap_or(0) as u32)
    }

    /// Chain of parents from an artifact back to its first ancestor, nearest first.
    pub fn lineage(&self, artifact_id: &str) -> Result<Vec<String>> {
        let entries = self.entries()?;
        let mut chain = vec![artifact_id.to_string()];
        let mut cur = artifact_id.to_string();
        for _ in 0..1000 {
            let parent = entries
                .iter()
                .find(|e| e.artifact_id.as_deref() == Some(cur.as_str()) && matches!(e.kind.as_str(), "generated" | "edited" | "imported" | "processed"))
                .and_then(|e| e.data.get("parent").or_else(|| e.data.get("source_id")))
                .and_then(Value::as_str)
                .map(str::to_string);
            match parent {
                Some(p) if !chain.contains(&p) => {
                    chain.push(p.clone());
                    cur = p;
                }
                _ => break,
            }
        }
        Ok(chain)
    }

    /// Path of the ledger file, for display.
    pub fn path(&self) -> PathBuf {
        self.fs.root().join(LEDGER_PATH)
    }

    /// True when the ledger file exists.
    pub fn exists(&self) -> bool {
        Path::new(&self.path()).is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashing::FixedClock;

    fn ledger(dir: &tempfile::TempDir) -> Ledger {
        Ledger::new(ProjectFs::new(dir.path()).unwrap(), Arc::new(FixedClock(1_793_404_800)))
    }

    #[test]
    fn append_chain_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        let l = ledger(&dir);
        assert!(l.verify().unwrap().ok);
        let a = l.append("generated", Some("f.a"), Some("f.a-v1"), json!({"version": 1, "idempotency_key": "k1"})).unwrap();
        let b = l.append("validated", Some("f.a"), Some("f.a-v1"), json!({"status": "pass"})).unwrap();
        assert_eq!((a.seq, b.seq), (1, 2));
        assert_eq!(b.prev, a.hash);
        assert_eq!(a.ts, "2026-10-31T00:00:00Z");
        let report = l.verify().unwrap();
        assert!(report.ok && report.entries == 2, "{report:?}");
        assert_eq!(l.for_asset("f.a").unwrap().len(), 2);
        assert_eq!(l.find_idempotency("k1").unwrap().unwrap().seq, 1);
        assert!(l.find_idempotency("nope").unwrap().is_none());
        assert_eq!(l.highest_version("f.a").unwrap(), 1);
    }

    #[test]
    fn tampering_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let l = ledger(&dir);
        for i in 0..3 {
            l.append("generated", Some("f.a"), Some(&format!("f.a-v{i}")), json!({"version": i})).unwrap();
        }
        let path = l.path();
        let original = std::fs::read_to_string(&path).unwrap();

        // edit a payload
        std::fs::write(&path, original.replacen("\"version\":1", "\"version\":9", 1)).unwrap();
        let r = l.verify().unwrap();
        assert!(!r.ok && r.first_bad.as_ref().unwrap().0 == 2, "{r:?}");

        // delete a middle line
        let lines: Vec<&str> = original.lines().collect();
        std::fs::write(&path, format!("{}\n{}\n", lines[0], lines[2])).unwrap();
        assert!(!l.verify().unwrap().ok);

        // swap two lines
        std::fs::write(&path, format!("{}\n{}\n{}\n", lines[1], lines[0], lines[2])).unwrap();
        assert!(!l.verify().unwrap().ok);

        // garbage
        std::fs::write(&path, "not json\n").unwrap();
        let r = l.verify().unwrap();
        assert!(!r.ok);

        // restored
        std::fs::write(&path, original).unwrap();
        assert!(l.verify().unwrap().ok);
    }

    #[test]
    fn partial_last_line_blocks_appends() {
        let dir = tempfile::tempdir().unwrap();
        let l = ledger(&dir);
        l.append("generated", Some("f.a"), None, json!({})).unwrap();
        let mut f = OpenOptions::new().append(true).open(l.path()).unwrap();
        f.write_all(b"{\"seq\":2").unwrap();
        drop(f);
        assert!(l.append("generated", Some("f.a"), None, json!({})).is_err());
    }

    #[test]
    fn concurrent_appends_keep_the_chain_intact() {
        let dir = tempfile::tempdir().unwrap();
        let l = ledger(&dir);
        let handles: Vec<_> = (0..12)
            .map(|i| {
                let l = l.clone();
                std::thread::spawn(move || l.append("generated", Some("f.a"), Some(&format!("f.a-v{i}")), json!({"version": i})).unwrap().seq)
            })
            .collect();
        let mut seqs: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        seqs.sort_unstable();
        assert_eq!(seqs, (1..=12).collect::<Vec<_>>());
        let r = l.verify().unwrap();
        assert!(r.ok && r.entries == 12, "{r:?}");
        assert!(!dir.path().join(LOCK_PATH).exists(), "lock released");
    }

    #[test]
    fn lineage_follows_parents_and_source_ids() {
        let dir = tempfile::tempdir().unwrap();
        let l = ledger(&dir);
        l.append("generated", Some("f.a"), Some("f.a-v1"), json!({"version": 1})).unwrap();
        l.append("edited", Some("f.a"), Some("f.a-v2"), json!({"version": 2, "parent": "f.a-v1"})).unwrap();
        l.append("processed", Some("f.a"), Some("f.a-v2-p1"), json!({"source_id": "f.a-v2"})).unwrap();
        assert_eq!(l.lineage("f.a-v2-p1").unwrap(), vec!["f.a-v2-p1", "f.a-v2", "f.a-v1"]);
        assert_eq!(l.lineage("f.a-v1").unwrap(), vec!["f.a-v1"]);
    }
}
