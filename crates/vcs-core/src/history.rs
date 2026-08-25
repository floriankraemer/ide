//! Commit history: the full log via `gix`, and per-file history via a
//! walk-and-compare `gix` has no pathspec-filtered revwalk to do for us
//! (F3-10). Both cached, both off the hot path — a caller reaches for
//! either on demand (opening a history panel), never on a keystroke.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::VcsError;
use crate::repo::Repository;

/// One commit, as much as a history or blame view needs — not the full
/// `gix_object::Commit`, so a `gix` upgrade cannot change this crate's
/// public surface out from under a caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Full hex object id.
    pub id: String,
    /// The commit message's first line.
    pub summary: String,
    pub author_name: String,
    pub author_email: String,
    /// Seconds since the Unix epoch, author time (not committer time — the
    /// one a "when was this written" view wants).
    pub author_time: i64,
}

impl Repository {
    /// The commit history reachable from `HEAD`, newest first, via `gix`'s
    /// commit walk — no subprocess. `max` caps how many commits are
    /// returned (a history panel pages; nothing here needs the whole repo
    /// at once).
    pub fn log(&self, max: Option<usize>) -> Result<Vec<LogEntry>, VcsError> {
        let head_id = match self.inner.head_id() {
            Ok(id) => id,
            // Unborn HEAD: no commits exist yet. Not an error — the same
            // judgement F3-2's `DiscoverResult` made for "not a repository".
            Err(_) => return Ok(Vec::new()),
        };
        let ancestors = head_id
            .ancestors()
            .all()
            .map_err(|e| VcsError::Read(e.to_string()))?;

        let mut entries = Vec::new();
        for info in ancestors {
            let info = info.map_err(|e| VcsError::Read(e.to_string()))?;
            let commit = info.object().map_err(|e| VcsError::Read(e.to_string()))?;
            entries.push(log_entry(&commit)?);
            if max.is_some_and(|max| entries.len() >= max) {
                break;
            }
        }
        Ok(entries)
    }

    /// Commits that changed `relative_path`, newest first.
    ///
    /// `gix` 0.87 has no path-filtered revwalk (`rev_walk().selected(pred)`
    /// takes a commit-id predicate, not a pathspec — checked directly in
    /// `gix-0.87.1/src/revision/`), so this walks every ancestor and
    /// compares the path's tree entry against its first parent's, which is
    /// what "did this commit touch this path" reduces to without full
    /// rename tracking. A merge commit's non-first parents are not
    /// compared against — same simplification `git log --follow` avoids
    /// but `git log <path>` without `-m` makes by default.
    pub fn file_history(&self, relative_path: &Path) -> Result<Vec<LogEntry>, VcsError> {
        let head_id = match self.inner.head_id() {
            Ok(id) => id,
            Err(_) => return Ok(Vec::new()),
        };
        let ancestors = head_id
            .ancestors()
            .all()
            .map_err(|e| VcsError::Read(e.to_string()))?;

        let mut entries = Vec::new();
        for info in ancestors {
            let info = info.map_err(|e| VcsError::Read(e.to_string()))?;
            let commit = info.object().map_err(|e| VcsError::Read(e.to_string()))?;
            let entry_oid = tree_entry_oid(&commit, relative_path)?;

            let parent_oid = match info.parent_ids().next() {
                Some(parent_id) => {
                    let parent_commit = parent_id
                        .object()
                        .map_err(|e| VcsError::Read(e.to_string()))?
                        .into_commit();
                    tree_entry_oid(&parent_commit, relative_path)?
                }
                // A root commit has no parent to diff against: the path
                // counts as changed exactly when it exists there at all.
                None => None,
            };

            if entry_oid != parent_oid {
                entries.push(log_entry(&commit)?);
            }
        }
        Ok(entries)
    }
}

fn tree_entry_oid(
    commit: &gix::Commit<'_>,
    relative_path: &Path,
) -> Result<Option<gix::ObjectId>, VcsError> {
    let tree = commit.tree().map_err(|e| VcsError::Read(e.to_string()))?;
    Ok(tree
        .lookup_entry_by_path(relative_path)
        .map_err(|e| VcsError::Read(e.to_string()))?
        .map(|entry| entry.object_id()))
}

fn log_entry(commit: &gix::Commit<'_>) -> Result<LogEntry, VcsError> {
    let message = commit
        .message()
        .map_err(|e| VcsError::Read(e.to_string()))?;
    let author = commit.author().map_err(|e| VcsError::Read(e.to_string()))?;
    let time = author.time().map_err(|e| VcsError::Read(e.to_string()))?;
    Ok(LogEntry {
        id: commit.id.to_hex().to_string(),
        summary: message.summary().to_string(),
        author_name: author.name.to_string(),
        author_email: author.email.to_string(),
        author_time: time.seconds,
    })
}

/// Caches [`Repository::log`] and [`Repository::file_history`], keyed by
/// the repository's current `HEAD` commit (and, for file history, the
/// path). A history view is opened by the user, not driven by a keystroke,
/// so this exists to make re-opening it or switching between files cheap,
/// not to survive concurrent-edit races the way [`crate::hunks::HunkCache`]
/// must.
type LogKey = (String, Option<usize>);
type FileHistoryKey = (PathBuf, String);

#[derive(Default)]
pub struct HistoryCache {
    log: Mutex<HashMap<LogKey, Vec<LogEntry>>>,
    file_history: Mutex<HashMap<FileHistoryKey, Vec<LogEntry>>>,
}

impl HistoryCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn log(&self, repo: &Repository, max: Option<usize>) -> Result<Vec<LogEntry>, VcsError> {
        let key = (head_key(repo)?, max);
        if let Some(cached) = self.log.lock().unwrap().get(&key) {
            return Ok(cached.clone());
        }
        let entries = repo.log(max)?;
        self.log.lock().unwrap().insert(key, entries.clone());
        Ok(entries)
    }

    pub fn file_history(
        &self,
        repo: &Repository,
        relative_path: &Path,
    ) -> Result<Vec<LogEntry>, VcsError> {
        let key = (relative_path.to_path_buf(), head_key(repo)?);
        if let Some(cached) = self.file_history.lock().unwrap().get(&key) {
            return Ok(cached.clone());
        }
        let entries = repo.file_history(relative_path)?;
        self.file_history
            .lock()
            .unwrap()
            .insert(key, entries.clone());
        Ok(entries)
    }
}

/// The cache key for "current `HEAD`": the commit hex id, or a fixed string
/// for the unborn-HEAD case (no commit id exists yet to key on). Shared
/// with [`crate::blame::BlameCache`], which needs the identical key.
pub(crate) fn head_key(repo: &Repository) -> Result<String, VcsError> {
    match repo.inner.head_id() {
        Ok(id) => Ok(id.to_hex().to_string()),
        Err(_) => Ok("unborn".to_string()),
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
    fn log_on_an_unborn_repository_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        let repo = open(dir.path());
        assert!(repo.log(None).unwrap().is_empty());
    }

    #[test]
    fn log_lists_commits_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "first"]);
        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "second"]);

        let repo = open(dir.path());
        let log = repo.log(None).unwrap();
        assert_eq!(
            log.iter().map(|e| e.summary.as_str()).collect::<Vec<_>>(),
            vec!["second", "first"]
        );
        assert_eq!(log[0].author_email, "test@example.com");
    }

    #[test]
    fn log_respects_max() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        for n in 0..3 {
            std::fs::write(dir.path().join("a.txt"), format!("{n}\n")).unwrap();
            git(dir.path(), &["add", "a.txt"]);
            git(dir.path(), &["commit", "-m", &format!("commit {n}")]);
        }
        let repo = open(dir.path());
        assert_eq!(repo.log(Some(2)).unwrap().len(), 2);
    }

    #[test]
    fn file_history_only_lists_commits_that_touched_the_path() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "add a"]);

        std::fs::write(dir.path().join("b.txt"), "unrelated\n").unwrap();
        git(dir.path(), &["add", "b.txt"]);
        git(dir.path(), &["commit", "-m", "add b, unrelated to a"]);

        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "edit a"]);

        let repo = open(dir.path());
        let history = repo.file_history(Path::new("a.txt")).unwrap();
        assert_eq!(
            history
                .iter()
                .map(|e| e.summary.as_str())
                .collect::<Vec<_>>(),
            vec!["edit a", "add a"]
        );
    }

    #[test]
    fn history_cache_serves_a_second_call_without_recomputing_a_changed_key() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "first"]);

        let repo = open(dir.path());
        let cache = HistoryCache::new();
        let first = cache.log(&repo, None).unwrap();
        let second = cache.log(&repo, None).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 1);
    }
}
