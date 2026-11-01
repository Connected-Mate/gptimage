//! Project-contained file access and never-overwrite writes.
//!
//! Upstream gptimage saved anywhere (absolute paths, `..`, any extension) and checked for an existing
//! file before writing, which races. This module replaces that: every path is project-relative and
//! checked lexically and on disk (no `..`, no drive or UNC forms, no alternate streams, no reserved
//! device names, no 8.3 short names, no symlink or junction on the way), and new files are created
//! exclusively (write a temp file, then hard-link it into place, which fails if the name exists).

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};
use crate::naming::{highest_version, is_reserved_name};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory tree an agent may read and write inside.
#[derive(Debug, Clone)]
pub struct ProjectFs {
    root: PathBuf,
}

/// Result of a versioned write.
#[derive(Debug, Clone)]
pub struct Written {
    /// Project-relative path with forward slashes.
    pub rel: String,
    /// Absolute path.
    pub abs: PathBuf,
    /// The `-vN` number that was allocated.
    pub version: u32,
}

/// One file of an all-or-nothing batch write.
#[derive(Debug, Clone)]
pub struct PendingWrite {
    /// Project-relative path.
    pub rel: String,
    /// Bytes to write.
    pub bytes: Vec<u8>,
    /// True to replace an existing file (the previous bytes are kept until the batch is finished), false for a
    /// file that must not exist yet.
    pub replace: bool,
}

/// A batch that has been put in place and can still be undone. Dropping it without calling
/// [`CommittedBatch::finish`] or [`CommittedBatch::rollback`] leaves the new files in place and the backups of
/// replaced files next to them.
#[derive(Debug)]
pub struct CommittedBatch {
    /// `(target, backup of the replaced file)` in commit order.
    done: Vec<(PathBuf, Option<PathBuf>)>,
    /// Directories the batch created, deepest last.
    made_dirs: Vec<PathBuf>,
}

impl CommittedBatch {
    /// Keep the new files and delete the backups.
    pub fn finish(self) {
        for (_, backup) in &self.done {
            if let Some(b) = backup {
                let _ = fs::remove_file(b);
            }
        }
    }

    /// Put every file back as it was: new files are removed, replaced files restored, directories the batch
    /// created are removed when empty.
    pub fn rollback(self) {
        undo(&self.done);
        for d in self.made_dirs.iter().rev() {
            let _ = fs::remove_dir(d);
        }
    }
}

fn undo(done: &[(PathBuf, Option<PathBuf>)]) {
    for (target, backup) in done.iter().rev() {
        let _ = fs::remove_file(target);
        if let Some(b) = backup {
            let _ = fs::rename(b, target);
        }
    }
}

/// Split and check a project-relative path. Returns the normalized components.
pub fn check_relative(rel: &str) -> Result<Vec<String>> {
    let reject = |why: &str| Err(Error::PathRejected(format!("'{rel}': {why}")));
    if rel.is_empty() || rel.len() > 400 {
        return reject("empty or too long");
    }
    if rel.chars().any(|c| c == '\0' || c.is_control()) {
        return reject("contains control characters");
    }
    let unified = rel.replace('\\', "/");
    if unified.starts_with('/') {
        return reject("absolute or UNC paths are not allowed");
    }
    let bytes = unified.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return reject("drive-letter paths are not allowed");
    }
    let mut parts = Vec::new();
    for comp in unified.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            return reject("'..' is not allowed");
        }
        if comp.len() > 128 {
            return reject("a path component is too long");
        }
        if comp.ends_with('.') || comp.ends_with(' ') {
            return reject("components may not end with a dot or space");
        }
        if comp.chars().any(|c| matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
            return reject("components may not contain : * ? \" < > |");
        }
        if is_reserved_name(comp) {
            return reject("reserved device name");
        }
        if looks_like_short_name(comp) {
            return reject("8.3 short names (NAME~1) are not allowed");
        }
        parts.push(comp.to_string());
    }
    if parts.is_empty() {
        return reject("no path components");
    }
    Ok(parts)
}

/// `PROGRA~1`, `FILE~2.TXT`: a Windows 8.3 alias can point at a different file than its spelling suggests.
fn looks_like_short_name(comp: &str) -> bool {
    let stem = comp.split('.').next().unwrap_or("");
    match stem.rfind('~') {
        Some(i) => i > 0 && i + 1 < stem.len() && stem[i + 1..].chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

#[cfg(windows)]
fn is_link(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_link(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// Remove the `\\?\` prefix Windows adds to canonical paths so paths print normally.
pub fn display_path(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

impl ProjectFs {
    /// Open a project root. The directory must exist.
    pub fn new(root: &Path) -> Result<Self> {
        let canon = fs::canonicalize(root).map_err(|e| Error::io(root.display(), e))?;
        if !canon.is_dir() {
            return Err(Error::Invalid(format!("project root {} is not a directory", root.display())));
        }
        Ok(ProjectFs { root: canon })
    }

    /// The canonical root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a project-relative path, refusing escapes and links.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        let parts = check_relative(rel)?;
        let mut cur = self.root.clone();
        for part in &parts {
            cur.push(part);
            match fs::symlink_metadata(&cur) {
                Ok(meta) => {
                    if is_link(&meta) {
                        return Err(Error::PathRejected(format!("'{rel}': '{part}' is a symbolic link or junction")));
                    }
                }
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(cur.display(), e)),
            }
        }
        // Belt and braces: the deepest existing ancestor must still be inside the root once canonical.
        let mut probe = cur.clone();
        while !probe.exists() {
            if !probe.pop() {
                break;
            }
        }
        let canon = fs::canonicalize(&probe).map_err(|e| Error::io(probe.display(), e))?;
        if !canon.starts_with(&self.root) {
            return Err(Error::PathRejected(format!("'{rel}' resolves outside the project")));
        }
        Ok(cur)
    }

    /// Project-relative display form of an absolute path inside the root.
    pub fn rel(&self, abs: &Path) -> Result<String> {
        let stripped = abs.strip_prefix(&self.root).map_err(|_| Error::PathRejected(format!("{} is outside the project", display_path(abs))))?;
        Ok(stripped.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"))
    }

    /// True when the project-relative path exists.
    pub fn exists(&self, rel: &str) -> bool {
        self.resolve(rel).map(|p| p.exists()).unwrap_or(false)
    }

    /// Read a file, refusing anything larger than `max_bytes`.
    pub fn read(&self, rel: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let path = self.resolve(rel)?;
        read_limited(&path, max_bytes)
    }

    /// Create a directory and its parents.
    pub fn create_dir_all(&self, rel: &str) -> Result<PathBuf> {
        let path = self.resolve(rel)?;
        fs::create_dir_all(&path).map_err(|e| Error::io(path.display(), e))?;
        Ok(path)
    }

    /// Names (not paths) of the entries in a directory; empty when it does not exist.
    pub fn list_names(&self, dir_rel: &str) -> Result<Vec<String>> {
        let dir = self.resolve(dir_rel)?;
        match fs::read_dir(&dir) {
            Ok(rd) => Ok(rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).collect()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(Error::io(dir.display(), e)),
        }
    }

    /// Create a file that must not exist yet.
    pub fn write_new(&self, rel: &str, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent.display(), e))?;
        }
        create_new_atomic(&path, bytes)?;
        Ok(path)
    }

    /// Write `dir/stem-vN.ext` with the first free N greater than both `min_version` and every
    /// version already on disk. Safe against concurrent writers: a lost race moves on to the next number.
    pub fn write_next_version(&self, dir_rel: &str, stem: &str, ext: &str, min_version: u32, bytes: &[u8]) -> Result<Written> {
        let dir = self.create_dir_all(dir_rel)?;
        let existing = self.list_names(dir_rel)?;
        let mut v = highest_version(existing.iter().map(String::as_str), stem, ext).max(min_version) + 1;
        for _ in 0..10_000 {
            let name = format!("{stem}-v{v}.{ext}");
            let rel = format!("{}/{}", dir_rel.trim_end_matches('/'), name);
            let abs = self.resolve(&rel)?;
            match create_new_atomic(&abs, bytes) {
                Ok(()) => return Ok(Written { rel, abs, version: v }),
                Err(Error::Exists(_)) => v += 1,
                Err(e) => return Err(e),
            }
        }
        Err(Error::Exists(format!("{}/{stem}: no free version number", display_path(&dir))))
    }

    /// Write several files all or nothing. Every file is first written to a temporary name next to its target, so a
    /// failure while writing leaves nothing behind. The files are then moved into place one by one; if one move
    /// fails, the ones already moved are removed again (replaced files are restored from their backup) and the
    /// error is returned. On success the returned batch can still be rolled back, for a step that follows
    /// (the ledger entry) and can fail too.
    pub fn write_batch(&self, writes: &[PendingWrite]) -> Result<CommittedBatch> {
        let mut targets = Vec::with_capacity(writes.len());
        for w in writes {
            let t = self.resolve(&w.rel)?;
            if targets.contains(&t) {
                return Err(Error::Invalid(format!("{} appears twice in one batch", w.rel)));
            }
            targets.push(t);
        }
        // stage
        let mut made_dirs: Vec<PathBuf> = Vec::new();
        let mut staged: Vec<PathBuf> = Vec::new();
        let cleanup = |staged: &[PathBuf], made: &[PathBuf]| {
            for t in staged {
                let _ = fs::remove_file(t);
            }
            for d in made.iter().rev() {
                let _ = fs::remove_dir(d);
            }
        };
        for (w, target) in writes.iter().zip(&targets) {
            if let Some(parent) = target.parent() {
                let mut missing = Vec::new();
                let mut p = parent.to_path_buf();
                while !p.exists() {
                    missing.push(p.clone());
                    if !p.pop() {
                        break;
                    }
                }
                if let Err(e) = fs::create_dir_all(parent) {
                    cleanup(&staged, &made_dirs);
                    return Err(Error::io(parent.display(), e));
                }
                missing.reverse();
                made_dirs.extend(missing);
            }
            let tmp = temp_name(target);
            let written = OpenOptions::new().write(true).create_new(true).open(&tmp).and_then(|mut f| {
                f.write_all(&w.bytes)?;
                f.sync_all()
            });
            if let Err(e) = written {
                let _ = fs::remove_file(&tmp);
                cleanup(&staged, &made_dirs);
                return Err(Error::io(tmp.display(), e));
            }
            staged.push(tmp);
        }
        // commit
        let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        for (i, (w, target)) in writes.iter().zip(&targets).enumerate() {
            let outcome: Result<Option<PathBuf>> = (|| {
                if w.replace {
                    if target.exists() {
                        let mut backup = temp_name(target).into_os_string();
                        backup.push(".bak");
                        let backup = PathBuf::from(backup);
                        fs::rename(target, &backup).map_err(|e| Error::io(target.display(), e))?;
                        if let Err(e) = fs::rename(&staged[i], target) {
                            let _ = fs::rename(&backup, target);
                            return Err(Error::io(target.display(), e));
                        }
                        return Ok(Some(backup));
                    }
                    fs::rename(&staged[i], target).map_err(|e| Error::io(target.display(), e))?;
                    return Ok(None);
                }
                // a new file: the hard link fails when the name exists, which a plain rename would not
                match fs::hard_link(&staged[i], target) {
                    Ok(()) => {
                        let _ = fs::remove_file(&staged[i]);
                        Ok(None)
                    }
                    Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(Error::Exists(display_path(target))),
                    Err(_) => {
                        if target.exists() {
                            return Err(Error::Exists(display_path(target)));
                        }
                        fs::rename(&staged[i], target).map_err(|e| Error::io(target.display(), e))?;
                        Ok(None)
                    }
                }
            })();
            match outcome {
                Ok(backup) => done.push((target.clone(), backup)),
                Err(e) => {
                    undo(&done);
                    cleanup(&staged, &made_dirs);
                    return Err(e);
                }
            }
        }
        // anything still staged (a hard link left its temp behind) is removed
        for t in &staged {
            let _ = fs::remove_file(t);
        }
        Ok(CommittedBatch { done, made_dirs })
    }

    /// Atomically replace (or create) a file. Only for explicit, approved replacements.
    pub fn replace(&self, rel: &str, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent.display(), e))?;
        }
        let tmp = temp_name(&path);
        {
            let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp).map_err(|e| Error::io(tmp.display(), e))?;
            f.write_all(bytes).map_err(|e| Error::io(tmp.display(), e))?;
            f.sync_all().map_err(|e| Error::io(tmp.display(), e))?;
        }
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(Error::io(path.display(), e));
        }
        Ok(path)
    }
}

/// Read a file with a size cap, checking the length before allocating.
pub fn read_limited(path: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    let meta = fs::metadata(path).map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            Error::NotFound(display_path(path))
        } else {
            Error::io(path.display(), e)
        }
    })?;
    if !meta.is_file() {
        return Err(Error::Invalid(format!("{} is not a file", display_path(path))));
    }
    if meta.len() as usize > max_bytes {
        return Err(Error::Invalid(format!("{} is {} bytes, the limit is {max_bytes}", display_path(path), meta.len())));
    }
    fs::read(path).map_err(|e| Error::io(path.display(), e))
}

fn temp_name(target: &Path) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = target.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    target.with_file_name(format!(".{name}.tmp-{}-{n}", std::process::id()))
}

/// Create `path` with `bytes` only if it does not exist. Returns [`Error::Exists`] otherwise.
pub fn create_new_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = temp_name(path);
    {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp).map_err(|e| Error::io(tmp.display(), e))?;
        f.write_all(bytes).map_err(|e| Error::io(tmp.display(), e))?;
        f.sync_all().map_err(|e| Error::io(tmp.display(), e))?;
    }
    match fs::hard_link(&tmp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp);
            Err(Error::Exists(display_path(path)))
        }
        Err(_) => {
            // The filesystem has no hard links. Fall back to an exclusive create of the final name.
            let _ = fs::remove_file(&tmp);
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut f) => {
                    f.write_all(bytes).map_err(|e| Error::io(path.display(), e))?;
                    f.sync_all().map_err(|e| Error::io(path.display(), e))
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(Error::Exists(display_path(path))),
                Err(e) => Err(Error::io(path.display(), e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn fs_in(dir: &tempfile::TempDir) -> ProjectFs {
        ProjectFs::new(dir.path()).unwrap()
    }

    fn batch(rel: &str, bytes: &str, replace: bool) -> PendingWrite {
        PendingWrite { rel: rel.into(), bytes: bytes.as_bytes().to_vec(), replace }
    }

    fn leftovers(dir: &tempfile::TempDir) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![dir.path().to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if e.file_name().to_string_lossy().contains(".tmp-") || e.file_name().to_string_lossy().ends_with(".bak") {
                    out.push(p.display().to_string());
                }
            }
        }
        out
    }

    #[test]
    fn a_batch_writes_every_file_and_can_be_finished_or_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let pfs = fs_in(&dir);
        pfs.write_new("old/a.txt", b"before").unwrap();
        let b =
            pfs.write_batch(&[batch("new/one.txt", "1", false), batch("new/deep/two.txt", "2", false), batch("old/a.txt", "after", true)]).unwrap();
        assert_eq!(pfs.read("new/one.txt", 100).unwrap(), b"1");
        assert_eq!(pfs.read("new/deep/two.txt", 100).unwrap(), b"2");
        assert_eq!(pfs.read("old/a.txt", 100).unwrap(), b"after");
        b.rollback();
        assert!(
            !pfs.exists("new/one.txt") && !pfs.exists("new/deep/two.txt") && !pfs.exists("new"),
            "new files and the directories made for them are gone"
        );
        assert_eq!(pfs.read("old/a.txt", 100).unwrap(), b"before", "the replaced file is restored");
        assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));

        let b = pfs.write_batch(&[batch("new/one.txt", "1", false), batch("old/a.txt", "after", true)]).unwrap();
        b.finish();
        assert_eq!(pfs.read("old/a.txt", 100).unwrap(), b"after");
        assert!(leftovers(&dir).is_empty(), "backups are deleted when the batch is finished: {:?}", leftovers(&dir));
    }

    #[test]
    fn a_failure_while_moving_files_into_place_undoes_the_ones_already_moved() {
        let dir = tempfile::tempdir().unwrap();
        let pfs = fs_in(&dir);
        pfs.write_new("keep/a.txt", b"original").unwrap();
        // the third target exists as a directory, so a new file cannot be put there
        std::fs::create_dir_all(dir.path().join("blocked/c.txt")).unwrap();
        let err = pfs
            .write_batch(&[batch("fresh/a.txt", "1", false), batch("keep/a.txt", "changed", true), batch("blocked/c.txt", "3", false)])
            .unwrap_err();
        assert_eq!(err.code(), "already_exists");
        assert!(!pfs.exists("fresh/a.txt"), "the first file was removed again");
        assert_eq!(pfs.read("keep/a.txt", 100).unwrap(), b"original", "the replaced file was restored");
        assert!(dir.path().join("blocked/c.txt").is_dir());
        assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
    }

    #[test]
    fn a_failure_while_staging_leaves_nothing_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let pfs = fs_in(&dir);
        // `interface` is a file, so a file below it cannot be staged
        std::fs::write(dir.path().join("interface"), b"not a folder").unwrap();
        let err = pfs.write_batch(&[batch("gfx/x.dds", "1", false), batch("interface/x.gfx", "2", false)]).unwrap_err();
        assert!(matches!(err.code(), "io_error" | "path_rejected"), "{err}");
        assert!(!pfs.exists("gfx/x.dds") && !pfs.exists("gfx"), "nothing was written, not even the folder");
        assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
        assert!(pfs.write_batch(&[batch("a.txt", "1", false), batch("a.txt", "2", false)]).is_err(), "one target twice is refused up front");
    }

    #[test]
    fn lexical_rejections() {
        let bad = [
            "",
            "..",
            "../x",
            "a/../../x",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "C:/x",
            "c:\\x",
            "a/b:stream",
            "a/NUL",
            "a/con.txt",
            "a/b.",
            "a/b ",
            "a/PROGRA~1",
            "a/x*y",
            "a\0b",
            "a/<x>",
        ];
        for b in bad {
            assert!(check_relative(b).is_err(), "should reject {b:?}");
        }
        assert_eq!(check_relative("a\\b/./c.png").unwrap(), vec!["a", "b", "c.png"]);
        assert_eq!(check_relative("docs/assets/x/y-v1.png").unwrap().len(), 4);
    }

    #[test]
    fn resolve_stays_inside() {
        let dir = tempfile::tempdir().unwrap();
        let pfs = fs_in(&dir);
        let p = pfs.resolve("a/b/c.png").unwrap();
        assert!(p.starts_with(pfs.root()));
        assert!(pfs.resolve("../escape").is_err());
        assert_eq!(pfs.rel(&p).unwrap(), "a/b/c.png");
    }

    #[test]
    fn symlinked_directories_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = dir.path().join("linked");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(outside.path(), &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok();
        if !made {
            eprintln!("SKIPPED symlink_directories_are_refused: this account cannot create symlinks");
            return;
        }
        let pfs = fs_in(&dir);
        assert!(pfs.resolve("linked/x.png").is_err());
        assert!(pfs.write_new("linked/x.png", b"x").is_err());
        assert!(!outside.path().join("x.png").exists());
    }

    #[test]
    fn write_new_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let pfs = fs_in(&dir);
        pfs.write_new("out/a.png", b"one").unwrap();
        let err = pfs.write_new("out/a.png", b"two").unwrap_err();
        assert!(matches!(err, Error::Exists(_)));
        assert_eq!(pfs.read("out/a.png", 100).unwrap(), b"one");
        // no temp litter
        let names = pfs.list_names("out").unwrap();
        assert_eq!(names, vec!["a.png".to_string()], "{names:?}");
    }

    #[test]
    fn versions_count_up_and_respect_the_minimum() {
        let dir = tempfile::tempdir().unwrap();
        let pfs = fs_in(&dir);
        let a = pfs.write_next_version("src", "wall", "png", 0, b"a").unwrap();
        let b = pfs.write_next_version("src", "wall", "png", 0, b"b").unwrap();
        let c = pfs.write_next_version("src", "wall", "png", 7, b"c").unwrap();
        assert_eq!((a.version, b.version, c.version), (1, 2, 8));
        assert_eq!(a.rel, "src/wall-v1.png");
        assert_eq!(pfs.read(&b.rel, 10).unwrap(), b"b");
    }

    #[test]
    fn concurrent_writers_get_distinct_versions_and_keep_their_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let pfs = Arc::new(fs_in(&dir));
        let handles: Vec<_> = (0..24)
            .map(|i| {
                let pfs = Arc::clone(&pfs);
                std::thread::spawn(move || {
                    let body = format!("writer-{i}");
                    let w = pfs.write_next_version("race", "asset", "png", 0, body.as_bytes()).unwrap();
                    (w.version, w.rel, body)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let mut versions: Vec<u32> = results.iter().map(|r| r.0).collect();
        versions.sort_unstable();
        versions.dedup();
        assert_eq!(versions.len(), 24, "versions collided: {results:?}");
        for (_, rel, body) in &results {
            assert_eq!(pfs.read(rel, 100).unwrap(), body.as_bytes(), "{rel} lost its bytes");
        }
        let names = pfs.list_names("race").unwrap();
        assert_eq!(names.len(), 24, "temp files left behind: {names:?}");
    }

    #[test]
    fn replace_is_atomic_and_read_is_limited() {
        let dir = tempfile::tempdir().unwrap();
        let pfs = fs_in(&dir);
        pfs.replace("x/f.gfx", b"old").unwrap();
        pfs.replace("x/f.gfx", b"newer").unwrap();
        assert_eq!(pfs.read("x/f.gfx", 100).unwrap(), b"newer");
        assert!(pfs.read("x/f.gfx", 2).is_err());
        assert!(matches!(pfs.read("x/none.gfx", 10).unwrap_err(), Error::NotFound(_)));
    }
}
