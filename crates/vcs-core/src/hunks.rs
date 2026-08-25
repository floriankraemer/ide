//! Working-tree hunks against `HEAD`, and the cache that keeps a gutter
//! diff off the disk and off `gix` on every keystroke (F3-4).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use editor_core::diff::{self, DiffError, Hunk};

use crate::error::VcsError;
use crate::repo::Repository;

impl Repository {
    /// The `HEAD` blob for a repository-relative path, decoded as UTF-8, and
    /// its object id — or `None` if the path does not exist in `HEAD` (a new
    /// file, or a repository with no commits yet, which
    /// [`gix::Repository::head_tree_id_or_empty`] treats as an empty tree).
    ///
    /// A blob that is not valid UTF-8 is reported as [`VcsError::Read`]
    /// rather than lossily decoded — a diff against a mangled binary-as-text
    /// blob is worse than an explicit "can't diff this" the caller can
    /// distinguish from "no changes".
    pub fn head_blob(&self, relative_path: &Path) -> Result<Option<(String, String)>, VcsError> {
        let tree_id = self
            .inner
            .head_tree_id_or_empty()
            .map_err(|e| VcsError::Read(e.to_string()))?;
        let tree = self
            .inner
            .find_tree(tree_id)
            .map_err(|e| VcsError::Read(e.to_string()))?;
        let Some(entry) = tree
            .lookup_entry_by_path(relative_path)
            .map_err(|e| VcsError::Read(e.to_string()))?
        else {
            return Ok(None);
        };
        let object = entry.object().map_err(|e| VcsError::Read(e.to_string()))?;
        let oid = object.id.to_hex().to_string();
        let text = String::from_utf8(object.data.clone()).map_err(|_| {
            VcsError::Read(format!("{} is not valid UTF-8", relative_path.display()))
        })?;
        Ok(Some((oid, text)))
    }
}

/// Working-tree hunks for one file: `HEAD`'s blob id (`"none"` for a file
/// `HEAD` does not have) and the line hunks between it and the working text
/// handed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingHunks {
    pub head_oid: String,
    pub hunks: Vec<Hunk>,
}

/// The blob id used as the cache key (and returned to the caller) when
/// `HEAD` has no version of the file at all.
pub const NO_HEAD_BLOB: &str = "none";

struct CacheEntry {
    head_oid: String,
    revision: u64,
    hunks: Vec<Hunk>,
}

/// Caches [`WorkingHunks`] per path, invalidated by `(head_oid, revision)`.
///
/// `revision` is the caller's: this crate has no idea about the live buffer
/// (`QTextDocument` on the other side of the seam), so the caller supplies
/// whatever it can cheaply bump on every edit — a monotonic counter is
/// enough, since the cache only needs to tell "same edit state" from "not",
/// never to reconstruct history.
#[derive(Default)]
pub struct HunkCache {
    entries: Mutex<HashMap<PathBuf, CacheEntry>>,
}

impl HunkCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hunks between `HEAD`'s copy of `path` and `working_text`, from cache
    /// when `head_oid`/`revision` still match what produced the cached
    /// value, computed and stored otherwise.
    ///
    /// Errs [`VcsError::TooLargeToDiff`] rather than serving or caching a
    /// diff for a file past `editor_core::diff::MAX_DIFF_BYTES` — a gutter
    /// with no markers on a huge file reads as "nothing changed", which is
    /// exactly the failure mode `editor_core::diff` was built to refuse.
    pub fn hunks(
        &self,
        repo: &Repository,
        path: &Path,
        working_text: &str,
        revision: u64,
    ) -> Result<WorkingHunks, VcsError> {
        let head = repo.head_blob(path)?;
        let (head_oid, before) = match head {
            Some((oid, text)) => (oid, text),
            None => (NO_HEAD_BLOB.to_string(), String::new()),
        };

        {
            let cache = self.entries.lock().unwrap();
            if let Some(entry) = cache.get(path) {
                if entry.head_oid == head_oid && entry.revision == revision {
                    return Ok(WorkingHunks {
                        head_oid,
                        hunks: entry.hunks.clone(),
                    });
                }
            }
        }

        let hunks = diff::diff_lines(&before, working_text).map_err(|e| match e {
            DiffError::TooLarge => VcsError::TooLargeToDiff,
        })?;

        self.entries.lock().unwrap().insert(
            path.to_path_buf(),
            CacheEntry {
                head_oid: head_oid.clone(),
                revision,
                hunks: hunks.clone(),
            },
        );

        Ok(WorkingHunks { head_oid, hunks })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::DiscoverResult;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn open(dir: &Path) -> Repository {
        match Repository::discover(dir).unwrap() {
            DiscoverResult::Found(repo) => *repo,
            DiscoverResult::NotARepository => panic!("expected a repository"),
        }
    }

    #[test]
    fn a_file_with_no_head_version_diffs_against_empty() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        let repo = open(dir.path());
        let cache = HunkCache::new();
        let result = cache
            .hunks(&repo, Path::new("new.txt"), "one\ntwo\n", 0)
            .unwrap();
        assert_eq!(result.head_oid, NO_HEAD_BLOB);
        assert_eq!(result.hunks.len(), 1);
    }

    #[test]
    fn hunks_reflect_a_modification_against_head() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "first"]);

        let repo = open(dir.path());
        let cache = HunkCache::new();
        let result = cache
            .hunks(&repo, Path::new("a.txt"), "one\nTWO\n", 0)
            .unwrap();
        assert_ne!(result.head_oid, NO_HEAD_BLOB);
        assert_eq!(result.hunks.len(), 1);
    }

    #[test]
    fn a_stale_revision_is_recomputed_not_served_from_cache() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "first"]);

        let repo = open(dir.path());
        let cache = HunkCache::new();
        let first = cache
            .hunks(&repo, Path::new("a.txt"), "one\ntwo\n", 0)
            .unwrap();
        assert_eq!(first.hunks.len(), 1);

        // Same path, bumped revision, different working text: must not
        // reuse the stale cached hunks from revision 0.
        let second = cache
            .hunks(&repo, Path::new("a.txt"), "one\ntwo\nthree\n", 1)
            .unwrap();
        assert_eq!(second.hunks.len(), 1);
        assert_eq!(second.hunks[0].new, 1..3);
    }

    #[test]
    fn a_repeated_call_with_the_same_revision_is_served_from_cache() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "first"]);

        let repo = open(dir.path());
        let cache = HunkCache::new();
        let first = cache
            .hunks(&repo, Path::new("a.txt"), "one\ntwo\n", 7)
            .unwrap();
        // Pass working text that would diff differently, but keep the same
        // revision: the cached value from revision 7 must still come back,
        // proving the cache — not just a correct recompute — is exercised.
        let second = cache
            .hunks(&repo, Path::new("a.txt"), "completely different", 7)
            .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn a_file_past_the_diff_ceiling_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        let repo = open(dir.path());
        let cache = HunkCache::new();
        let huge = "x\n".repeat(diff::MAX_DIFF_BYTES);
        let err = cache
            .hunks(&repo, Path::new("huge.txt"), &huge, 0)
            .unwrap_err();
        assert!(matches!(err, VcsError::TooLargeToDiff));
    }
}
