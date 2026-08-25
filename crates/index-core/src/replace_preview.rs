//! Project-wide Replace in Files: [`TextIndex::replace_in_files`] (the
//! write) and [`TextIndex::preview_replacements`] (the read-only preview
//! that shows what it would do, F3-15), plus the splice logic both share.
//!
//! Split out of `lib.rs` rather than grown into it: that file is
//! grandfathered at a ratcheted line-count ceiling
//! (`scripts/check-file-size.sh`) that may only shrink, so new behavior gets
//! a file of its own from the start.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{FileReplacement, IndexError, ReplaceReport, TextIndex};

/// One file's before/after text for the Replace-in-Files preview, plus the
/// line hunks between them for `DiffView`. Never written to disk —
/// [`TextIndex::replace_in_files`] is the version that writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiffPreview {
    pub path: PathBuf,
    pub old_text: String,
    pub new_text: String,
    pub hunks: Vec<editor_core::diff::Hunk>,
}

impl TextIndex {
    /// Apply `edits` to the files they name and re-index each touched file.
    ///
    /// Spans are the ones `search` produced: `line` is 1-based, `start`/`end`
    /// are byte offsets within that line. Edits are grouped per file and
    /// applied back-to-front (last line first, rightmost span first) so that
    /// earlier spans keep their recorded offsets.
    ///
    /// A file whose lines no longer contain the recorded spans — it changed
    /// between the search and the replace — is skipped whole and counted in
    /// [`ReplaceReport::skipped_files`], never partially rewritten.
    ///
    /// Open editor tabs need no special handling: the write lands on disk and
    /// the existing watcher -> `check_external_change` flow prompts affected
    /// tabs to reload, the same as any other outside-the-editor change.
    pub fn replace_in_files(
        &mut self,
        edits: &[FileReplacement],
    ) -> Result<ReplaceReport, IndexError> {
        let mut report = ReplaceReport::default();
        for (path, file_edits) in group_by_file(edits) {
            let count = file_edits.len();
            let Some((_old, new_text)) = spliced_content(path, file_edits) else {
                report.skipped_files += 1;
                continue;
            };
            if fs::write(path, new_text).is_err() {
                report.skipped_files += 1;
                continue;
            }
            report.files += 1;
            report.matches += count;
            self.reindex_file(path)?;
        }
        Ok(report)
    }

    /// [`Self::replace_in_files`], without writing anything: the before and
    /// after text of every file the edits would touch, plus the line hunks
    /// between them, for the Replace-in-Files preview.
    ///
    /// A file that cannot be read or whose spans no longer fit is left out
    /// entirely, the same "skip the whole file" rule `replace_in_files`
    /// applies — a preview that showed a file the real replace would skip
    /// would be showing a change that never happens.
    pub fn preview_replacements(&self, edits: &[FileReplacement]) -> Vec<FileDiffPreview> {
        group_by_file(edits)
            .into_iter()
            .filter_map(|(path, file_edits)| {
                let (old_text, new_text) = spliced_content(path, file_edits)?;
                let hunks = editor_core::diff::diff_lines(&old_text, &new_text).unwrap_or_default();
                Some(FileDiffPreview {
                    path: path.to_path_buf(),
                    old_text,
                    new_text,
                    hunks,
                })
            })
            .collect()
    }
}

/// Group `edits` by the file they touch, preserving nothing about order
/// beyond what [`BTreeMap`] gives — both callers re-sort per file before
/// applying anything.
fn group_by_file(edits: &[FileReplacement]) -> BTreeMap<&Path, Vec<&FileReplacement>> {
    let mut by_file: BTreeMap<&Path, Vec<&FileReplacement>> = BTreeMap::new();
    for edit in edits {
        by_file.entry(edit.path.as_path()).or_default().push(edit);
    }
    by_file
}

/// Splice `file_edits` (all belonging to `path`) into that file's current
/// on-disk content, without writing it back. `None` when the file cannot be
/// read, or any span no longer fits the line it names — the file changed
/// since the edits were computed.
///
/// Spans are the ones `search` produced: `line` is 1-based, `start`/`end`
/// are byte offsets within that line. Applied back-to-front (last line
/// first, rightmost span first) so earlier spans keep their recorded
/// offsets.
fn spliced_content(path: &Path, mut file_edits: Vec<&FileReplacement>) -> Option<(String, String)> {
    let content = fs::read_to_string(path).ok()?;
    // `split_inclusive` keeps each line's terminator, so re-joining
    // preserves the file's original line endings and trailing newline.
    let mut lines: Vec<String> = content.split_inclusive('\n').map(String::from).collect();

    file_edits.sort_by(|a, b| b.line.cmp(&a.line).then(b.start.cmp(&a.start)));
    let applicable = file_edits.iter().all(|e| {
        e.line >= 1
            && e.start <= e.end
            && lines
                .get(e.line - 1)
                .is_some_and(|l| e.end <= l.trim_end_matches(['\n', '\r']).len())
    });
    if !applicable {
        return None;
    }

    for edit in &file_edits {
        lines[edit.line - 1].replace_range(edit.start..edit.end, &edit.text);
    }
    Some((content, lines.concat()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let path = dir.join(rel);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn replace_in_files_rewrites_every_span_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "one two one\nkeep me\none\n");
        let mut index = TextIndex::build(dir.path()).unwrap();

        let edits: Vec<FileReplacement> = index
            .search("one", false, true)
            .unwrap()
            .into_iter()
            .map(|m| FileReplacement {
                path: m.path,
                line: m.line,
                start: m.start,
                end: m.end,
                text: "1".into(),
            })
            .collect();
        let report = index.replace_in_files(&edits).unwrap();

        assert_eq!(report.files, 1);
        assert_eq!(report.matches, 3);
        assert_eq!(report.skipped_files, 0);
        assert_eq!(fs::read_to_string(&file).unwrap(), "1 two 1\nkeep me\n1\n");
        // The index followed the write, so the old text is gone from it.
        assert!(index.search("one", false, true).unwrap().is_empty());
    }

    #[test]
    fn replace_in_files_skips_a_file_that_changed_since_the_search() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "needle here\n");
        let mut index = TextIndex::build(dir.path()).unwrap();
        let matches = index.search("needle", false, true).unwrap();

        fs::write(&file, "x\n").unwrap();
        let edits: Vec<FileReplacement> = matches
            .into_iter()
            .map(|m| FileReplacement {
                path: m.path,
                line: m.line,
                start: m.start,
                end: m.end,
                text: "pin".into(),
            })
            .collect();
        let report = index.replace_in_files(&edits).unwrap();

        assert_eq!(report.files, 0);
        assert_eq!(report.skipped_files, 1);
        assert_eq!(fs::read_to_string(&file).unwrap(), "x\n");
    }

    #[test]
    fn preview_replacements_does_not_write_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "one two one\n");
        let index = TextIndex::build(dir.path()).unwrap();

        let edits: Vec<FileReplacement> = index
            .search("one", false, true)
            .unwrap()
            .into_iter()
            .map(|m| FileReplacement {
                path: m.path,
                line: m.line,
                start: m.start,
                end: m.end,
                text: "1".into(),
            })
            .collect();
        let previews = index.preview_replacements(&edits);

        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].old_text, "one two one\n");
        assert_eq!(previews[0].new_text, "1 two 1\n");
        // Untouched: `preview_replacements` never opens the file for
        // writing.
        assert_eq!(fs::read_to_string(&file).unwrap(), "one two one\n");
    }

    #[test]
    fn preview_replacements_reports_hunks_between_old_and_new() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "needle\nkeep\n");
        let index = TextIndex::build(dir.path()).unwrap();

        let edits: Vec<FileReplacement> = index
            .search("needle", false, true)
            .unwrap()
            .into_iter()
            .map(|m| FileReplacement {
                path: m.path,
                line: m.line,
                start: m.start,
                end: m.end,
                text: "pin".into(),
            })
            .collect();
        let previews = index.preview_replacements(&edits);

        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].hunks.len(), 1);
        assert_eq!(
            previews[0].hunks[0].kind,
            editor_core::diff::HunkKind::Modified
        );
    }

    #[test]
    fn preview_replacements_skips_a_file_that_changed_since_the_search() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "needle here\n");
        let index = TextIndex::build(dir.path()).unwrap();
        let matches = index.search("needle", false, true).unwrap();

        fs::write(&file, "x\n").unwrap();
        let edits: Vec<FileReplacement> = matches
            .into_iter()
            .map(|m| FileReplacement {
                path: m.path,
                line: m.line,
                start: m.start,
                end: m.end,
                text: "pin".into(),
            })
            .collect();

        assert!(index.preview_replacements(&edits).is_empty());
    }
}
