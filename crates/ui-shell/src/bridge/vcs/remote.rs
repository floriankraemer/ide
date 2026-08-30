//! Branches, remotes, history and blame (F3-12d).
//!
//! No UI consumer in this chunk — the Changes dock, branch widget and
//! history/blame panels are F3-17/18, blocked on F0-7's dock registry — the
//! same situation `vcs-core` itself landed a whole chunk earlier with zero
//! bridge. Translation only, like every other method in this module: what a
//! branch or a commit *is* stays `vcs-core`'s.

use core::pin::Pin;
use std::path::Path;

use cxx_qt::Threading;
use cxx_qt_lib::QString;

use crate::bridge::ffi;

use super::{to_ffi_result, VcsWorker};

fn to_ffi_log_entry(entry: &vcs_core::LogEntry) -> ffi::FfiLogEntry {
    ffi::FfiLogEntry {
        id: QString::from(entry.id.as_str()),
        summary: QString::from(entry.summary.as_str()),
        author_name: QString::from(entry.author_name.as_str()),
        author_email: QString::from(entry.author_email.as_str()),
        author_time: entry.author_time,
    }
}

fn to_ffi_blame_line(line: &vcs_core::BlameLine) -> ffi::FfiBlameLine {
    ffi::FfiBlameLine {
        line: line.line as u32,
        commit: QString::from(line.commit.as_str()),
        author_name: QString::from(line.author_name.as_str()),
        author_email: QString::from(line.author_email.as_str()),
        summary: QString::from(line.summary.as_str()),
        content: QString::from(line.content.as_str()),
    }
}

impl ffi::VcsService {
    pub fn refresh_branches(mut self: Pin<&mut Self>) {
        let qt_thread = self.as_mut().qt_thread();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            // One round trip for both: a caller asking for the branch list
            // almost always wants to know which one is current too (the
            // branch widget and menu both do), and `current_branch` is a
            // cheap `HEAD` read next to a ref listing.
            let names = worker.repo.branches();
            let current = worker.repo.current_branch();
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match (names, current) {
                (Ok(names), Ok(current)) => {
                    *service.branches.borrow_mut() = names;
                    *service.current_branch.borrow_mut() = current.unwrap_or_default();
                    service.as_mut().branch_changed();
                }
                (Err(err), _) | (_, Err(err)) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn branches(&self) -> Vec<ffi::FfiBranch> {
        self.branches
            .borrow()
            .iter()
            .map(|name| ffi::FfiBranch {
                name: QString::from(name.as_str()),
            })
            .collect()
    }

    pub fn current_branch(&self) -> QString {
        QString::from(self.current_branch.borrow().as_str())
    }

    pub fn checkout(mut self: Pin<&mut Self>, name: &QString) {
        let name = name.to_string();
        let qt_thread = self.as_mut().qt_thread();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            let result = worker.repo.checkout(&name);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match result {
                Ok(()) => {
                    service.as_mut().branch_changed();
                    service.as_mut().refresh_status();
                }
                Err(err) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn create_branch(mut self: Pin<&mut Self>, name: &QString, start_point: &QString) {
        let name = name.to_string();
        let start_point = start_point.to_string();
        let qt_thread = self.as_mut().qt_thread();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            let start = if start_point.is_empty() {
                None
            } else {
                Some(start_point.as_str())
            };
            let result = worker.repo.create_branch(&name, start);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match result {
                Ok(()) => service.as_mut().branch_changed(),
                Err(err) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn delete_branch(mut self: Pin<&mut Self>, name: &QString, force: bool) {
        let name = name.to_string();
        let qt_thread = self.as_mut().qt_thread();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            let result = worker.repo.delete_branch(&name, force);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match result {
                Ok(()) => service.as_mut().branch_changed(),
                Err(err) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn fetch(mut self: Pin<&mut Self>, remote: &QString) {
        let remote = remote.to_string();
        let qt_thread = self.as_mut().qt_thread();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            let result = worker.repo.fetch(&remote);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                if let Err(err) = result {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn pull(mut self: Pin<&mut Self>, remote: &QString, branch: &QString) {
        let remote = remote.to_string();
        let branch = branch.to_string();
        let qt_thread = self.as_mut().qt_thread();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            let result = worker.repo.pull(&remote, &branch);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match result {
                Ok(()) => {
                    service.as_mut().refresh_status();
                    service.as_mut().branch_changed();
                }
                Err(err) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn push(mut self: Pin<&mut Self>, remote: &QString, branch: &QString, set_upstream: bool) {
        let remote = remote.to_string();
        let branch = branch.to_string();
        let qt_thread = self.as_mut().qt_thread();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            let result = worker.repo.push(&remote, &branch, set_upstream);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match result {
                Ok(()) => service.as_mut().branch_changed(),
                Err(err) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn file_history(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        let qt_thread = self.as_mut().qt_thread();
        let signal_path = path.clone();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            // `path` is the tab's own path (`DocumentManager::tabPath`),
            // absolute — `Repository::file_history` looks a path up in each
            // commit's tree, which only ever holds repository-relative
            // paths, the same fix `requestHunks` already needs.
            let absolute = Path::new(&path);
            let relative = match worker.repo.work_dir() {
                Some(root) => absolute.strip_prefix(&root).unwrap_or(absolute),
                None => absolute,
            };
            let result = worker.history_cache.file_history(&worker.repo, relative);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match result {
                Ok(entries) => {
                    let entries: Vec<ffi::FfiLogEntry> =
                        entries.iter().map(to_ffi_log_entry).collect();
                    service
                        .as_mut()
                        .history_ready(QString::from(signal_path.as_str()), entries);
                }
                Err(err) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }

    pub fn blame(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        let qt_thread = self.as_mut().qt_thread();
        let signal_path = path.clone();
        self.as_ref().push_job(move |worker: &VcsWorker| {
            // Same absolute-vs-repository-relative fix `file_history` and
            // `requestHunks` both need.
            let absolute = Path::new(&path);
            let relative = match worker.repo.work_dir() {
                Some(root) => absolute.strip_prefix(&root).unwrap_or(absolute),
                None => absolute,
            };
            let result = worker.blame_cache.blame(&worker.repo, relative);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match result {
                Ok(lines) => {
                    let lines: Vec<ffi::FfiBlameLine> =
                        lines.iter().map(to_ffi_blame_line).collect();
                    service
                        .as_mut()
                        .blame_ready(QString::from(signal_path.as_str()), lines);
                }
                Err(err) => {
                    let result = to_ffi_result(&err);
                    service.as_mut().vcs_failed(result);
                }
            });
        });
    }
}
