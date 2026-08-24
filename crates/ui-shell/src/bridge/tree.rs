use core::pin::Pin;
use std::cell::RefCell;
use std::rc::Rc;

use app_core::{AppError, AppSession};
use cxx_qt::Threading;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};

use crate::bridge::convert::{push_recent_project, to_ffi_result};
use crate::bridge::ffi::{self, FfiResult, Roles};
use crate::bridge::registry::{shared_icons, shared_session, SharedIcons};

/// Rust side of the `ProjectTreeModel` QObject: handles on the shared
/// session and icon theme, nothing else — the tree data itself lives in
/// `app-core`.
pub struct ProjectTreeModelRust {
    session: Rc<RefCell<AppSession>>,
    icons: Rc<SharedIcons>,
}

impl Default for ProjectTreeModelRust {
    fn default() -> Self {
        Self {
            session: shared_session(),
            icons: shared_icons(),
        }
    }
}

/// Separates the collapsed key from the expanded one in the `IconKey` role.
///
/// The role carries *both* states of a row's icon, because that is what Qt
/// already knows how to use: a `QIcon` holds a `QIcon::Off` and a
/// `QIcon::On` pixmap, `QTreeView` paints an expanded row with
/// `QStyle::State_Open`, and `QStyledItemDelegate` turns that into
/// `QIcon::On`. Handing the view one icon that knows both states means an
/// expand or collapse repaints the open/closed folder art on its own — no
/// `dataChanged` plumbing on the expansion signals, and no C++ asking the
/// view what state a row is in.
///
/// A newline because an icon key is `<pack-id>/<icon-id>` and neither part
/// can contain one: `plugin-api` restricts the id charset and an icon id is
/// a file stem.
const ICON_KEY_STATE_SEPARATOR: char = '\n';

/// `Qt::UserRole` — the first role number Qt promises never to use itself.
const QT_USER_ROLE: i32 = 0x0100;

/// The role number a `Roles` variant actually travels as. See the `Roles`
/// doc comment: the variants are offsets, because cxx-qt cannot give them
/// discriminants of their own.
const fn user_role(role: Roles) -> i32 {
    QT_USER_ROLE + role.repr
}

impl ffi::ProjectTreeModel {
    /// Row count for `parent` — the number of children the arena node has.
    /// Files (and empty directories) simply have no children, so this
    /// naturally yields 0 without any separate "is leaf" tracking; Qt's
    /// tree view relies on that to skip drawing an expand affordance.
    pub fn row_count(&self, parent: &QModelIndex) -> i32 {
        let session = self.session.borrow();
        let Some(project) = session.project() else {
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
        let session = self.session.borrow();
        let Some(project) = session.project() else {
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
        let session = self.session.borrow();
        let Some(project) = session.project() else {
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
        let session = self.session.borrow();
        let Some(project) = session.project() else {
            return QVariant::default();
        };
        if !index.is_valid() {
            return QVariant::default();
        }
        let node = project.tree.node(index.internal_id());
        match role {
            // Qt::DisplayRole
            0 => QVariant::from(&QString::from(node.name.as_str())),
            r if r == user_role(Roles::Path) => {
                QVariant::from(&QString::from(node.path.to_string_lossy().as_ref()))
            }
            r if r == user_role(Roles::IsDir) => QVariant::from(&node.is_dir),
            r if r == user_role(Roles::IconKey) => {
                // The arena's root node is the model's invisible root, so no
                // row a view ever asks about is the project root itself.
                let key = |expanded| {
                    self.icons.service.borrow().icon_key(
                        &node.path,
                        node.is_dir,
                        expanded,
                        false,
                        self.icons.appearance,
                    )
                };
                match (key(false), key(true)) {
                    (Some(closed), Some(open)) => QVariant::from(&QString::from(
                        format!("{closed}{ICON_KEY_STATE_SEPARATOR}{open}").as_str(),
                    )),
                    // No icon theme active: an empty key, which the
                    // decoration proxy turns into an invalid QVariant so the
                    // row reserves no icon width.
                    _ => QVariant::from(&QString::default()),
                }
            }
            // Every role Qt itself defines (decoration, edit, tooltip, size
            // hint, ...) lands here and gets an invalid QVariant, which is
            // what tells the view "this item has no icon, no tooltip, no
            // size of its own" and keeps the label flush against its own
            // branch indicator.
            _ => QVariant::default(),
        }
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(0, QByteArray::from("display"));
        roles.insert(user_role(Roles::Path), QByteArray::from("path"));
        roles.insert(user_role(Roles::IsDir), QByteArray::from("isDir"));
        roles.insert(user_role(Roles::IconKey), QByteArray::from("iconKey"));
        roles
    }

    pub fn open_folder(mut self: Pin<&mut Self>, path: &QString) -> FfiResult {
        let path = std::path::PathBuf::from(path.to_string());
        // Borrow scoped tightly: `endResetModel` synchronously re-enters
        // `rowCount`/`data`, which take their own borrow of the session.
        let result = self.session.borrow_mut().open_project(&path);
        if result.is_ok() {
            unsafe {
                self.as_mut().begin_reset_model();
                self.as_mut().end_reset_model();
            }
            self.as_mut().start_watcher();
            self.as_mut().emit_project_opened();
            push_recent_project(path);
        }
        to_ffi_result(result)
    }

    pub fn reopen_last_project(mut self: Pin<&mut Self>) -> bool {
        let opened = self.session.borrow_mut().reopen_last_project();
        if opened {
            unsafe {
                self.as_mut().begin_reset_model();
                self.as_mut().end_reset_model();
            }
            self.as_mut().start_watcher();
            self.as_mut().emit_project_opened();
        }
        opened
    }

    /// Shared tail for `open_folder`/`reopen_last_project`: re-reads the
    /// now-current root path from the session (rather than trusting the
    /// caller-supplied `path` verbatim) and emits `projectOpened`.
    fn emit_project_opened(mut self: Pin<&mut Self>) {
        let root = self
            .session
            .borrow()
            .root_path()
            .map(|p| p.to_string_lossy().into_owned());
        if let Some(root) = root {
            self.as_mut().project_opened(QString::from(root.as_str()));
        }
    }

    /// (Re)start the filesystem watcher for whatever project is now
    /// current, replacing any previous watcher (single watcher). Each fs
    /// event queues a closure onto this `ProjectTreeModel`'s own Qt thread —
    /// the one cross-thread hop in the whole design — which, only for a
    /// *structural* event (see `project_model::is_structural_change`),
    /// rebuilds the tree and resets the model; every event (structural or
    /// not) still emits `filesChangedExternally(path)` for `main_window.cpp`
    /// to relay to `DocumentManager` via an ordinary (already-on-the-Qt-
    /// thread) signal connection, so US-3's reload/keep prompt for an open
    /// tab's content change keeps working. That relay is why `project-model`'s
    /// watcher only ever needs one `CxxQtThread` handle, not two.
    ///
    /// Root cause of the "saving a file collapses the sidebar" bug: this
    /// used to reset the model on *every* fs event unconditionally,
    /// including the app's own `Ctrl+S` write of a file that was already in
    /// the tree — a content-only change that doesn't move a single row.
    /// `beginResetModel`/`endResetModel` throws away Qt's per-item expand
    /// state for the whole tree, so every save re-collapsed it. Filtering
    /// on the event kind here fixes both the app's own saves and genuinely
    /// external content-only edits (no reason to reset for either), while
    /// still fully rebuilding for real structural changes (US-2).
    fn start_watcher(self: Pin<&mut Self>) {
        let qt_thread = self.qt_thread();
        self.session
            .borrow_mut()
            .start_watcher(move |kind, changed_path| {
                let structural = project_model::is_structural_change(&kind);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    if structural {
                        let rebuilt = model.session.borrow_mut().rebuild_tree().is_ok();
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

    pub fn root_path(&self) -> QString {
        match self.session.borrow().root_path() {
            Some(path) => QString::from(path.to_string_lossy().as_ref()),
            None => QString::default(),
        }
    }

    pub fn create_file(
        mut self: Pin<&mut Self>,
        parent_dir: &QString,
        name: &QString,
    ) -> FfiResult {
        let parent = std::path::PathBuf::from(parent_dir.to_string());
        let result = self
            .session
            .borrow_mut()
            .create_file(&parent, &name.to_string());
        self.as_mut().finish_mutation(result.map(|()| None))
    }

    pub fn create_folder(
        mut self: Pin<&mut Self>,
        parent_dir: &QString,
        name: &QString,
    ) -> FfiResult {
        let parent = std::path::PathBuf::from(parent_dir.to_string());
        let result = self
            .session
            .borrow_mut()
            .create_folder(&parent, &name.to_string());
        self.as_mut().finish_mutation(result.map(|()| None))
    }

    pub fn rename_path(mut self: Pin<&mut Self>, path: &QString, new_name: &QString) -> FfiResult {
        let path = std::path::PathBuf::from(path.to_string());
        let result = self
            .session
            .borrow_mut()
            .rename_entry(&path, &new_name.to_string());
        self.as_mut().finish_mutation(result)
    }

    pub fn delete_path(mut self: Pin<&mut Self>, path: &QString) -> FfiResult {
        let path = std::path::PathBuf::from(path.to_string());
        let result = self.session.borrow_mut().delete_entry(&path);
        self.as_mut().finish_mutation(result)
    }

    /// Shared tail for the four tree-mutation slots above: reset the model
    /// so the view re-reads the rebuilt tree, and relay any retitled tab to
    /// the tab strip. The model is also reset when only the tree re-snapshot
    /// failed (`TreeRebuild`) — the disk mutation itself succeeded, so the
    /// stale rows must still be dropped (same behavior as before the
    /// refactoring). Full reset, no incremental diffing — consistent with
    /// the reset-based approach at MVP scope.
    fn finish_mutation(
        mut self: Pin<&mut Self>,
        result: Result<Option<app_core::RetitledTab>, AppError>,
    ) -> FfiResult {
        let mutated_disk = matches!(&result, Ok(_) | Err(AppError::TreeRebuild(_)));
        if mutated_disk {
            unsafe {
                self.as_mut().begin_reset_model();
                self.as_mut().end_reset_model();
            }
        }
        match result {
            Ok(retitled) => {
                if let Some(tab) = retitled {
                    self.as_mut()
                        .tab_title_changed(tab.id.raw(), QString::from(tab.title.as_str()));
                }
                FfiResult::default()
            }
            Err(err) => FfiResult {
                code: err.code(),
                message: QString::from(err.to_string().as_str()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Qt asks a model for `Qt::DecorationRole` (1), `Qt::EditRole` (2) and a
    /// dozen more on every paint. A custom role sharing one of those numbers
    /// answers a question Qt asked itself — a path `QString` handed back as
    /// the decoration made the tree reserve icon width it never drew, so
    /// every label sat well right of its own branch indicator.
    #[test]
    fn tree_roles_stay_out_of_the_range_qt_reserves() {
        // `IconKey` is in here rather than answering Qt::DecorationRole
        // directly: the decoration is `IconDecorationProxy`'s job, and the
        // Rust model stays free of every role Qt defines.
        let roles = [
            ("Path", Roles::Path),
            ("IsDir", Roles::IsDir),
            ("IconKey", Roles::IconKey),
        ];
        for (name, role) in roles {
            assert!(
                user_role(role) >= QT_USER_ROLE,
                "role {name} collides with a Qt-defined role"
            );
        }
        let numbers: Vec<i32> = roles.iter().map(|&(_, role)| user_role(role)).collect();
        let mut distinct = numbers.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), numbers.len(), "two roles share a number");
    }
}
