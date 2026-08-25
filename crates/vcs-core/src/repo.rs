//! Repository discovery and reads (F3-2, F3-3).

use std::path::{Path, PathBuf};

use crate::error::VcsError;

/// A discovered, opened repository. Wraps `gix::Repository` rather than
/// re-exporting it, so a future `gix` major-version bump (or a swap to a
/// different backend) does not ripple past this crate's boundary — the same
/// reason `settings-model` wraps rather than leaks its dependents' types.
pub struct Repository {
    inner: gix::Repository,
}

/// The outcome of looking for a repository at or above a path.
///
/// Opening a plain folder with no `.git` is an entirely ordinary outcome —
/// most folders are not repositories — so it is a variant of a successful
/// result, not an `Err`. A caller that only cares whether Git features
/// should be offered at all can match this without touching `VcsError`.
pub enum DiscoverResult {
    Found(Box<Repository>),
    NotARepository,
}

impl Repository {
    /// Walk upward from `path` looking for a `.git`. Returns
    /// [`DiscoverResult::NotARepository`], not an error, when none is found
    /// before the filesystem root or a discovery ceiling.
    pub fn discover(path: impl AsRef<Path>) -> Result<DiscoverResult, VcsError> {
        use gix::discover::upwards::Error as UpwardsError;
        use gix::discover::Error as DiscoverError;

        match gix::discover(path.as_ref()) {
            Ok(inner) => Ok(DiscoverResult::Found(Box::new(Repository { inner }))),
            Err(DiscoverError::Discover(
                UpwardsError::NoGitRepository { .. }
                | UpwardsError::NoGitRepositoryWithinCeiling { .. }
                | UpwardsError::NoGitRepositoryWithinFs { .. },
            )) => Ok(DiscoverResult::NotARepository),
            Err(err) => Err(VcsError::Discover(err.to_string())),
        }
    }

    /// The repository's working tree root, if it is not bare.
    pub fn work_dir(&self) -> Option<PathBuf> {
        self.inner.workdir().map(Path::to_path_buf)
    }

    /// The `.git` directory itself.
    pub fn git_dir(&self) -> PathBuf {
        self.inner.git_dir().to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_repo(dir: &Path) {
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir)
            .status()
            .expect("git must be on PATH for these tests");
        assert!(status.success());
    }

    #[test]
    fn a_plain_folder_is_not_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let result = Repository::discover(dir.path()).unwrap();
        assert!(matches!(result, DiscoverResult::NotARepository));
    }

    #[test]
    fn a_git_init_ed_folder_is_found() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let result = Repository::discover(dir.path()).unwrap();
        assert!(matches!(result, DiscoverResult::Found(_)));
    }

    #[test]
    fn discovery_walks_upward_from_a_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let sub = dir.path().join("a/b/c");
        std::fs::create_dir_all(&sub).unwrap();
        let result = Repository::discover(&sub).unwrap();
        assert!(matches!(result, DiscoverResult::Found(_)));
    }

    #[test]
    fn work_dir_is_the_repository_root() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let Ok(DiscoverResult::Found(repo)) = Repository::discover(dir.path()) else {
            panic!("expected a repository");
        };
        // Canonicalize both sides: on macOS `TMPDIR` is under a symlink
        // (`/tmp` -> `/private/tmp`), and `gix` resolves it.
        assert_eq!(
            repo.work_dir().unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }
}
