//! Read-only [`TabKind::Diff`] tabs (F3-14): a revision picked from File
//! History, or an arbitrary second file picked from the Project Tree — the
//! two comparisons that have no live `Document` on either side.
//!
//! The working-tree-vs-`HEAD` diff is deliberately **not** this kind: it has
//! a live `Document` (the tab being diffed), and stays a plain
//! [`TabKind::Text`] tab that the view toggles into a diff *layout* around
//! that same `Document`, so there is never a second, competing owner of a
//! file's dirty/undo state (ADR-0003). A `TabKind::Diff` tab would have
//! forced a choice between owning a second `Document` for the same file, or
//! faking editability over static text that goes nowhere — see this
//! feature's plan doc for the fuller reasoning.
//!
//! Split out once `lib.rs` hit its ratcheted file-size ceiling, the same
//! reason `file_ops.rs`/`tree_sort.rs`/`preview.rs` exist as siblings rather
//! than growing it further.

use std::path::PathBuf;

#[cfg(test)]
use crate::TabKind;
use crate::{AppSession, TabContent, TabEntry, TabId};

/// A read-only, never-dirty comparison of two texts with no live `Document`
/// backing either side. `path` identifies the primary/left side for title
/// and language-id detection only — it is never a save target, so
/// [`TabContent::set_path`]/`mark_deleted` are no-ops for this variant.
///
/// `hunks` is computed once, eagerly, when the tab opens (mirroring
/// `lsp_core::file_diff`'s "diff now, ceiling degrades to no markers rather
/// than failing" shape) — unlike the gutter's `HunkCache`, nothing here ever
/// changes after the tab is created, so there is no staleness to guard
/// against and no cache key to invalidate.
pub(crate) struct DiffContent {
    pub(crate) path: PathBuf,
    left_label: String,
    right_label: String,
    left_text: String,
    right_text: String,
    hunks: Vec<editor_core::diff::Hunk>,
}

impl DiffContent {
    pub(crate) fn new(
        path: PathBuf,
        left_label: String,
        right_label: String,
        left_text: String,
        right_text: String,
    ) -> Self {
        let hunks = editor_core::diff::diff_lines(&left_text, &right_text).unwrap_or_default();
        Self {
            path,
            left_label,
            right_label,
            left_text,
            right_text,
            hunks,
        }
    }

    /// Tab title derived from the file name, same convention
    /// `editor_core::Document::title` and `BinaryFile::title` use.
    pub(crate) fn title(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }
}

impl AppSession {
    /// Open a read-only [`TabKind::Diff`] tab comparing two already-read
    /// texts (F3-14) — a revision from File History, or an arbitrary second
    /// file from the Project Tree. Unlike [`AppSession::open_file`], this
    /// never touches the filesystem or the open-tab table by path: two diff
    /// tabs for the same `path` are two distinct tabs, matching how
    /// JetBrains and every other diff viewer treat "compare again" as a new
    /// comparison, not a reused one.
    pub fn open_diff_tab(
        &mut self,
        path: PathBuf,
        left_label: String,
        right_label: String,
        left_text: String,
        right_text: String,
    ) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        let content = TabContent::Diff(DiffContent::new(
            path,
            left_label,
            right_label,
            left_text,
            right_text,
        ));
        self.docs.push(TabEntry { id, content });
        self.active = Some(id);
        id
    }

    /// The two side labels a [`TabKind::Diff`] tab was opened with (e.g. two
    /// revision short-ids, or two file names). `None` for any other tab.
    pub fn diff_labels(&self, id: TabId) -> Option<(&str, &str)> {
        match self.entry(id).map(|e| &e.content) {
            Some(TabContent::Diff(diff)) => {
                Some((diff.left_label.as_str(), diff.right_label.as_str()))
            }
            _ => None,
        }
    }

    /// The two texts a [`TabKind::Diff`] tab is comparing. `None` for any
    /// other tab.
    pub fn diff_texts(&self, id: TabId) -> Option<(&str, &str)> {
        match self.entry(id).map(|e| &e.content) {
            Some(TabContent::Diff(diff)) => {
                Some((diff.left_text.as_str(), diff.right_text.as_str()))
            }
            _ => None,
        }
    }

    /// The line hunks between a [`TabKind::Diff`] tab's two texts, computed
    /// once when the tab opened. Empty for any other tab.
    pub fn diff_hunks(&self, id: TabId) -> Vec<editor_core::diff::Hunk> {
        match self.entry(id).map(|e| &e.content) {
            Some(TabContent::Diff(diff)) => diff.hunks.clone(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppError;
    use std::fs;

    fn session_with_project() -> (tempfile::TempDir, tempfile::TempDir, AppSession) {
        let project_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        fs::write(project_dir.path().join("a.txt"), "alpha").unwrap();
        let mut session = AppSession::with_config_dir(config_dir.path().to_path_buf());
        session.open_project(project_dir.path()).unwrap();
        (project_dir, config_dir, session)
    }

    #[test]
    fn open_diff_tab_computes_hunks_and_exposes_labels_and_texts() {
        let mut session = AppSession::new();
        let id = session.open_diff_tab(
            PathBuf::from("a.txt"),
            "HEAD".to_string(),
            "Working Tree".to_string(),
            "one\ntwo\n".to_string(),
            "one\nTWO\n".to_string(),
        );

        assert_eq!(session.tab_kind(id), Some(TabKind::Diff));
        assert_eq!(session.tab_title(id).as_deref(), Some("a.txt"));
        assert_eq!(session.diff_labels(id), Some(("HEAD", "Working Tree")));
        assert_eq!(session.diff_texts(id), Some(("one\ntwo\n", "one\nTWO\n")));
        let hunks = session.diff_hunks(id);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, editor_core::diff::HunkKind::Modified);
    }

    #[test]
    fn a_diff_tab_is_never_dirty_and_refuses_text_editing_commands() {
        let mut session = AppSession::new();
        let id = session.open_diff_tab(
            PathBuf::from("a.txt"),
            "left".to_string(),
            "right".to_string(),
            "one\n".to_string(),
            "two\n".to_string(),
        );

        assert_eq!(session.tab_is_dirty(id), Some(false));
        assert_eq!(session.tab_content(id), None);
        for err in [
            session.save_tab(id, "nope").unwrap_err(),
            session.edit_tab(id, "nope").unwrap_err(),
            session.reload_tab(id).unwrap_err(),
            session.save_buffer(id).unwrap_err(),
        ] {
            assert_eq!(err.code(), AppError::CODE_NOT_A_TEXT_TAB);
        }
    }

    #[test]
    fn a_diff_tab_is_never_renamed_or_flagged_deleted_by_an_unrelated_file_op() {
        let (project_dir, _config, mut session) = session_with_project();
        let id = session.open_diff_tab(
            project_dir.path().join("a.txt"),
            "left".to_string(),
            "right".to_string(),
            "one\n".to_string(),
            "two\n".to_string(),
        );

        // Renaming/deleting the real "a.txt" the diff tab happens to be
        // titled after must not retarget or flag a tab that owns no file
        // handle — unlike a text or binary tab opened on that same path.
        session
            .rename_entry(&project_dir.path().join("a.txt"), "renamed.txt")
            .unwrap();
        assert_eq!(session.tab_title(id).as_deref(), Some("a.txt"));
        assert_eq!(session.tab_is_dirty(id), Some(false));
    }

    #[test]
    fn diff_accessors_answer_none_for_a_non_diff_tab() {
        let (project_dir, _config, mut session) = session_with_project();
        let id = session
            .open_file(&project_dir.path().join("a.txt"))
            .unwrap()
            .id;

        assert_eq!(session.diff_labels(id), None);
        assert_eq!(session.diff_texts(id), None);
        assert!(session.diff_hunks(id).is_empty());
    }
}
