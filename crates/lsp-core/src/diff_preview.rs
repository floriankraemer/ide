//! Whole-file before/after text and line hunks for one document in a
//! pending refactoring, for `RefactorPreviewDialog`'s diff panel (F3-15).
//!
//! `DocumentEdits` only carries the edits themselves — the preview needs the
//! finished text too, and a diff between the two, which is what this module
//! adds on top of [`crate::workspace_edit::apply_to_text`]. Kept out of
//! `workspace_edit` itself: that module is about what a `WorkspaceEdit`
//! means and how it applies, not about presenting the result.

use crate::workspace_edit::{apply_to_text, DocumentEdits, EditError};

/// One document's diff for the preview: the text it applies against, the
/// text it would produce, and the line hunks between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub old_text: String,
    pub new_text: String,
    pub hunks: Vec<editor_core::diff::Hunk>,
}

/// Apply `doc`'s edits to `old_text` and diff the result against it.
///
/// Errors are `apply_to_text`'s — a stale or malformed edit is refused here
/// exactly as it would be when actually applied, so the preview never shows
/// a result the real apply could not produce. A diff over
/// [`editor_core::diff::MAX_DIFF_BYTES`] answers no hunks rather than
/// failing the whole preview: the two texts are still shown, just without
/// change markers, which is the same ceiling the gutter would apply.
pub fn file_diff(old_text: &str, doc: &DocumentEdits) -> Result<FileDiff, EditError> {
    let new_text = apply_to_text(old_text, &doc.edits)?;
    let hunks = editor_core::diff::diff_lines(old_text, &new_text).unwrap_or_default();
    Ok(FileDiff {
        old_text: old_text.to_string(),
        new_text,
        hunks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_edit::TextEdit;
    use editor_core::diff::HunkKind;

    fn doc(edits: Vec<TextEdit>) -> DocumentEdits {
        DocumentEdits {
            uri: "file:///a.rs".to_string(),
            path: "a.rs".to_string(),
            version: None,
            edits,
        }
    }

    fn edit(sl: u32, sc: u32, el: u32, ec: u32, text: &str) -> TextEdit {
        TextEdit {
            start_line: sl,
            start_character: sc,
            end_line: el,
            end_character: ec,
            new_text: text.to_string(),
        }
    }

    #[test]
    fn a_rename_produces_the_new_text_and_one_modified_hunk() {
        let old = "let alpha = 1;\n";
        let d = doc(vec![edit(0, 4, 0, 9, "beta")]);
        let diff = file_diff(old, &d).unwrap();
        assert_eq!(diff.new_text, "let beta = 1;\n");
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].kind, HunkKind::Modified);
    }

    #[test]
    fn no_edits_means_no_hunks() {
        let old = "unchanged\n";
        let diff = file_diff(old, &doc(vec![])).unwrap();
        assert_eq!(diff.new_text, old);
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn an_out_of_bounds_edit_is_refused_like_apply_to_text() {
        let old = "one line\n";
        let d = doc(vec![edit(9, 0, 9, 1, "x")]);
        assert_eq!(file_diff(old, &d), Err(EditError::RangeOutOfBounds));
    }

    #[test]
    fn an_insertion_produces_an_added_hunk() {
        let old = "a\nc\n";
        let d = doc(vec![edit(1, 0, 1, 0, "b\n")]);
        let diff = file_diff(old, &d).unwrap();
        assert_eq!(diff.new_text, "a\nb\nc\n");
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].kind, HunkKind::Added);
    }
}
