//! `notify`-based filesystem watcher (Task 8, mvp-implementation-plan.md
//! §2). One instance watches one project root recursively on a background
//! thread; dropping it stops watching. No Qt dependency — the callback is
//! a plain `Fn(PathBuf)`, so `ui-shell` supplies a closure that queues work
//! onto the Qt thread via `cxx-qt`'s `CxxQtThread`. This crate never touches
//! Qt or knows what the callback does with the path.

use std::path::{Path, PathBuf};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};

// Re-exported so downstream crates (`app-core`, `ui-shell`) can name the
// event kind in watcher callbacks without depending on `notify` themselves —
// the watcher backend stays this crate's implementation detail.
pub use notify::EventKind;

/// Whether a filesystem-watcher event actually shifts the project tree's
/// shape (a file/folder created, removed, or renamed) as opposed to a
/// content-only change to a path that's already in the tree (a plain write,
/// e.g. every `Ctrl+S` save — same rows, same structure, just different
/// bytes on disk). Only the former needs the tree model rebuilt and reset;
/// resetting for the latter is exactly what caused the sidebar to
/// re-collapse on every save (`beginResetModel`/`endResetModel` discards
/// Qt's per-item expand state for the whole tree).
pub fn is_structural_change(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(notify::event::ModifyKind::Name(_))
    )
}

/// A running watcher for one project root. Kept alive for as long as the
/// project is open; dropping it (e.g. when a different project is opened)
/// stops watching — `notify`'s `Drop` impl tears down the OS-level watch.
pub struct ProjectWatcher {
    // Never read directly — its only job is to stay alive so the OS-level
    // watch it owns keeps running until this is dropped.
    _watcher: RecommendedWatcher,
}

impl ProjectWatcher {
    /// Start watching `root` recursively. `on_change` is invoked (on
    /// `notify`'s background thread, not the caller's thread) once per
    /// changed path reported by an event, along with the event's
    /// `EventKind` — callers need this to tell a structural change (file
    /// created/removed/renamed, which shifts the tree) apart from a
    /// content-only write to a file that already exists (which doesn't, and
    /// notably is what every `Ctrl+S` save looks like). Callers needing to
    /// touch Qt objects must marshal onto the Qt thread themselves
    /// (`ui-shell`'s job, not this crate's).
    pub fn start(
        root: &Path,
        on_change: impl Fn(EventKind, PathBuf) + Send + 'static,
    ) -> notify::Result<Self> {
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                for path in event.paths {
                    on_change(event.kind, path);
                }
            }
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self { _watcher: watcher })
    }
}

#[cfg(test)]
mod is_structural_change_tests {
    use super::is_structural_change;
    use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
    use notify::EventKind;

    #[test]
    fn create_and_remove_are_structural() {
        assert!(is_structural_change(&EventKind::Create(CreateKind::File)));
        assert!(is_structural_change(&EventKind::Remove(RemoveKind::File)));
    }

    #[test]
    fn a_rename_is_structural() {
        assert!(is_structural_change(&EventKind::Modify(ModifyKind::Name(
            RenameMode::Both
        ))));
    }

    #[test]
    fn a_plain_content_write_is_not_structural() {
        // What `fs::write` on an already-existing file (every save)
        // reports under Linux's inotify backend.
        assert!(!is_structural_change(&EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Any
        ))));
    }

    #[test]
    fn metadata_only_changes_are_not_structural() {
        assert!(!is_structural_change(&EventKind::Modify(
            ModifyKind::Metadata(notify::event::MetadataKind::Any)
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;
    use std::time::Duration;

    // `notify`'s OS-level event delivery is inherently timing-dependent, so
    // this polls with a short timeout rather than asserting on a fixed
    // sleep — tolerant of the raciness, per the task's own guidance.
    #[test]
    fn detects_a_new_file_under_the_watched_root() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel::<PathBuf>();

        let _watcher = ProjectWatcher::start(dir.path(), move |_kind, path| {
            let _ = tx.send(path);
        })
        .unwrap();

        let new_file = dir.path().join("new.txt");
        fs::write(&new_file, "hello").unwrap();

        let mut saw_it = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(path) = rx.recv_timeout(Duration::from_millis(200)) {
                if path == new_file {
                    saw_it = true;
                    break;
                }
            }
        }
        assert!(saw_it, "expected a watcher event for the new file");
    }

    // Regression test for the bug where saving a file (Ctrl+S) collapsed
    // the whole sidebar tree: the watcher callback carries `EventKind`
    // precisely so a caller can tell a content-only write to an
    // already-existing file (what every save looks like) apart from a
    // structural change (create/remove/rename) that actually shifts the
    // tree. This asserts the watcher reports a non-structural `EventKind`
    // for a plain rewrite of an existing file's contents.
    #[test]
    fn rewriting_an_existing_files_contents_is_not_reported_as_a_structural_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("existing.txt");
        fs::write(&file, "original").unwrap();

        let (tx, rx) = mpsc::channel::<EventKind>();
        let _watcher = ProjectWatcher::start(dir.path(), move |kind, path| {
            if path == file {
                let _ = tx.send(kind);
            }
        })
        .unwrap();

        let watched_file = dir.path().join("existing.txt");
        fs::write(&watched_file, "changed content, same path").unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut saw_structural = false;
        let mut saw_any = false;
        while std::time::Instant::now() < deadline {
            if let Ok(kind) = rx.recv_timeout(Duration::from_millis(200)) {
                saw_any = true;
                let structural = matches!(
                    kind,
                    EventKind::Create(_)
                        | EventKind::Remove(_)
                        | EventKind::Modify(notify::event::ModifyKind::Name(_))
                );
                if structural {
                    saw_structural = true;
                }
            }
        }
        assert!(
            saw_any,
            "expected at least one watcher event for the rewrite"
        );
        assert!(
            !saw_structural,
            "a content-only rewrite of an existing file must not be reported as structural"
        );
    }
}
