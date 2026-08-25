//! Revert one hunk as an edit against the open buffer, not a write to disk
//! (F3-11).
//!
//! The point, worth getting right now even though nothing crosses the FFI
//! seam from this crate yet: reverting a hunk must be **spliced into the
//! open buffer**, exactly like every other edit source this IDE has (a
//! reformat, an intention, an applied AI block) — so the same
//! `beginEditBlock` makes it one `Ctrl+Z`, and a file with unsaved changes
//! elsewhere never gets clobbered by a whole-file write.

use std::path::Path;

use editor_core::diff::Hunk;

use crate::error::VcsError;
use crate::repo::Repository;

/// A text-range replacement, in the shape a future bridge can turn
/// directly into an `FfiTextEdit` — mirroring the spirit of
/// `lsp_core::workspace_edit::TextEdit` (a range plus replacement text)
/// rather than its exact units: an LSP edit addresses UTF-16 characters
/// because a server can touch part of a line, but a hunk revert only ever
/// replaces whole lines (that is what a [`Hunk`] *is*), so this is a
/// half-open **line** range instead. `start_line == end_line` is a pure
/// insertion, exactly as an empty LSP range is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    /// 0-based, inclusive.
    pub start_line: usize,
    /// 0-based, exclusive.
    pub end_line: usize,
    /// Replacement text for the range, each line newline-terminated.
    pub new_text: String,
}

impl Repository {
    /// The edit that reverts `hunk` in `relative_path`, computed against
    /// `HEAD`'s copy of the file (via [`Repository::head_blob`]) — the
    /// caller splices this into the live buffer rather than writing
    /// anything to disk.
    pub fn revert_hunk_edit(
        &self,
        relative_path: &Path,
        hunk: &Hunk,
    ) -> Result<TextEdit, VcsError> {
        let before = match self.head_blob(relative_path)? {
            Some((_, text)) => text,
            // A file HEAD has no copy of (new, or HEAD is unborn) can only
            // have a pure-addition hunk against it; reverting deletes
            // everything the hunk added, against an empty "before".
            None => String::new(),
        };
        Ok(revert_hunk(&before, hunk))
    }
}

/// Build the edit that reverts one hunk, given `HEAD`'s text for the file.
///
/// A trailing-newline mismatch at end-of-file is not handled, matching
/// `staging::hunk_patch`'s same simplification and for the same reason:
/// every caller today reads real files, which end in a newline.
pub fn revert_hunk(before: &str, hunk: &Hunk) -> TextEdit {
    let old_lines: Vec<&str> = before.lines().collect();
    let mut new_text = String::new();
    for line in &old_lines[hunk.old.clone()] {
        new_text.push_str(line);
        new_text.push('\n');
    }
    TextEdit {
        start_line: hunk.new.start,
        end_line: hunk.new.end,
        new_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::DiscoverResult;
    use editor_core::diff::diff_lines;
    use std::process::Command;

    /// Apply a [`TextEdit`] to `text`, so a test can assert the *result* of
    /// splicing rather than just the edit's shape.
    fn apply(text: &str, edit: &TextEdit) -> String {
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let replacement: Vec<String> = edit.new_text.lines().map(str::to_string).collect();
        lines.splice(edit.start_line..edit.end_line, replacement);
        let mut out = lines.join("\n");
        if text.ends_with('\n') && !out.is_empty() {
            out.push('\n');
        }
        out
    }

    #[test]
    fn reverting_a_modification_replaces_just_that_line() {
        let before = "one\ntwo\nthree\n";
        let after = "one\nTWO\nthree\n";
        let hunk = diff_lines(before, after).unwrap().remove(0);
        let edit = revert_hunk(before, &hunk);
        assert_eq!(edit.start_line, 1);
        assert_eq!(edit.end_line, 2);
        assert_eq!(edit.new_text, "two\n");
        assert_eq!(apply(after, &edit), before);
    }

    #[test]
    fn reverting_an_addition_deletes_the_added_lines() {
        let before = "a\nc\n";
        let after = "a\nb\nc\n";
        let hunk = diff_lines(before, after).unwrap().remove(0);
        let edit = revert_hunk(before, &hunk);
        assert_eq!(edit.start_line, 1);
        assert_eq!(edit.end_line, 2);
        assert_eq!(edit.new_text, "");
        assert_eq!(apply(after, &edit), before);
    }

    #[test]
    fn reverting_a_deletion_reinserts_the_removed_lines() {
        let before = "a\nb\nc\n";
        let after = "a\nc\n";
        let hunk = diff_lines(before, after).unwrap().remove(0);
        let edit = revert_hunk(before, &hunk);
        assert_eq!(
            edit.start_line, edit.end_line,
            "an insertion targets a point, not a range"
        );
        assert_eq!(edit.new_text, "b\n");
        assert_eq!(apply(after, &edit), before);
    }

    #[test]
    fn reverting_one_of_two_hunks_leaves_the_other_alone() {
        let before = "1\n2\n3\n4\n5\n";
        let after = "1\nX\n3\nY\n5\n";
        let hunks = diff_lines(before, after).unwrap();
        assert_eq!(hunks.len(), 2);

        let edit = revert_hunk(before, &hunks[1]);
        let reverted = apply(after, &edit);
        assert!(reverted.contains('X'), "the first hunk must be untouched");
        assert!(!reverted.contains('Y'), "the second hunk must be reverted");
        assert!(reverted.contains('4'));
    }

    // -----------------------------------------------------------------
    // Repository::revert_hunk_edit, against a real HEAD blob.
    // -----------------------------------------------------------------

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
    fn revert_hunk_edit_reads_head_and_builds_the_edit() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "first"]);

        let working = "one\nTWO\nthree\n";
        let hunk = diff_lines("one\ntwo\nthree\n", working).unwrap().remove(0);

        let repo = open(dir.path());
        let edit = repo.revert_hunk_edit(Path::new("a.txt"), &hunk).unwrap();
        assert_eq!(apply(working, &edit), "one\ntwo\nthree\n");
    }

    #[test]
    fn revert_hunk_edit_on_a_brand_new_file_deletes_everything_the_hunk_added() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);

        let working = "new content\n";
        let hunk = diff_lines("", working).unwrap().remove(0);

        let repo = open(dir.path());
        let edit = repo.revert_hunk_edit(Path::new("new.txt"), &hunk).unwrap();
        assert_eq!(apply(working, &edit), "");
    }
}
