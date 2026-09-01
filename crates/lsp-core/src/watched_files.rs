//! `workspace/didChangeWatchedFiles`: whether a changed file on disk is one
//! a server asked this client to tell it about, per `client/registerCapability`
//! (C4's `Registrations::watchers`).
//!
//! Compiling a `GlobSet` per file-change event would be wasteful; a
//! server's watchers are compiled once, into [`WatchedFiles`], whenever its
//! registrations change — registration is rare, so a plain recompile on
//! register/unregister is simple enough with no need for the reload-on-swap
//! pattern `syntax_core::registry` uses for its own, much hotter, reload
//! path.

use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde_json::Value;

use crate::registration::Watcher;

/// LSP's `FileChangeType`: what kind of change happened to a watched file,
/// as sent in `workspace/didChangeWatchedFiles`'s `changes` array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeKind {
    Created = 1,
    Changed = 2,
    Deleted = 3,
}

impl FileChangeKind {
    /// This kind's bit in a `WatchKind` bitmask (create=1, change=2,
    /// delete=4 per the LSP spec) — deliberately not the same numbering as
    /// `FileChangeType`'s wire values above.
    fn watch_kind_bit(self) -> u32 {
        match self {
            FileChangeKind::Created => 1,
            FileChangeKind::Changed => 2,
            FileChangeKind::Deleted => 4,
        }
    }
}

impl From<notify::EventKind> for FileChangeKind {
    /// Reuses `notify`'s own classification rather than re-detecting
    /// anything — `project_model::watcher::is_structural_change` already
    /// draws the create/remove/rename-vs-content-write line this maps onto.
    /// A rename arrives from `notify` as one event per path with no
    /// reliable way to tell "the old name" from "the new name" apart (and
    /// `ProjectWatcher::start` does not preserve the paths' order), so it
    /// falls back to `Changed` — a server that cares is still told the file
    /// moved, just not as a clean delete-then-create pair.
    fn from(kind: notify::EventKind) -> Self {
        use notify::EventKind as NotifyKind;
        match kind {
            NotifyKind::Create(_) => FileChangeKind::Created,
            NotifyKind::Remove(_) => FileChangeKind::Deleted,
            _ => FileChangeKind::Changed,
        }
    }
}

/// One server's watched-file registrations, compiled once into a matchable
/// form.
pub struct WatchedFiles {
    set: GlobSet,
    kinds: Vec<Option<u32>>,
}

impl WatchedFiles {
    /// Compile a server's current `watchers()` (`Registrations::watchers`)
    /// into a `GlobSet`. A watcher whose `globPattern` this client can't
    /// read as a plain string — a `RelativePattern` object, which this
    /// client's `relativePatternSupport: false` tells servers not to send —
    /// is skipped rather than failing the whole compile; likewise an
    /// unparsable glob.
    pub fn compile(watchers: &[Watcher]) -> Self {
        let mut builder = GlobSetBuilder::new();
        let mut kinds = Vec::with_capacity(watchers.len());
        for watcher in watchers {
            let Some(pattern) = glob_pattern_str(&watcher.glob_pattern) else {
                continue;
            };
            let Ok(glob) = Glob::new(pattern) else {
                continue;
            };
            builder.add(glob);
            kinds.push(watcher.kind);
        }
        // Only fails on an internal regex-set build error, which an empty
        // or successfully-added set of globs never triggers.
        let set = builder.build().expect("compiled globs are always valid");
        WatchedFiles { set, kinds }
    }

    /// An empty set of watchers — what a server that registered nothing is
    /// interested in.
    pub fn none() -> Self {
        WatchedFiles {
            set: GlobSetBuilder::new().build().expect("empty glob set"),
            kinds: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// Whether `path`, changed in `kind`'s way, matches any registered
    /// watcher — and, if that watcher named a `kind` bitmask, that the
    /// bitmask includes this change. No `kind` on a watcher means all
    /// three, per spec default.
    pub fn interested(&self, path: &Path, kind: FileChangeKind) -> bool {
        self.set
            .matches(path)
            .into_iter()
            .any(|i| self.kinds[i].is_none_or(|mask| mask & kind.watch_kind_bit() != 0))
    }
}

fn glob_pattern_str(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("pattern").and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registration::Watcher;
    use serde_json::json;
    use std::path::Path;

    fn watcher(pattern: &str, kind: Option<u32>) -> Watcher {
        Watcher {
            glob_pattern: json!(pattern),
            kind,
        }
    }

    #[test]
    fn brace_glob_matches_named_extensions_only() {
        let watched = WatchedFiles::compile(&[watcher("**/*.{cs,csproj,sln}", None)]);
        assert!(watched.interested(Path::new("src/Program.cs"), FileChangeKind::Changed));
        assert!(watched.interested(Path::new("Foo.csproj"), FileChangeKind::Created));
        assert!(watched.interested(Path::new("x.sln"), FileChangeKind::Deleted));
        assert!(!watched.interested(Path::new("readme.md"), FileChangeKind::Changed));
    }

    #[test]
    fn kind_mask_restricts_to_the_kinds_it_claims() {
        // Create only (bit 1).
        let watched = WatchedFiles::compile(&[watcher("**/*.rs", Some(1))]);
        assert!(watched.interested(Path::new("a.rs"), FileChangeKind::Created));
        assert!(!watched.interested(Path::new("a.rs"), FileChangeKind::Changed));
        assert!(!watched.interested(Path::new("a.rs"), FileChangeKind::Deleted));
    }

    #[test]
    fn absent_kind_means_all_three() {
        let watched = WatchedFiles::compile(&[watcher("**/*.rs", None)]);
        assert!(watched.interested(Path::new("a.rs"), FileChangeKind::Created));
        assert!(watched.interested(Path::new("a.rs"), FileChangeKind::Changed));
        assert!(watched.interested(Path::new("a.rs"), FileChangeKind::Deleted));
    }

    #[test]
    fn a_relative_pattern_object_is_read_by_its_pattern_field() {
        let watched = WatchedFiles::compile(&[Watcher {
            glob_pattern: json!({"baseUri": "file:///x", "pattern": "*.cs"}),
            kind: None,
        }]);
        assert!(watched.interested(Path::new("a.cs"), FileChangeKind::Changed));
    }

    #[test]
    fn no_watchers_means_nothing_matches() {
        let watched = WatchedFiles::none();
        assert!(watched.is_empty());
        assert!(!watched.interested(Path::new("a.rs"), FileChangeKind::Changed));
    }

    #[test]
    fn notify_event_kinds_map_onto_lsp_file_change_kinds() {
        use notify::event::{CreateKind, ModifyKind, RemoveKind};
        assert_eq!(
            FileChangeKind::from(notify::EventKind::Create(CreateKind::File)),
            FileChangeKind::Created
        );
        assert_eq!(
            FileChangeKind::from(notify::EventKind::Remove(RemoveKind::File)),
            FileChangeKind::Deleted
        );
        assert_eq!(
            FileChangeKind::from(notify::EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Any
            ))),
            FileChangeKind::Changed
        );
    }
}
