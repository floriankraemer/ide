//! What a file is tidied into on the way to disk: trailing whitespace, the
//! final newline, and the line terminator.
//!
//! Every entry point is a pure function over `&str` returning a
//! [`Transaction`], for two reasons that matter more than they look:
//!
//! - The save path already knows how to splice a transaction into the live
//!   buffer, so save rules take the same route every other edit takes. There
//!   is no second write path that could disagree with it about offsets or
//!   about dirty state.
//! - A transaction is one undo entry. Save rules are therefore **their own**
//!   entry, separate from whatever the user typed last: one Ctrl+Z after a
//!   save undoes the tidying, a second undoes the edit. Undoing both at once
//!   is the behaviour that makes people turn these settings off.
//!
//! # Where the carets go
//!
//! Nothing here moves a caret itself. The caller maps its
//! [`SelectionSet`](crate::SelectionSet) through
//! [`map_carets`](crate::map_carets), exactly as it does for a typed
//! character, and the answers fall out of the rules already written there:
//!
//! - A caret on a line that was not touched does not move.
//! - A caret **inside** whitespace that was trimmed collapses to the start of
//!   the deleted range — which is the end of the line's text. That is the
//!   kind answer: the caret stays where the user was working. Letting it fall
//!   to column 0 is the bug this note exists to prevent.
//! - Every caret survives; N carets on N different lines all land at their
//!   own line ends.
//!
//! # The line the caret is on is trimmed too
//!
//! Some editors leave it alone so that typing a space and hitting save keeps
//! the space. This one does not, deliberately: whether a saved file contains
//! trailing whitespace would then depend on where the cursor happened to be,
//! which produces exactly the "why does this line have a diff" noise the
//! setting exists to remove. The caret is pinned to the end of the line, so
//! nothing about the user's position is lost — only bytes nobody can see.

use crate::transaction::{TextEdit, Transaction};

/// A line terminator. `\r` alone is not one: it has not been a line ending
/// on any platform this IDE runs on for two decades, and treating a stray
/// carriage return as a terminator would rewrite binary-ish text nobody
/// asked us to touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }

    /// What this platform writes when nothing else says otherwise.
    pub const fn platform() -> Self {
        if cfg!(windows) {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        }
    }
}

/// The terminator a file already uses, taken from its first one. `None` for a
/// file with no line ending at all, where there is nothing to preserve.
///
/// First rather than most-common on purpose: a mixed file is being repaired
/// by whichever rule the caller applies next, and "what the top of the file
/// does" is both cheaper and easier to explain than a vote.
pub fn detect_line_ending(text: &str) -> Option<LineEnding> {
    let index = text.find('\n')?;
    if text[..index].ends_with('\r') {
        Some(LineEnding::Crlf)
    } else {
        Some(LineEnding::Lf)
    }
}

/// Which tidying a save performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveRules {
    pub trim_trailing_whitespace: bool,
    pub insert_final_newline: bool,
    /// The terminator to normalise to, or `None` to keep what the file
    /// already uses. `None` is the default: rewriting every line of a file
    /// the user merely opened is a whole-file diff nobody asked for.
    pub line_endings: Option<LineEnding>,
}

impl Default for SaveRules {
    fn default() -> Self {
        Self {
            trim_trailing_whitespace: true,
            insert_final_newline: true,
            line_endings: None,
        }
    }
}

/// One line: where its text starts, and the byte range of its terminator.
/// The terminator range is empty for a last line that has none.
struct Line {
    start: usize,
    terminator: std::ops::Range<usize>,
}

fn lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0;
    for (index, _) in text.match_indices('\n') {
        let terminator_start = if index > start && text.as_bytes()[index - 1] == b'\r' {
            index - 1
        } else {
            index
        };
        out.push(Line {
            start,
            terminator: terminator_start..index + 1,
        });
        start = index + 1;
    }
    if start < text.len() {
        out.push(Line {
            start,
            terminator: text.len()..text.len(),
        });
    }
    out
}

/// Where this line's trailing whitespace begins — the end of its text.
///
/// `\r` and `\n` are excluded, and the terminator is outside the searched
/// range anyway: trimming a CRLF file must not eat the carriage return and
/// turn every line ending into an LF as a side effect.
fn text_end(text: &str, line: &Line) -> usize {
    let content = &text[line.start..line.terminator.start];
    line.start
        + content
            .trim_end_matches(|c: char| c.is_whitespace() && c != '\n' && c != '\r')
            .len()
}

fn trim_edits(text: &str, lines: &[Line]) -> Vec<TextEdit> {
    lines
        .iter()
        .filter_map(|line| {
            let end = text_end(text, line);
            (end < line.terminator.start).then(|| TextEdit::delete(end..line.terminator.start))
        })
        .collect()
}

fn terminator_edits(text: &str, lines: &[Line], target: LineEnding) -> Vec<TextEdit> {
    lines
        .iter()
        .filter(|line| {
            !line.terminator.is_empty() && &text[line.terminator.clone()] != target.as_str()
        })
        .map(|line| TextEdit::new(line.terminator.clone(), target.as_str()))
        .collect()
}

/// Delete the trailing whitespace on every line.
pub fn trim_trailing_whitespace(text: &str) -> Transaction {
    Transaction::new(trim_edits(text, &lines(text)))
}

/// Rewrite every terminator that is not already `target`.
pub fn normalize_line_endings(text: &str, target: LineEnding) -> Transaction {
    Transaction::new(terminator_edits(text, &lines(text), target))
}

/// End the file with a newline, if it has content and does not already.
///
/// An empty file stays empty: a file with no lines does not need a last one,
/// and creating a file whose only content is a newline is not "tidying".
pub fn insert_final_newline(text: &str, ending: LineEnding) -> Transaction {
    if text.is_empty() || text.ends_with('\n') {
        return Transaction::empty();
    }
    Transaction::new(vec![TextEdit::insert(text.len(), ending.as_str())])
}

/// Every rule the settings turned on, as one transaction.
///
/// Composed rather than applied one after another, so the whole tidying is a
/// single undo entry — and so trimming and the final newline cannot fight.
/// A file ending `"  \n  "` is the case that catches a naive composition:
/// trimming leaves it ending in a newline, so the final-newline rule must not
/// then append a second one. The rules are therefore resolved against what
/// the trim will leave behind, never against the original tail.
///
/// Returns an empty transaction when there is nothing to do, so saving an
/// already-clean file never marks it dirty.
pub fn on_save(text: &str, rules: &SaveRules) -> Transaction {
    let lines = lines(text);
    let target = rules
        .line_endings
        .or_else(|| detect_line_ending(text))
        .unwrap_or_else(LineEnding::platform);

    let mut edits = if rules.trim_trailing_whitespace {
        trim_edits(text, &lines)
    } else {
        Vec::new()
    };
    // Whitespace being trimmed off the very end of the file: after the trim,
    // the text stops here.
    let end_after_trim = edits
        .iter()
        .find(|edit| edit.range.end == text.len())
        .map_or(text.len(), |edit| edit.range.start);

    if let Some(target) = rules.line_endings {
        edits.extend(terminator_edits(text, &lines, target));
    }

    if rules.insert_final_newline && end_after_trim > 0 && !text[..end_after_trim].ends_with('\n') {
        match edits
            .iter_mut()
            .find(|edit| edit.range.start == end_after_trim && edit.range.end == text.len())
        {
            // The trailing whitespace becomes the newline instead of being
            // deleted: one edit, so nothing overlaps at the end of the file.
            Some(trailing) => trailing.text = target.as_str().to_string(),
            None => edits.push(TextEdit::insert(text.len(), target.as_str())),
        }
    }

    Transaction::new(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::{Caret, SelectionSet};
    use crate::transaction::map_carets;

    fn carets(offsets: &[usize]) -> SelectionSet {
        SelectionSet::from_carets(offsets.iter().copied().map(Caret::at).collect(), 0).unwrap()
    }

    fn heads(selection: &SelectionSet) -> Vec<usize> {
        selection.carets().iter().map(|c| c.head).collect()
    }

    #[test]
    fn trailing_whitespace_goes_from_every_line() {
        let text = "one   \ntwo\t\nthree";
        let tx = trim_trailing_whitespace(text);
        assert_eq!(tx.apply(text).unwrap(), "one\ntwo\nthree");
    }

    #[test]
    fn a_clean_file_is_not_dirtied_by_rules_with_nothing_to_do() {
        let text = "clean\nfile\n";
        assert!(on_save(text, &SaveRules::default()).is_empty());
        assert!(trim_trailing_whitespace(text).is_empty());
        assert!(insert_final_newline(text, LineEnding::Lf).is_empty());
        assert!(normalize_line_endings(text, LineEnding::Lf).is_empty());
    }

    #[test]
    fn a_caret_on_an_untouched_line_does_not_move() {
        let text = "keep me\ntrailing   \n";
        let selection = carets(&[3]);
        let tx = trim_trailing_whitespace(text);
        assert_eq!(heads(&map_carets(&selection, &tx)), vec![3]);
    }

    #[test]
    fn a_caret_inside_the_trimmed_whitespace_pins_to_the_end_of_its_line() {
        // "text   " with the caret two spaces in: it must land after "text",
        // not at column 0.
        let text = "text   \nnext\n";
        let selection = carets(&[6]);
        let tx = trim_trailing_whitespace(text);
        let after = map_carets(&selection, &tx);
        assert_eq!(heads(&after), vec![4]);
        let trimmed = tx.apply(text).unwrap();
        assert_eq!(&trimmed[..after.primary().head], "text");
    }

    #[test]
    fn every_caret_survives_a_trim_of_every_line() {
        let text = "aa  \nbb  \ncc  \n";
        // One caret inside each line's whitespace.
        let selection = carets(&[3, 8, 13]);
        let tx = trim_trailing_whitespace(text);
        let after = map_carets(&selection, &tx);
        assert_eq!(after.len(), 3);
        assert_eq!(tx.apply(text).unwrap(), "aa\nbb\ncc\n");
        assert_eq!(heads(&after), vec![2, 5, 8]);
    }

    #[test]
    fn the_line_the_caret_is_on_is_trimmed_like_any_other() {
        // The deliberate choice: a saved file's contents do not depend on
        // where the cursor was.
        let text = "typing here   ";
        let selection = carets(&[14]);
        let tx = on_save(text, &SaveRules::default());
        assert_eq!(tx.apply(text).unwrap(), "typing here\n");
        assert_eq!(heads(&map_carets(&selection, &tx)), vec![12]);
    }

    #[test]
    fn trimming_is_its_own_transaction_after_the_users_edit() {
        // Two transactions, so two undo entries: undoing the save rules must
        // not take the typed word with it.
        let original = "hello";
        let typed = Transaction::new(vec![TextEdit::insert(5, " world   ")]);
        let after_typing = typed.apply(original).unwrap();
        assert_eq!(after_typing, "hello world   ");

        let saved = on_save(&after_typing, &SaveRules::default());
        assert!(!saved.is_empty());
        assert_eq!(saved.apply(&after_typing).unwrap(), "hello world\n");
        // Undoing only the save rules is dropping that second transaction,
        // which leaves the typing intact.
        assert_eq!(after_typing, "hello world   ");
    }

    #[test]
    fn trimming_and_the_final_newline_do_not_fight_over_a_whitespace_tail() {
        // The classic double-newline: the trim already leaves this ending in
        // one, so nothing more may be appended.
        let text = "  \n  ";
        let tx = on_save(text, &SaveRules::default());
        assert_eq!(tx.apply(text).unwrap(), "\n");
    }

    #[test]
    fn a_trailing_whitespace_run_after_content_becomes_the_final_newline() {
        let text = "line one\nline two   ";
        let tx = on_save(text, &SaveRules::default());
        assert_eq!(tx.apply(text).unwrap(), "line one\nline two\n");
    }

    #[test]
    fn an_empty_file_gains_nothing() {
        assert!(on_save("", &SaveRules::default()).is_empty());
        assert!(insert_final_newline("", LineEnding::Lf).is_empty());
    }

    #[test]
    fn a_file_of_nothing_but_whitespace_is_emptied_not_newlined() {
        let text = "   ";
        let tx = on_save(text, &SaveRules::default());
        assert_eq!(tx.apply(text).unwrap(), "");
    }

    #[test]
    fn trimming_a_crlf_file_leaves_the_carriage_returns_alone() {
        let text = "one   \r\ntwo\t\r\n";
        let tx = trim_trailing_whitespace(text);
        assert_eq!(tx.apply(text).unwrap(), "one\r\ntwo\r\n");
    }

    #[test]
    fn a_crlf_file_keeps_crlf_when_a_final_newline_is_added() {
        let text = "one\r\ntwo";
        let tx = on_save(text, &SaveRules::default());
        assert_eq!(tx.apply(text).unwrap(), "one\r\ntwo\r\n");
    }

    #[test]
    fn normalising_rewrites_only_the_terminators_that_differ() {
        let text = "one\r\ntwo\nthree\r\n";
        assert_eq!(
            normalize_line_endings(text, LineEnding::Lf)
                .apply(text)
                .unwrap(),
            "one\ntwo\nthree\n"
        );
        assert_eq!(
            normalize_line_endings(text, LineEnding::Crlf)
                .apply(text)
                .unwrap(),
            "one\r\ntwo\r\nthree\r\n"
        );
    }

    #[test]
    fn normalising_and_trimming_apply_in_one_transaction() {
        let text = "one   \r\ntwo  ";
        let rules = SaveRules {
            line_endings: Some(LineEnding::Lf),
            ..SaveRules::default()
        };
        let tx = on_save(text, &rules);
        assert_eq!(tx.apply(text).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn a_caret_survives_line_ending_normalisation() {
        let text = "one\r\ntwo\r\n";
        // On "two", which shifts back by the one byte the first CRLF loses.
        let selection = carets(&[6]);
        let tx = normalize_line_endings(text, LineEnding::Lf);
        assert_eq!(heads(&map_carets(&selection, &tx)), vec![5]);
    }

    #[test]
    fn rules_that_are_switched_off_do_nothing() {
        let text = "trailing   \nno newline at the end   ";
        let rules = SaveRules {
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            line_endings: None,
        };
        assert!(on_save(text, &rules).is_empty());
    }

    #[test]
    fn the_final_newline_alone_does_not_trim() {
        let text = "keep my spaces   ";
        let rules = SaveRules {
            trim_trailing_whitespace: false,
            ..SaveRules::default()
        };
        assert_eq!(
            on_save(text, &rules).apply(text).unwrap(),
            "keep my spaces   \n"
        );
    }

    #[test]
    fn the_terminator_of_a_file_without_one_falls_back_to_the_platform() {
        assert_eq!(detect_line_ending("no newline"), None);
        assert_eq!(detect_line_ending("a\nb"), Some(LineEnding::Lf));
        assert_eq!(detect_line_ending("a\r\nb"), Some(LineEnding::Crlf));
        let tx = on_save("no newline", &SaveRules::default());
        assert_eq!(
            tx.apply("no newline").unwrap(),
            format!("no newline{}", LineEnding::platform().as_str())
        );
    }
}
