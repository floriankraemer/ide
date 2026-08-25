use core::pin::Pin;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::Sender;

use cxx_qt::Threading;
use cxx_qt_lib::QString;

use crate::bridge::ffi;

/// Branches, remotes, history and blame (F3-12d) — no UI consumer in this
/// chunk (Changes dock / branch widget / history panel are F3-17/18, blocked
/// on F0-7), same situation `vcs-core` itself landed a whole chunk earlier.
mod remote;
/// Git v1 (F3-12): staging and commit, split out once this file crossed the
/// ceiling, the way `language/lsp_surface.rs` splits out of
/// `language/mod.rs`.
mod staging;

/// One unit of work for the worker thread that owns the `vcs_core::Repository`
/// handle. Mirrors `LanguageService`'s `LspJob`: a repository handle plus a
/// `git` subprocess call must never run on the UI thread, and running every
/// call through one queue keeps ordering honest (a `stageHunk` queued while a
/// `refreshStatus` is still running is still applied after it).
type VcsJob = Box<dyn FnOnce(&VcsWorker) + Send>;

/// What the worker thread owns: the repository handle plus the caches
/// `vcs-core` already built for hunks, history and blame. Never touched from
/// the Qt thread — every field here is reached only from inside a `VcsJob`.
pub(crate) struct VcsWorker {
    repo: vcs_core::Repository,
    hunk_cache: vcs_core::HunkCache,
    history_cache: vcs_core::HistoryCache,
    blame_cache: vcs_core::BlameCache,
}

/// The `HEAD` text and hunks a `requestHunks` call last found for one path,
/// plus the working text they were diffed against — enough to revert a hunk
/// with no worker round trip (`vcs_core::revert_hunk` is a pure function) and
/// to stage/unstage one against the same two texts it was computed from.
#[derive(Clone)]
struct CachedHunks {
    before_text: String,
    working_text: String,
    hunks: Vec<editor_core::diff::Hunk>,
}

/// Rust side of the `VcsService` QObject: a handle to the worker, and
/// whatever its last answers were. No rules — see the bridge declaration for
/// what stays out (ADR-0002).
pub struct VcsServiceRust {
    jobs: RefCell<Option<Sender<VcsJob>>>,
    is_repository: Cell<bool>,
    status: RefCell<vcs_core::RepoStatus>,
    hunks: RefCell<HashMap<String, CachedHunks>>,
    branches: RefCell<Vec<String>>,
}

impl Default for VcsServiceRust {
    fn default() -> Self {
        VcsServiceRust {
            jobs: RefCell::default(),
            is_repository: Cell::new(false),
            status: RefCell::default(),
            hunks: RefCell::default(),
            branches: RefCell::default(),
        }
    }
}

/// `err`, as the typed code + message that crosses the FFI seam (ADR-0003).
/// `vcs-core`'s own codes (700-799, `error.rs`) are already stable, so this
/// is a field copy, not a translation.
fn to_ffi_result(err: &vcs_core::VcsError) -> ffi::FfiResult {
    ffi::FfiResult {
        code: err.code(),
        message: QString::from(err.to_string().as_str()),
    }
}

fn to_ffi_change_kind(kind: Option<vcs_core::ChangeKind>) -> ffi::FfiChangeKind {
    match kind {
        None => ffi::FfiChangeKind::None,
        Some(vcs_core::ChangeKind::Added) => ffi::FfiChangeKind::Added,
        Some(vcs_core::ChangeKind::Modified) => ffi::FfiChangeKind::Modified,
        Some(vcs_core::ChangeKind::Deleted) => ffi::FfiChangeKind::Deleted,
        Some(vcs_core::ChangeKind::TypeChanged) => ffi::FfiChangeKind::TypeChanged,
    }
}

impl ffi::VcsService {
    pub fn open_project(mut self: Pin<&mut Self>, root_path: &QString) {
        let root = root_path.to_string();

        // Dropping the previous sender ends that worker's loop — no separate
        // stop path to keep in sync, same shutdown `LanguageService` uses.
        self.jobs.borrow_mut().take();
        self.hunks.borrow_mut().clear();
        self.branches.borrow_mut().clear();
        *self.status.borrow_mut() = vcs_core::RepoStatus::default();
        self.is_repository.set(false);

        if root.is_empty() {
            self.as_mut().repository_changed();
            return;
        }

        let (jobs, rx) = std::sync::mpsc::channel::<VcsJob>();
        let qt_thread = self.as_mut().qt_thread();
        std::thread::spawn(move || match vcs_core::Repository::discover(&root) {
            Ok(vcs_core::DiscoverResult::Found(repo)) => {
                let worker = VcsWorker {
                    repo: *repo,
                    hunk_cache: vcs_core::HunkCache::new(),
                    history_cache: vcs_core::HistoryCache::new(),
                    blame_cache: vcs_core::BlameCache::new(),
                };
                let _ = qt_thread.queue(|mut service: Pin<&mut Self>| {
                    service.is_repository.set(true);
                    service.as_mut().repository_changed();
                });
                // Ends when the sender above is dropped (project closed, or
                // the app is going away) — no children to kill: every write
                // here is a `git` subprocess that already finished by the
                // time its job's closure returns.
                for job in rx {
                    job(&worker);
                }
            }
            Ok(vcs_core::DiscoverResult::NotARepository) => {
                let _ = qt_thread.queue(|mut service: Pin<&mut Self>| {
                    service.is_repository.set(false);
                    service.as_mut().repository_changed();
                });
            }
            Err(err) => {
                let result = to_ffi_result(&err);
                let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                    service.is_repository.set(false);
                    service.as_mut().repository_changed();
                    service.as_mut().vcs_failed(result);
                });
            }
        });
        *self.jobs.borrow_mut() = Some(jobs);
    }

    pub fn is_repository(&self) -> bool {
        self.is_repository.get()
    }

    pub fn refresh_status(mut self: Pin<&mut Self>) {
        let qt_thread = self.as_mut().qt_thread();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            let result = worker.repo.status();
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match result {
                Ok(status) => {
                    *service.status.borrow_mut() = status;
                    service.as_mut().status_changed();
                }
                Err(err) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn changed_files(&self) -> Vec<ffi::FfiChangedFile> {
        let status = self.status.borrow();
        let mut out: Vec<ffi::FfiChangedFile> = status
            .files
            .iter()
            .map(|file| ffi::FfiChangedFile {
                path: QString::from(file.path.to_string_lossy().as_ref()),
                staged: to_ffi_change_kind(file.staged),
                unstaged: to_ffi_change_kind(file.unstaged),
            })
            .collect();
        out.extend(status.untracked.iter().map(|path| ffi::FfiChangedFile {
            path: QString::from(path.to_string_lossy().as_ref()),
            staged: ffi::FfiChangeKind::None,
            unstaged: ffi::FfiChangeKind::Untracked,
        }));
        out
    }

    pub fn request_hunks(
        mut self: Pin<&mut Self>,
        path: &QString,
        working_text: &QString,
        revision: i64,
    ) {
        let path = path.to_string();
        let working_text = working_text.to_string();
        // Negative is not a revision a caller sends deliberately; treated as
        // 0 rather than panicking on the cast.
        let revision = revision.max(0) as u64;
        let qt_thread = self.as_mut().qt_thread();
        let job_path = path.clone();
        let job_text = working_text.clone();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            let relative = Path::new(&job_path);
            let head = worker.repo.head_blob(relative);
            let hunks = worker
                .hunk_cache
                .hunks(&worker.repo, relative, &job_text, revision);
            let outcome = match (head, hunks) {
                (Ok(head), Ok(working_hunks)) => {
                    let before_text = head.map(|(_, text)| text).unwrap_or_default();
                    Ok((before_text, working_hunks.hunks))
                }
                (Err(err), _) | (_, Err(err)) => Err(err),
            };
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match outcome {
                Ok((before_text, hunks)) => {
                    service.hunks.borrow_mut().insert(
                        job_path.clone(),
                        CachedHunks {
                            before_text,
                            working_text: job_text,
                            hunks,
                        },
                    );
                    service
                        .as_mut()
                        .hunks_changed(QString::from(job_path.as_str()));
                }
                Err(err) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn hunks(&self, path: &QString) -> Vec<ffi::FfiHunk> {
        match self.hunks.borrow().get(&path.to_string()) {
            Some(cached) => crate::bridge::convert::to_ffi_hunks(&cached.hunks),
            None => Vec::new(),
        }
    }

    pub fn revert_hunk(&self, path: &QString, hunk_index: u32) -> Vec<ffi::FfiTextEdit> {
        let path = path.to_string();
        let cache = self.hunks.borrow();
        let Some(cached) = cache.get(&path) else {
            return Vec::new();
        };
        let Some(hunk) = cached.hunks.get(hunk_index as usize) else {
            return Vec::new();
        };
        let edit = vcs_core::revert_hunk(&cached.before_text, hunk);
        vec![ffi::FfiTextEdit {
            path: QString::from(path.as_str()),
            in_buffer: true,
            start_line: edit.start_line as u32,
            start_character: 0,
            end_line: edit.end_line as u32,
            end_character: 0,
            new_text: QString::from(edit.new_text.as_str()),
        }]
    }

    /// Queue work for the worker thread. Returns false when there is no
    /// worker (no project open, or the project is not a repository) — the
    /// same "no worker yet" shape `LanguageService::push_job` has.
    pub(crate) fn push_job(&self, job: impl FnOnce(&VcsWorker) + Send + 'static) -> bool {
        match self.jobs.borrow().as_ref() {
            Some(jobs) => jobs.send(Box::new(job)).is_ok(),
            None => false,
        }
    }
}
