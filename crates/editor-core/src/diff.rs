//! Line and intra-line differences between two texts.
//!
//! Deliberately Git-free. A diff is two texts and what changed between them,
//! and the four places this IDE needs one are not all about version control:
//! the rename preview, the project-wide replace preview, the AI's apply flow
//! and the VCS gutter. Putting this in a `vcs-core` would mean a rename
//! preview needed a repository to show a diff, and a project with no
//! repository got none.
//!
//! # What this replaces
//!
//! The refactor preview currently shows the first line of the replacement
//! text, trimmed and truncated to 80 characters. That is what a user is
//! offered before agreeing to rewrite files across their project, and the
//! project-wide replace has no preview and no undo at all.
//!
//! # Ceilings
//!
//! Diffing is linear in the texts, but the gutter runs it on every keystroke,
//! so callers get [`MAX_DIFF_BYTES`] to decide against rather than a
//! surprise. Past it, say so — a gutter that quietly shows no markers on a
//! large file is indistinguishable from one that thinks nothing changed.

use std::ops::Range;

use imara_diff::{Algorithm, Diff, InternedInput};

/// Texts above this are not diffed. Matches the highlighting ceiling in
/// `syntax-core`, so a file that is too big to colour is also too big to
/// mark up — one threshold for the user to understand rather than two.
pub const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;

/// What happened to a run of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    Added,
    Removed,
    Modified,
}

/// A run of changed lines, as 0-based half-open line ranges into each side.
///
/// An empty `old` means the lines were added; an empty `new` means they were
/// removed; both non-empty means modified. The kind is precomputed because
/// every consumer wants it and deriving it from two empty-range checks at
/// each call site is how they end up disagreeing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub kind: HunkKind,
}

/// A changed span **within** a line, as byte offsets into that line.
///
/// This is what makes a diff readable when someone renamed one identifier on
/// a 200-character line: without it the whole line is highlighted and the
/// reader has to find the difference themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSpan {
    /// 0-based line number on the side this span belongs to.
    pub line: usize,
    pub range: Range<usize>,
}

/// Intra-line detail for one modified hunk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineDiff {
    pub removed: Vec<InlineSpan>,
    pub added: Vec<InlineSpan>,
}

/// Why a diff was not produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffError {
    /// One of the texts is past [`MAX_DIFF_BYTES`]. Callers must say so
    /// rather than showing an empty diff, which reads as "nothing changed".
    TooLarge,
}

/// Line-level hunks between `before` and `after`.
pub fn diff_lines(before: &str, after: &str) -> Result<Vec<Hunk>, DiffError> {
    if before.len() > MAX_DIFF_BYTES || after.len() > MAX_DIFF_BYTES {
        return Err(DiffError::TooLarge);
    }
    let input = InternedInput::new(before, after);
    let diff = Diff::compute(Algorithm::Histogram, &input);
    Ok(diff
        .hunks()
        .map(|h| {
            let old = h.before.start as usize..h.before.end as usize;
            let new = h.after.start as usize..h.after.end as usize;
            let kind = match (old.is_empty(), new.is_empty()) {
                (true, false) => HunkKind::Added,
                (false, true) => HunkKind::Removed,
                _ => HunkKind::Modified,
            };
            Hunk { old, new, kind }
        })
        .collect())
}

/// Intra-line spans for a modified hunk, by word.
///
/// Only meaningful when the hunk has the same number of lines on each side —
/// otherwise there is no line-to-line correspondence to compare, and
/// inventing one produces worse output than highlighting the whole hunk.
/// Returns an empty diff in that case, which callers render as a whole-line
/// change.
pub fn diff_inline(before: &str, after: &str, hunk: &Hunk) -> InlineDiff {
    if hunk.kind != HunkKind::Modified || hunk.old.len() != hunk.new.len() {
        return InlineDiff::default();
    }
    let old_lines: Vec<&str> = before.lines().collect();
    let new_lines: Vec<&str> = after.lines().collect();

    let mut out = InlineDiff::default();
    for (offset, (old_line, new_line)) in hunk.old.clone().zip(hunk.new.clone()).enumerate() {
        let _ = offset;
        let (Some(old_text), Some(new_text)) = (old_lines.get(old_line), new_lines.get(new_line))
        else {
            continue;
        };
        let (removed, added) = word_spans(old_text, new_text);
        if !removed.is_empty() {
            out.removed.push(InlineSpan {
                line: old_line,
                range: removed,
            });
        }
        if !added.is_empty() {
            out.added.push(InlineSpan {
                line: new_line,
                range: added,
            });
        }
    }
    out
}

/// Trim the common prefix and suffix of two lines, and report what is left.
///
/// A word diff proper would be better on a heavily rewritten line, but this
/// is the case that actually matters — one identifier renamed, one argument
/// added — and it costs nothing. The prefix and suffix are trimmed on
/// **character** boundaries, so a multi-byte character is never split.
fn word_spans(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
    if old == new {
        return (0..0, 0..0);
    }
    let prefix = old
        .char_indices()
        .zip(new.char_indices())
        .take_while(|((_, a), (_, b))| a == b)
        .map(|((i, c), _)| i + c.len_utf8())
        .last()
        .unwrap_or(0);

    let mut suffix = 0;
    let old_tail = &old[prefix..];
    let new_tail = &new[prefix..];
    for (a, b) in old_tail.chars().rev().zip(new_tail.chars().rev()) {
        if a != b {
            break;
        }
        suffix += a.len_utf8();
    }
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;

    // Snap outward to word boundaries.
    //
    // Without this, renaming `alpha` to `beta` highlights `alph` and `bet`,
    // because the two share a trailing "a" that the suffix trim eats. That is
    // the minimal span and it is unreadable: the eye expects a renamed
    // identifier to light up as a word, not as a word minus one letter.
    let (old_start, old_end) = snap_to_words(old, prefix, old_end.max(prefix));
    let (new_start, new_end) = snap_to_words(new, prefix, new_end.max(prefix));
    (old_start..old_end, new_start..new_end)
}

/// Widen `start..end` to cover whole words where it already cuts through one.
///
/// Only extends across word characters, so punctuation-only changes stay
/// tight — `f(a)` to `f(a, b)` still highlights `, b` rather than swallowing
/// the identifier beside it.
fn snap_to_words(text: &str, start: usize, end: usize) -> (usize, usize) {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';

    let mut start = start;
    while start > 0 {
        let prev = text[..start].chars().next_back().unwrap_or(' ');
        let next = text[start..].chars().next().unwrap_or(' ');
        if is_word(prev) && is_word(next) {
            start -= prev.len_utf8();
        } else {
            break;
        }
    }

    let mut end = end;
    while end < text.len() {
        let prev = text[..end].chars().next_back().unwrap_or(' ');
        let next = text[end..].chars().next().unwrap_or(' ');
        if is_word(prev) && is_word(next) {
            end += next.len_utf8();
        } else {
            break;
        }
    }
    (start, end)
}

/// Apply one hunk's *before* side back over `after`, undoing it.
///
/// This is the operation behind "revert this hunk" in the gutter, and it is
/// also the strongest invariant available for testing a diff: reverting every
/// hunk of a diff must reproduce the original text exactly. Hunks are applied
/// in descending order so earlier line numbers stay valid.
pub fn revert_hunks(before: &str, after: &str, hunks: &[Hunk]) -> String {
    let old_lines: Vec<&str> = before.lines().collect();
    let mut new_lines: Vec<String> = after.lines().map(str::to_string).collect();

    for hunk in hunks.iter().rev() {
        let replacement: Vec<String> = old_lines[hunk.old.clone()]
            .iter()
            .map(|s| s.to_string())
            .collect();
        new_lines.splice(hunk.new.clone(), replacement);
    }

    let mut out = new_lines.join("\n");
    // `lines()` drops the trailing terminator, so it is restored from the
    // text the result is meant to equal. Getting this wrong is how a revert
    // silently strips or adds a final newline.
    if before.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(before: &str, after: &str) -> Vec<HunkKind> {
        diff_lines(before, after)
            .unwrap()
            .iter()
            .map(|h| h.kind)
            .collect()
    }

    #[test]
    fn identical_texts_have_no_hunks() {
        assert!(diff_lines("a\nb\n", "a\nb\n").unwrap().is_empty());
    }

    #[test]
    fn a_pure_insertion_is_added() {
        assert_eq!(kinds("a\nc\n", "a\nb\nc\n"), vec![HunkKind::Added]);
    }

    #[test]
    fn a_pure_deletion_is_removed() {
        assert_eq!(kinds("a\nb\nc\n", "a\nc\n"), vec![HunkKind::Removed]);
    }

    #[test]
    fn a_replacement_is_modified() {
        assert_eq!(kinds("a\nb\nc\n", "a\nB\nc\n"), vec![HunkKind::Modified]);
    }

    #[test]
    fn insertion_at_the_start_and_end_are_found() {
        assert_eq!(kinds("b\n", "a\nb\n"), vec![HunkKind::Added]);
        assert_eq!(kinds("a\n", "a\nb\n"), vec![HunkKind::Added]);
    }

    #[test]
    fn a_file_that_becomes_empty_and_one_that_gains_content() {
        assert_eq!(kinds("a\nb\n", ""), vec![HunkKind::Removed]);
        assert_eq!(kinds("", "a\nb\n"), vec![HunkKind::Added]);
    }

    #[test]
    fn hunks_are_ascending_and_do_not_overlap() {
        let before = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let after = "1\nX\n3\n4\nY\n6\n7\nZ\n";
        let hunks = diff_lines(before, after).unwrap();
        assert!(hunks.len() >= 2);
        for pair in hunks.windows(2) {
            assert!(
                pair[0].new.end <= pair[1].new.start,
                "hunks overlap: {:?}",
                pair
            );
            assert!(pair[0].old.end <= pair[1].old.start);
        }
    }

    /// The strongest invariant available: undoing every hunk must reproduce
    /// the original exactly. Worth more than the individual shape tests
    /// above, because it holds for inputs nobody thought to enumerate.
    #[test]
    fn reverting_every_hunk_reproduces_the_before_text() {
        let cases = [
            ("a\nb\nc\n", "a\nB\nc\n"),
            ("a\nc\n", "a\nb\nc\n"),
            ("a\nb\nc\n", "a\nc\n"),
            ("", "a\n"),
            ("a\n", ""),
            ("one\ntwo\nthree\nfour\n", "one\n2\nthree\n4\nfive\n"),
            ("x", "y"),
            ("a\nb\nc\nd\ne\nf\n", "a\nc\ne\n"),
            ("é\n中\n🙂\n", "é\nCHANGED\n🙂\n"),
            ("no trailing newline", "no trailing newline!"),
        ];
        for (before, after) in cases {
            let hunks = diff_lines(before, after).unwrap();
            assert_eq!(
                revert_hunks(before, after, &hunks),
                before,
                "reverting {after:?} did not reproduce {before:?}"
            );
        }
    }

    #[test]
    fn reverting_one_hunk_leaves_the_others_correctly_offset() {
        let before = "1\n2\n3\n4\n5\n";
        let after = "1\nX\n3\nY\n5\n";
        let hunks = diff_lines(before, after).unwrap();
        assert_eq!(hunks.len(), 2, "expected two separate hunks");

        // Revert only the second; the first must be untouched.
        let reverted = revert_hunks(before, after, &hunks[1..]);
        assert!(reverted.contains("X"), "the first hunk was reverted too");
        assert!(!reverted.contains("Y"), "the second hunk was not reverted");
    }

    #[test]
    fn crlf_is_not_mistaken_for_a_change() {
        // Same content, same terminators: no hunks. (Mixed terminators are a
        // real difference and are reported as one, which is correct.)
        assert!(diff_lines("a\r\nb\r\n", "a\r\nb\r\n").unwrap().is_empty());
    }

    #[test]
    fn a_whitespace_only_change_is_still_a_change() {
        assert_eq!(kinds("a\n", "a \n"), vec![HunkKind::Modified]);
    }

    #[test]
    fn a_text_past_the_ceiling_is_refused_rather_than_reported_as_unchanged() {
        let big = "x\n".repeat(MAX_DIFF_BYTES);
        assert_eq!(diff_lines(&big, "y\n"), Err(DiffError::TooLarge));
        assert_eq!(diff_lines("y\n", &big), Err(DiffError::TooLarge));
    }

    // -----------------------------------------------------------------
    // Intra-line
    // -----------------------------------------------------------------

    #[test]
    fn one_renamed_identifier_narrows_to_that_word() {
        let before = "let alpha = compute(1);\n";
        let after = "let beta = compute(1);\n";
        let hunks = diff_lines(before, after).unwrap();
        let inline = diff_inline(before, after, &hunks[0]);
        assert_eq!(inline.removed.len(), 1);
        assert_eq!(&before[inline.removed[0].range.clone()], "alpha");
        assert_eq!(&after[inline.added[0].range.clone()], "beta");
    }

    #[test]
    fn an_appended_argument_narrows_to_the_tail() {
        let before = "f(a)\n";
        let after = "f(a, b)\n";
        let hunks = diff_lines(before, after).unwrap();
        let inline = diff_inline(before, after, &hunks[0]);
        assert_eq!(&after[inline.added[0].range.clone()], ", b");
    }

    #[test]
    fn intra_line_spans_never_split_a_multi_byte_character() {
        let before = "let 中文 = 1;\n";
        let after = "let 中文 = 2;\n";
        let hunks = diff_lines(before, after).unwrap();
        let inline = diff_inline(before, after, &hunks[0]);
        for span in inline.removed.iter().chain(&inline.added) {
            assert!(
                before.is_char_boundary(span.range.start)
                    || after.is_char_boundary(span.range.start),
                "span starts mid-character"
            );
        }
    }

    /// A hunk whose sides have different line counts has no line-to-line
    /// correspondence, so there is nothing honest to narrow to.
    #[test]
    fn an_uneven_hunk_has_no_intra_line_detail() {
        let before = "a\nb\n";
        let after = "a\nb1\nb2\n";
        let hunks = diff_lines(before, after).unwrap();
        assert_eq!(diff_inline(before, after, &hunks[0]), InlineDiff::default());
    }
}
