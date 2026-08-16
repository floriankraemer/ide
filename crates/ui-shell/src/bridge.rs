// cxx-qt bridge boundary for ui-shell.
//
// Task 2 scope was just enough bridge to show the native Qt6 main window
// with its menu bar. This adds `ProjectTreeModel`: a `QAbstractItemModel`
// implementation wrapping `project-model`'s arena-based `DirectoryTree`
// (Task 5, mvp-implementation-plan.md §2).
#[cxx_qt::bridge]
mod ffi {
    unsafe extern "C++Qt" {
        include!(<QtCore/QAbstractItemModel>);
        /// Base Qt class `ProjectTreeModel` inherits from.
        #[qobject]
        type QAbstractItemModel;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
    }

    /// Extra data roles `data()` answers, alongside `Qt::DisplayRole` (0 —
    /// the node's name, used for the tree view's label). cxx-qt's `qenum`
    /// doesn't support explicit discriminants, so `Reserved` occupies 0
    /// (matching, and never confused with, `Qt::DisplayRole`) purely to
    /// push `Path`/`IsDir` off of it.
    #[qenum(ProjectTreeModel)]
    enum Roles {
        #[doc(hidden)]
        Reserved,
        /// Absolute filesystem path of the node, as a `QString`.
        Path,
        /// Whether the node is a directory (`bool`).
        IsDir,
    }

    extern "RustQt" {
        /// `QAbstractItemModel` wrapping `project-model`'s `DirectoryTree`
        /// arena. The model's invisible root corresponds to the arena's
        /// root node (the open project folder); top-level rows are that
        /// folder's direct children.
        #[qobject]
        #[base = QAbstractItemModel]
        type ProjectTreeModel = super::ProjectTreeModelRust;
    }

    unsafe extern "RustQt" {
        /// # Safety
        ///
        /// Inherited `createIndex` from the base class.
        #[inherit]
        #[cxx_name = "createIndex"]
        unsafe fn create_index(
            self: &ProjectTreeModel,
            row: i32,
            column: i32,
            id: usize,
        ) -> QModelIndex;

        /// # Safety
        ///
        /// Inherited `beginResetModel`/`endResetModel` from the base class —
        /// bracket any full-tree replacement (Task 5 has no watcher yet, so
        /// this is only used for the initial "Open Folder" population).
        #[inherit]
        #[cxx_name = "beginResetModel"]
        unsafe fn begin_reset_model(self: Pin<&mut ProjectTreeModel>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        unsafe fn end_reset_model(self: Pin<&mut ProjectTreeModel>);
    }

    extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &ProjectTreeModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "columnCount"]
        fn column_count(self: &ProjectTreeModel, _parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn index(
            self: &ProjectTreeModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;

        #[qinvokable]
        #[cxx_override]
        fn parent(self: &ProjectTreeModel, child: &QModelIndex) -> QModelIndex;

        #[qinvokable]
        #[cxx_override]
        fn data(self: &ProjectTreeModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &ProjectTreeModel) -> QHash_i32_QByteArray;

        /// Open `path` as the active project (via `project-model`'s
        /// `ProjectSession`, which also persists it as last-opened) and
        /// reset the model to reflect the new tree. Returns an empty
        /// string on success, or a user-facing error message on failure —
        /// the current tree (if any) is left unchanged on failure (US-1).
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(self: Pin<&mut ProjectTreeModel>, path: &QString) -> QString;

        /// Binary-vs-text sniff (US-2b's last bullet) for a file path, used
        /// by the tree-view click handler before attempting to open it.
        #[qinvokable]
        #[cxx_name = "isBinaryFile"]
        fn is_binary_file(self: &ProjectTreeModel, path: &QString) -> bool;

        /// Absolute path of the open project's root folder, or an empty
        /// string if none is open. Used by the tree-view context menu to
        /// target "New File"/"New Folder" at the root when the user
        /// right-clicks empty space rather than a node (US-2b).
        #[qinvokable]
        #[cxx_name = "rootPath"]
        fn root_path(self: &ProjectTreeModel) -> QString;

        /// Create an empty file named `name` inside `parent_dir` and
        /// refresh the tree. Returns an empty string on success, or a
        /// user-facing error message on failure (e.g. name already taken).
        #[qinvokable]
        #[cxx_name = "createFile"]
        fn create_file(self: Pin<&mut ProjectTreeModel>, parent_dir: &QString, name: &QString) -> QString;

        /// Create an empty folder named `name` inside `parent_dir` and
        /// refresh the tree. Returns an empty string on success, or a
        /// user-facing error message on failure.
        #[qinvokable]
        #[cxx_name = "createFolder"]
        fn create_folder(self: Pin<&mut ProjectTreeModel>, parent_dir: &QString, name: &QString) -> QString;

        /// Rename `path` (file or folder) to `new_name` in place and
        /// refresh the tree. Returns an empty string on success, or a
        /// user-facing error message on failure.
        #[qinvokable]
        #[cxx_name = "renamePath"]
        fn rename_path(self: Pin<&mut ProjectTreeModel>, path: &QString, new_name: &QString) -> QString;

        /// Delete `path` (recursively if it's a folder) and refresh the
        /// tree. Returns an empty string on success, or a user-facing error
        /// message on failure.
        #[qinvokable]
        #[cxx_name = "deletePath"]
        fn delete_path(self: Pin<&mut ProjectTreeModel>, path: &QString) -> QString;

        /// Reopen the last-persisted project (US-1's "relaunch reopens the
        /// last project" criterion) and start its filesystem watcher.
        /// Returns whether a project was found and opened; false (with the
        /// model left empty) if nothing was persisted or it no longer
        /// exists — startup is silent about a missing last project rather
        /// than popping an error dialog before the window is even shown.
        #[qinvokable]
        #[cxx_name = "reopenLastProject"]
        fn reopen_last_project(self: Pin<&mut ProjectTreeModel>) -> bool;

        /// Emitted on the Qt thread after a filesystem-watcher event has
        /// already been folded into a tree rebuild + reset (Task 8, plan
        /// §2). `main_window.cpp` connects this to
        /// `DocumentManager::checkExternalChange` so an open tab whose
        /// backing file changed on disk gets the reload/keep prompt (US-3).
        #[qsignal]
        #[cxx_name = "filesChangedExternally"]
        fn files_changed_externally(self: Pin<&mut ProjectTreeModel>, path: QString);
    }

    // Enables `self.qt_thread()` on `ProjectTreeModel`, giving the
    // `notify` watcher thread (owned by `project-model`) a `CxxQtThread`
    // handle it can queue tree-rebuild closures onto safely (plan §2) —
    // the only cross-thread communication in the watcher design, no
    // hand-rolled synchronization.
    impl cxx_qt::Threading for ProjectTreeModel {}

    extern "RustQt" {
        /// `QObject` wrapping `editor-core`'s `TabList` (Task 6,
        /// mvp-implementation-plan.md §2). Owns which files are open and
        /// their dirty flags; the `QPlainTextEdit` widgets own live
        /// keystroke editing (see module docs on the "Live editing" split).
        #[qobject]
        type DocumentManager = super::DocumentManagerRust;

        /// Emitted when `openFile` opens a genuinely new tab (not when it
        /// just focuses an already-open one) — the tab strip appends a new
        /// page in response.
        #[qsignal]
        #[cxx_name = "tabOpened"]
        fn tab_opened(self: Pin<&mut DocumentManager>, index: i32, title: QString);

        /// Emitted after `closeTab` actually removes a tab — the tab strip
        /// removes the corresponding page in response.
        #[qsignal]
        #[cxx_name = "tabClosed"]
        fn tab_closed(self: Pin<&mut DocumentManager>, index: i32);

        /// Emitted when a tab's dirty flag changes (via `setTabModified` or
        /// a successful `saveTab`) — the tab strip updates its
        /// unsaved-changes indicator in response.
        #[qsignal]
        #[cxx_name = "tabModifiedChanged"]
        fn tab_modified_changed(self: Pin<&mut DocumentManager>, index: i32, modified: bool);

        /// Emitted when a tab's title needs to change without the tab
        /// itself opening/closing — a rename via the tree
        /// (`notifyPathRenamed`) or the "(deleted)" suffix from
        /// `notifyPathDeleted` (US-2b). The tab strip updates its label in
        /// response, preserving the unsaved-changes indicator.
        #[qsignal]
        #[cxx_name = "tabTitleChanged"]
        fn tab_title_changed(self: Pin<&mut DocumentManager>, index: i32, title: QString);

        /// Declared per mvp-implementation-plan.md §2 for the filesystem
        /// watcher (Task 8); not wired up yet — out of scope for this task.
        #[qsignal]
        #[cxx_name = "externalChangeDetected"]
        fn external_change_detected(self: Pin<&mut DocumentManager>, path: QString);

        /// Open `path` as a new tab, or focus its existing tab if already
        /// open (US-3: focus-not-duplicate). Returns the tab index, or -1
        /// on failure (see `lastError`).
        #[qinvokable]
        #[cxx_name = "openFile"]
        fn open_file(self: Pin<&mut DocumentManager>, path: &QString) -> i32;

        /// Close the tab at `index`. The caller (UI) is responsible for any
        /// unsaved-changes prompt before calling this.
        #[qinvokable]
        #[cxx_name = "closeTab"]
        fn close_tab(self: Pin<&mut DocumentManager>, index: i32);

        /// Replace the tab's content with `content` and write it to disk.
        /// Returns an empty string on success, or a user-facing error
        /// message on failure (US-4: no silent data loss — the dirty flag
        /// is left set on failure).
        #[qinvokable]
        #[cxx_name = "saveTab"]
        fn save_tab(self: Pin<&mut DocumentManager>, index: i32, content: &QString) -> QString;

        /// Update which tab `editor-core` considers active.
        #[qinvokable]
        #[cxx_name = "setActiveTab"]
        fn set_active_tab(self: Pin<&mut DocumentManager>, index: i32);

        /// Mirror `QPlainTextEdit`'s own `QTextDocument::modificationChanged`
        /// state into `editor-core`'s per-tab dirty flag (see module docs —
        /// live keystrokes are not marshalled through the rope).
        #[qinvokable]
        #[cxx_name = "setTabModified"]
        fn set_tab_modified(self: Pin<&mut DocumentManager>, index: i32, modified: bool);

        /// The tab's current buffer content, used to populate a newly
        /// created `QPlainTextEdit` page when a tab is opened.
        #[qinvokable]
        #[cxx_name = "tabContent"]
        fn tab_content(self: &DocumentManager, index: i32) -> QString;

        /// User-facing reason the last `openFile` call failed, if any.
        #[qinvokable]
        #[cxx_name = "lastError"]
        fn last_error(self: &DocumentManager) -> QString;

        /// If `old_path` has an open tab, point it at `new_path` and emit
        /// `tabTitleChanged` (US-2b: a rename via the tree must update the
        /// tab, not silently keep pointing at the stale path). No-op if
        /// `old_path` isn't open.
        #[qinvokable]
        #[cxx_name = "notifyPathRenamed"]
        fn notify_path_renamed(self: Pin<&mut DocumentManager>, old_path: &QString, new_path: &QString);

        /// If `path` has an open tab, mark it deleted (blocking further
        /// silent saves — see `editor_core::Document::mark_deleted`) and
        /// emit `tabTitleChanged` with a "(deleted)" suffix (US-2b). No-op
        /// if `path` isn't open.
        #[qinvokable]
        #[cxx_name = "notifyPathDeleted"]
        fn notify_path_deleted(self: Pin<&mut DocumentManager>, path: &QString);

        /// Handle a filesystem-watcher event for `path` (relayed via
        /// `ProjectTreeModel::filesChangedExternally`, already running on
        /// the Qt thread by the time this is called — plain signal/slot,
        /// no further cross-thread hop needed). Emits
        /// `externalChangeDetected(path)` if `path` has an open tab, unless
        /// the change is one this `DocumentManager` caused itself (a recent
        /// `saveTab`/tree-driven rename onto that path) or the tab was
        /// already flagged deleted by a tree-driven delete (Task 8).
        #[qinvokable]
        #[cxx_name = "checkExternalChange"]
        fn check_external_change(self: Pin<&mut DocumentManager>, path: &QString);

        /// Index of the open tab backed by `path`, or -1 if `path` isn't
        /// open. Used by `main_window.cpp` to resolve
        /// `externalChangeDetected(path)` to a tab before prompting.
        #[qinvokable]
        #[cxx_name = "tabIndexForPath"]
        fn tab_index_for_path(self: &DocumentManager, path: &QString) -> i32;

        /// Re-read the tab's backing file from disk, discarding any
        /// in-editor edits (the "Reload" choice on the external-change
        /// prompt, US-3). Returns an empty string on success, or a
        /// user-facing error message on failure.
        #[qinvokable]
        #[cxx_name = "reloadTabFromDisk"]
        fn reload_tab_from_disk(self: Pin<&mut DocumentManager>, index: i32) -> QString;
    }

    unsafe extern "C++" {
        include!("main_window.h");

        /// Builds and shows the main window, then runs the Qt event loop
        /// until it's closed. Returns the process exit code.
        #[namespace = "ui_shell"]
        fn run_app() -> i32;
    }
}

use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use ffi::Roles;
use std::time::{Duration, Instant};

/// How long after this `DocumentManager` writes a path to disk itself
/// (`saveTab`) or repoints a tab onto a new path (a tree-driven rename) a
/// matching filesystem-watcher event for that path is treated as an echo
/// of our own change rather than a genuine external edit — see
/// `DocumentManagerRust::suppressed_changes`. Generous enough to absorb
/// typical inotify/Qt-event-loop latency; not meant to be race-proof.
const SELF_CHANGE_SUPPRESSION_WINDOW: Duration = Duration::from_millis(1500);

/// Whether a filesystem-watcher event actually shifts the project tree's
/// shape (a file/folder created, removed, or renamed) as opposed to a
/// content-only change to a path that's already in the tree (a plain write,
/// e.g. every `Ctrl+S` save — same rows, same structure, just different
/// bytes on disk). Only the former needs the tree model rebuilt and reset;
/// resetting for the latter is exactly what caused the sidebar to
/// re-collapse on every save (`beginResetModel`/`endResetModel` discards
/// Qt's per-item expand state for the whole tree).
fn is_structural_change(kind: &notify::EventKind) -> bool {
    matches!(
        kind,
        notify::EventKind::Create(_)
            | notify::EventKind::Remove(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
    )
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
        assert!(!is_structural_change(&EventKind::Modify(
            ModifyKind::Data(notify::event::DataChange::Any)
        )));
    }

    #[test]
    fn metadata_only_changes_are_not_structural() {
        assert!(!is_structural_change(&EventKind::Modify(
            ModifyKind::Metadata(notify::event::MetadataKind::Any)
        )));
    }
}

/// Rust-side state behind the `ProjectTreeModel` QObject: the currently
/// open project (if any), owned via `project-model`'s `ProjectSession`.
#[derive(Default)]
pub struct ProjectTreeModelRust {
    session: project_model::ProjectSession,
}

impl ffi::ProjectTreeModel {
    /// Row count for `parent` — the number of children the arena node has.
    /// Files (and empty directories) simply have no children, so this
    /// naturally yields 0 without any separate "is leaf" tracking; Qt's
    /// tree view relies on that to skip drawing an expand affordance.
    pub fn row_count(&self, parent: &QModelIndex) -> i32 {
        let Some(project) = self.session.current() else {
            return 0;
        };
        let tree = &project.tree;
        let node_id = if parent.is_valid() {
            parent.internal_id()
        } else {
            tree.root_id()
        };
        tree.children(node_id).len() as i32
    }

    pub fn column_count(&self, _parent: &QModelIndex) -> i32 {
        1
    }

    /// Map (row, column, parent) to a `QModelIndex` carrying the child
    /// arena node's id as `internalId` — the id is the only piece of
    /// arena-mapping state a `QModelIndex` needs to carry, since `parent()`
    /// can always re-derive a node's row by searching its own parent's
    /// children.
    pub fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex {
        let Some(project) = self.session.current() else {
            return QModelIndex::default();
        };
        let tree = &project.tree;
        let parent_id = if parent.is_valid() {
            parent.internal_id()
        } else {
            tree.root_id()
        };
        let children = tree.children(parent_id);
        match children.get(row as usize) {
            Some(&child_id) => unsafe { self.create_index(row, column, child_id) },
            None => QModelIndex::default(),
        }
    }

    /// Map a child index back to its parent's `QModelIndex`. The arena's
    /// root node is never itself wrapped in a `QModelIndex` — it is the
    /// model's invisible root — so a child whose arena parent is the root
    /// correctly yields an invalid (root) `QModelIndex`.
    pub fn parent(&self, child: &QModelIndex) -> QModelIndex {
        let Some(project) = self.session.current() else {
            return QModelIndex::default();
        };
        let tree = &project.tree;
        if !child.is_valid() {
            return QModelIndex::default();
        }
        let node = tree.node(child.internal_id());
        let Some(parent_id) = node.parent else {
            return QModelIndex::default();
        };
        if parent_id == tree.root_id() {
            return QModelIndex::default();
        }
        let parent_node = tree.node(parent_id);
        // parent_id != root_id, so parent_node.parent is always Some.
        let grandparent_id = parent_node.parent.expect("non-root node has a parent");
        let row = tree
            .children(grandparent_id)
            .iter()
            .position(|&id| id == parent_id)
            .expect("parent_id must be one of its own parent's children") as i32;
        unsafe { self.create_index(row, 0, parent_id) }
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(project) = self.session.current() else {
            return QVariant::default();
        };
        if !index.is_valid() {
            return QVariant::default();
        }
        let node = project.tree.node(index.internal_id());
        match role {
            // Qt::DisplayRole
            0 => QVariant::from(&QString::from(node.name.as_str())),
            r if r == Roles::Path.repr => {
                QVariant::from(&QString::from(node.path.to_string_lossy().as_ref()))
            }
            r if r == Roles::IsDir.repr => QVariant::from(&node.is_dir),
            // Never sent from C++ (only `Path`/`IsDir` are used as roles) —
            // exists so `Roles::Reserved` (which only exists to push
            // `Path`/`IsDir` off of 0, since cxx-qt's `qenum` doesn't
            // support explicit discriminants) counts as used.
            r if r == Roles::Reserved.repr => QVariant::default(),
            _ => QVariant::default(),
        }
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(0, QByteArray::from("display"));
        roles.insert(Roles::Path.repr, QByteArray::from("path"));
        roles.insert(Roles::IsDir.repr, QByteArray::from("isDir"));
        roles
    }

    pub fn open_folder(mut self: Pin<&mut Self>, path: &QString) -> QString {
        let path = std::path::PathBuf::from(path.to_string());
        let config_dir = project_model::default_config_dir()
            .unwrap_or_else(|| std::env::temp_dir().join("ide"));

        let result = self.as_mut().rust_mut().session.open_folder(&path, &config_dir);

        match result {
            Ok(()) => {
                unsafe {
                    self.as_mut().begin_reset_model();
                    self.as_mut().end_reset_model();
                }
                self.as_mut().start_watcher();
                QString::default()
            }
            Err(err) => QString::from(err.to_string().as_str()),
        }
    }

    pub fn reopen_last_project(mut self: Pin<&mut Self>) -> bool {
        let config_dir = project_model::default_config_dir()
            .unwrap_or_else(|| std::env::temp_dir().join("ide"));

        let opened = self
            .as_mut()
            .rust_mut()
            .session
            .reopen_last(&config_dir)
            .unwrap_or(false);

        if opened {
            unsafe {
                self.as_mut().begin_reset_model();
                self.as_mut().end_reset_model();
            }
            self.as_mut().start_watcher();
        }
        opened
    }

    /// (Re)start the filesystem watcher for whatever project is now
    /// current, replacing any previous watcher (plan §2: single watcher).
    /// Each fs event queues a closure onto this `ProjectTreeModel`'s own Qt
    /// thread — the one cross-thread hop in the whole design — which, only
    /// for a *structural* event (see `is_structural_change`), rebuilds the
    /// tree and resets the model; every event (structural or not) still
    /// emits `filesChangedExternally(path)` for `main_window.cpp` to relay
    /// to `DocumentManager` via an ordinary (already-on-the-Qt-thread)
    /// signal connection, so US-3's reload/keep prompt for an open tab's
    /// content change keeps working. That relay is why `project-model`'s
    /// watcher only ever needs one `CxxQtThread` handle, not two.
    ///
    /// Root cause of the "saving a file collapses the sidebar" bug: this
    /// used to reset the model on *every* fs event unconditionally,
    /// including the app's own `Ctrl+S` write of a file that was already in
    /// the tree — a content-only change that doesn't move a single row.
    /// `beginResetModel`/`endResetModel` throws away Qt's per-item expand
    /// state for the whole tree, so every save re-collapsed it. Filtering
    /// on `EventKind` here fixes both the app's own saves and genuinely
    /// external content-only edits (no reason to reset for either), while
    /// still fully rebuilding for real structural changes (US-2).
    fn start_watcher(mut self: Pin<&mut Self>) {
        let qt_thread = self.qt_thread();
        self.as_mut()
            .rust_mut()
            .session
            .start_watcher(move |kind, changed_path| {
                let structural = is_structural_change(&kind);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    if structural {
                        let rebuilt = model
                            .as_mut()
                            .rust_mut()
                            .session
                            .rebuild_tree()
                            .is_ok();
                        if rebuilt {
                            unsafe {
                                model.as_mut().begin_reset_model();
                                model.as_mut().end_reset_model();
                            }
                        }
                    }
                    let path = QString::from(changed_path.to_string_lossy().as_ref());
                    model.as_mut().files_changed_externally(path);
                });
            });
    }

    pub fn is_binary_file(&self, path: &QString) -> bool {
        let path = std::path::PathBuf::from(path.to_string());
        editor_core::looks_binary_file(&path).unwrap_or(true)
    }

    pub fn root_path(&self) -> QString {
        match self.session.current() {
            Some(project) => QString::from(project.root.path().to_string_lossy().as_ref()),
            None => QString::default(),
        }
    }

    pub fn create_file(mut self: Pin<&mut Self>, parent_dir: &QString, name: &QString) -> QString {
        let parent = std::path::PathBuf::from(parent_dir.to_string());
        let result = project_model::create_file(&parent, &name.to_string());
        self.as_mut().finish_mutation(result.map(|_| ()))
    }

    pub fn create_folder(mut self: Pin<&mut Self>, parent_dir: &QString, name: &QString) -> QString {
        let parent = std::path::PathBuf::from(parent_dir.to_string());
        let result = project_model::create_folder(&parent, &name.to_string());
        self.as_mut().finish_mutation(result.map(|_| ()))
    }

    pub fn rename_path(mut self: Pin<&mut Self>, path: &QString, new_name: &QString) -> QString {
        let path = std::path::PathBuf::from(path.to_string());
        let result = project_model::rename_path(&path, &new_name.to_string());
        self.as_mut().finish_mutation(result.map(|_| ()))
    }

    pub fn delete_path(mut self: Pin<&mut Self>, path: &QString) -> QString {
        let path = std::path::PathBuf::from(path.to_string());
        let result = project_model::delete_path(&path);
        self.as_mut().finish_mutation(result)
    }

    /// Shared tail for the four mutation methods above: on success,
    /// re-snapshot the tree from disk and reset the model (full reset, no
    /// incremental diffing — consistent with Task 5's reset-based approach
    /// at MVP scope); on failure, leave the tree untouched and surface the
    /// error message.
    fn finish_mutation(
        mut self: Pin<&mut Self>,
        result: Result<(), project_model::FileOpError>,
    ) -> QString {
        match result {
            Ok(()) => {
                let rebuild = self.as_mut().rust_mut().session.rebuild_tree();
                unsafe {
                    self.as_mut().begin_reset_model();
                    self.as_mut().end_reset_model();
                }
                match rebuild {
                    Ok(()) => QString::default(),
                    Err(e) => QString::from(e.to_string().as_str()),
                }
            }
            Err(err) => QString::from(err.to_string().as_str()),
        }
    }
}

/// Rust-side state behind the `DocumentManager` QObject: `editor-core`'s
/// tab list, plus the last `openFile` error for `lastError()` (US-3/US-4,
/// Task 6).
#[derive(Default)]
pub struct DocumentManagerRust {
    tabs: editor_core::TabList,
    last_error: String,
    /// Paths this `DocumentManager` itself just changed on disk (a
    /// `saveTab`) or repointed a tab onto (a tree-driven rename), each with
    /// the `Instant` it happened — the own-save/own-rename feedback-loop
    /// guard for `check_external_change` (Task 8: the filesystem watcher
    /// would otherwise also see these as "external" changes).
    suppressed_changes: std::collections::HashMap<std::path::PathBuf, Instant>,
}

impl ffi::DocumentManager {
    pub fn open_file(mut self: Pin<&mut Self>, path: &QString) -> i32 {
        let path = std::path::PathBuf::from(path.to_string());
        let tabs_before = self.tabs.len();

        let result = self.as_mut().rust_mut().tabs.open(&path);
        match result {
            Ok(index) => {
                let is_new = self.tabs.len() > tabs_before;
                if is_new {
                    let title = self
                        .tabs
                        .get(index)
                        .map(|doc| doc.title())
                        .unwrap_or_default();
                    self.as_mut().tab_opened(index as i32, QString::from(title.as_str()));
                }
                index as i32
            }
            Err(err) => {
                self.as_mut().rust_mut().last_error = err.to_string();
                -1
            }
        }
    }

    pub fn close_tab(mut self: Pin<&mut Self>, index: i32) {
        if index < 0 {
            return;
        }
        let closed = self
            .as_mut()
            .rust_mut()
            .tabs
            .close(index as usize)
            .is_some();
        if closed {
            self.as_mut().tab_closed(index);
        }
    }

    pub fn save_tab(mut self: Pin<&mut Self>, index: i32, content: &QString) -> QString {
        if index < 0 {
            return QString::from("no such tab");
        }
        let content = content.to_string();
        let save_result = {
            let mut rust = self.as_mut().rust_mut();
            match rust.tabs.get_mut(index as usize) {
                Some(doc) => {
                    doc.replace_content(&content);
                    let path = doc.path().to_path_buf();
                    let result = doc.save().map_err(|err| err.to_string());
                    if result.is_ok() {
                        rust.suppressed_changes.insert(path, Instant::now());
                    }
                    result
                }
                None => Err("no such tab".to_string()),
            }
        };

        match save_result {
            Ok(()) => {
                self.as_mut().tab_modified_changed(index, false);
                QString::default()
            }
            Err(err) => QString::from(err.as_str()),
        }
    }

    pub fn set_active_tab(mut self: Pin<&mut Self>, index: i32) {
        if index >= 0 {
            self.as_mut().rust_mut().tabs.set_active(index as usize);
        }
    }

    pub fn set_tab_modified(mut self: Pin<&mut Self>, index: i32, modified: bool) {
        if index < 0 {
            return;
        }
        let changed = {
            let mut rust = self.as_mut().rust_mut();
            match rust.tabs.get_mut(index as usize) {
                Some(doc) => {
                    doc.set_dirty(modified);
                    true
                }
                None => false,
            }
        };
        if changed {
            self.as_mut().tab_modified_changed(index, modified);
        }
    }

    pub fn tab_content(&self, index: i32) -> QString {
        if index < 0 {
            return QString::default();
        }
        self.tabs
            .get(index as usize)
            .map(|doc| QString::from(doc.content().as_str()))
            .unwrap_or_default()
    }

    pub fn last_error(&self) -> QString {
        QString::from(self.last_error.as_str())
    }

    pub fn notify_path_renamed(mut self: Pin<&mut Self>, old_path: &QString, new_path: &QString) {
        let old_path = std::path::PathBuf::from(old_path.to_string());
        let new_path = std::path::PathBuf::from(new_path.to_string());
        let Some(index) = self.tabs.find_by_path(&old_path) else {
            return;
        };
        let title = {
            let mut rust = self.as_mut().rust_mut();
            let doc = rust
                .tabs
                .get_mut(index)
                .expect("index came from find_by_path on the same tab list");
            doc.set_path(new_path.clone());
            let title = doc.title();
            // The watcher will also see this rename land on `new_path` —
            // suppress the echo (same reasoning as `save_tab`).
            rust.suppressed_changes.insert(new_path, Instant::now());
            title
        };
        self.as_mut().tab_title_changed(index as i32, QString::from(title.as_str()));
    }

    /// Handle a filesystem-watcher event for `path`, relayed from
    /// `ProjectTreeModel::filesChangedExternally` (Task 8). No-op unless
    /// `path` has an open tab; further no-ops if that tab was already
    /// flagged deleted by a tree-driven delete (nothing to reload/keep —
    /// see `notify_path_deleted`), or if `path` was changed by this
    /// `DocumentManager` itself within the suppression window (`save_tab`
    /// or a tree-driven rename onto `path`) rather than externally.
    pub fn check_external_change(mut self: Pin<&mut Self>, path: &QString) {
        let path = std::path::PathBuf::from(path.to_string());
        let Some(index) = self.tabs.find_by_path(&path) else {
            return;
        };
        if self.tabs.get(index).map(|d| d.is_deleted()).unwrap_or(true) {
            return;
        }
        let is_own_change = self
            .suppressed_changes
            .get(&path)
            .map(|at| at.elapsed() < SELF_CHANGE_SUPPRESSION_WINDOW)
            .unwrap_or(false);
        if is_own_change {
            return;
        }
        self.as_mut()
            .external_change_detected(QString::from(path.to_string_lossy().as_ref()));
    }

    pub fn tab_index_for_path(&self, path: &QString) -> i32 {
        let path = std::path::PathBuf::from(path.to_string());
        self.tabs
            .find_by_path(&path)
            .map(|i| i as i32)
            .unwrap_or(-1)
    }

    pub fn reload_tab_from_disk(mut self: Pin<&mut Self>, index: i32) -> QString {
        if index < 0 {
            return QString::from("no such tab");
        }
        let mut rust = self.as_mut().rust_mut();
        match rust.tabs.get_mut(index as usize) {
            Some(doc) => match doc.reload() {
                Ok(()) => QString::default(),
                Err(err) => QString::from(err.to_string().as_str()),
            },
            None => QString::from("no such tab"),
        }
    }

    pub fn notify_path_deleted(mut self: Pin<&mut Self>, path: &QString) {
        let path = std::path::PathBuf::from(path.to_string());
        let Some(index) = self.tabs.find_by_path(&path) else {
            return;
        };
        let title = {
            let mut rust = self.as_mut().rust_mut();
            let doc = rust
                .tabs
                .get_mut(index)
                .expect("index came from find_by_path on the same tab list");
            doc.mark_deleted();
            format!("{} (deleted)", doc.title())
        };
        self.as_mut().tab_title_changed(index as i32, QString::from(title.as_str()));
    }
}

pub use ffi::run_app;
