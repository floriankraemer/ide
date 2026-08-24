//! Whole-line operations: duplicate, move, delete and join.
//!
//! Each one is pure — `(text: &str, &SelectionSet) -> Transaction` — and each
//! produces **one** transaction for every caret, so N carets on N lines is one
//! undo entry, not N. The `&str` is the live buffer text passed in by the
//! caller: [`crate::Document`]'s rope is only refreshed on save and is one
//! save behind (see [`crate::selection`]).
//!
//! Carets are grouped into maximal runs of adjacent lines before anything is
//! computed, so three carets on one line duplicate it once and a caret with a
//! selection spanning four lines moves all four as a block. Two runs are
//! always separated by at least one untouched line, which is what keeps
//! `move_up`/`move_down` from producing overlapping edits.
//!
//! Nothing here has an opinion about the caret afterwards:
//! [`crate::transaction::map_carets`] moves the set through the transaction,
//! the same way it does for a keystroke.
//!
//! Line endings are `\n`. CRLF normalisation belongs to the save path, not to
//! a caret operation.

use std::ops::Range;

use crate::offsets::{line_of, line_range, line_starts};
use crate::selection::SelectionSet;
use crate::transaction::{TextEdit, Transaction};

/// Duplicate every touched line (or block of lines), inserting the copy
/// directly below.
pub fn duplicate(text: &str, selection: &SelectionSet) -> Transaction {
    let lines = Lines::new(text);
    let mut edits = Vec::new();
    for run in lines.runs(selection) {
        let block = lines.block(&run);
        let copy = &text[block.clone()];
        // The last line of a file without a trailing newline needs one
        // inserted, or the duplicate lands on the same line as the original.
        let edit = if copy.ends_with('\n') {
            TextEdit::insert(block.end, copy)
        } else {
            TextEdit::insert(block.end, format!("\n{copy}"))
        };
        edits.push(edit);
    }
    Transaction::new(edits)
}

/// Swap every touched line (or block of lines) with the line above it.
///
/// A block already at the top of the text is left alone — a no-op, not an
/// error and not a panic.
pub fn move_up(text: &str, selection: &SelectionSet) -> Transaction {
    let lines = Lines::new(text);
    let mut edits = Vec::new();
    for run in lines.runs(selection) {
        if run.start == 0 {
            continue;
        }
        let above = lines.block(&(run.start - 1..run.start));
        let block = lines.block(&run);
        let (above_text, block_text) = (&text[above.clone()], &text[block.clone()]);
        let moved = if block_text.ends_with('\n') {
            format!("{block_text}{above_text}")
        } else {
            // The block is the last line and carries no newline, so the line
            // that was above it must give up its own.
            format!("{block_text}\n{}", above_text.trim_end_matches('\n'))
        };
        edits.push(TextEdit::new(above.start..block.end, moved));
    }
    Transaction::new(edits)
}

/// Swap every touched line (or block of lines) with the line below it.
///
/// A block already at the bottom is left alone. The empty line a trailing
/// newline implies is not a line anything can be moved past — moving the last
/// real line "down" past it would only add a blank line.
pub fn move_down(text: &str, selection: &SelectionSet) -> Transaction {
    let lines = Lines::new(text);
    let mut edits = Vec::new();
    for run in lines.runs(selection) {
        let last = run.end - 1;
        if last >= lines.last_movable() {
            continue;
        }
        let block = lines.block(&run);
        let below = lines.block(&(run.end..run.end + 1));
        let (block_text, below_text) = (&text[block.clone()], &text[below.clone()]);
        let moved = if below_text.ends_with('\n') {
            format!("{below_text}{block_text}")
        } else {
            format!("{below_text}\n{}", block_text.trim_end_matches('\n'))
        };
        edits.push(TextEdit::new(block.start..below.end, moved));
    }
    Transaction::new(edits)
}

/// Delete every touched line (or block of lines) outright.
pub fn delete(text: &str, selection: &SelectionSet) -> Transaction {
    let lines = Lines::new(text);
    let mut edits = Vec::new();
    for run in lines.runs(selection) {
        let block = lines.block(&run);
        // A block with no trailing newline is the end of the file: take the
        // newline in front of it instead, or deleting the last line leaves a
        // blank one behind.
        let range = if text[block.clone()].ends_with('\n') || block.start == 0 {
            block
        } else {
            block.start - 1..block.end
        };
        edits.push(TextEdit::delete(range));
    }
    Transaction::new(edits)
}

/// Join every touched line with the one below it, separated by a single
/// space.
///
/// A collapsed caret joins its line with the next; a selection joins every
/// line it covers into one. Either way a run produces **one** edit, so an
/// eight-line join is one entry in the undo stack.
///
/// Trailing whitespace on the left and leading whitespace on the right are
/// absorbed — joining an indented block should not leave the indentation
/// stranded mid-line — and a blank line contributes nothing but is still
/// swallowed. Joining the last line is a no-op.
pub fn join(text: &str, selection: &SelectionSet) -> Transaction {
    let lines = Lines::new(text);
    let mut edits = Vec::new();
    for run in lines.runs(selection) {
        let first = run.start;
        if first >= lines.last_movable() {
            continue;
        }
        let last = if run.end - run.start > 1 {
            (run.end - 1).min(lines.last_movable())
        } else {
            first + 1
        };

        let here = lines.range(first);
        let left = text[here.clone()].trim_end();
        let mut joined = String::new();
        for line in first + 1..=last {
            let piece = text[lines.range(line)].trim();
            if piece.is_empty() {
                continue;
            }
            if !left.is_empty() || !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(piece);
        }
        edits.push(TextEdit::new(
            here.start + left.len()..lines.range(last).end,
            joined,
        ));
    }
    Transaction::new(edits)
}

/// Line geometry for one text: where each line starts, and which lines a
/// selection touches.
struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            starts: line_starts(text),
        }
    }

    /// The byte range of `line`, without its newline.
    fn range(&self, line: usize) -> Range<usize> {
        line_range(self.text, &self.starts, line)
    }

    /// The last line an operation may move or join past.
    ///
    /// A text ending in a newline reports one more line than it has content
    /// for. That phantom line is a caret position, not something to swap with.
    fn last_movable(&self) -> usize {
        let last = self.starts.len() - 1;
        if self.text.ends_with('\n') {
            last.saturating_sub(1)
        } else {
            last
        }
    }

    /// The byte range covering lines `run`, including the trailing newline of
    /// its last line when there is one.
    fn block(&self, run: &Range<usize>) -> Range<usize> {
        let start = self.range(run.start).start;
        let end = match self.starts.get(run.end) {
            Some(&next) => next,
            None => self.text.len(),
        };
        start..end
    }

    /// Maximal runs of adjacent lines the selection touches, ascending.
    ///
    /// A selection ending exactly at the start of a line does not include
    /// that line: the user dragged to the line break, not into the next line.
    fn runs(&self, selection: &SelectionSet) -> Vec<Range<usize>> {
        let mut runs: Vec<Range<usize>> = Vec::with_capacity(selection.len());
        for caret in selection.carets() {
            let first = line_of(&self.starts, caret.start().min(self.text.len()));
            let mut last = line_of(&self.starts, caret.end().min(self.text.len()));
            if last > first && caret.end() == self.starts[last] {
                last -= 1;
            }
            match runs.last_mut() {
                // Adjacent as well as overlapping: two carets on neighbouring
                // lines are one block, so moving them cannot produce two edits
                // fighting over the line between.
                Some(prev) if first <= prev.end => prev.end = prev.end.max(last + 1),
                _ => runs.push(first..last + 1),
            }
        }
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Caret;
    use crate::transaction::map_carets;

    fn carets(offsets: &[usize]) -> SelectionSet {
        SelectionSet::from_carets(offsets.iter().copied().map(Caret::at).collect(), 0).unwrap()
    }

    const THREE: &str = "one\ntwo\nthree\n";

    #[test]
    fn duplicate_copies_the_line_below_itself() {
        let tx = duplicate(THREE, &carets(&[5]));
        assert_eq!(tx.edits.len(), 1);
        assert_eq!(tx.apply(THREE).unwrap(), "one\ntwo\ntwo\nthree\n");
    }

    #[test]
    fn duplicate_with_three_carets_is_one_transaction() {
        let tx = duplicate(THREE, &carets(&[0, 4, 8]));
        // Three carets on three adjacent lines are one block, duplicated once.
        assert_eq!(tx.edits.len(), 1);
        assert_eq!(
            tx.apply(THREE).unwrap(),
            "one\ntwo\nthree\none\ntwo\nthree\n"
        );
    }

    #[test]
    fn three_carets_on_non_adjacent_lines_are_one_transaction_with_three_edits() {
        let text = "a\nx\nb\nx\nc\n";
        let tx = duplicate(text, &carets(&[0, 4, 8]));
        assert_eq!(tx.edits.len(), 3);
        assert_eq!(tx.apply(text).unwrap(), "a\na\nx\nb\nb\nx\nc\nc\n");
    }

    #[test]
    fn duplicate_on_a_last_line_without_a_newline_adds_one() {
        let text = "one\ntwo";
        let tx = duplicate(text, &carets(&[5]));
        assert_eq!(tx.apply(text).unwrap(), "one\ntwo\ntwo");
    }

    #[test]
    fn duplicate_of_a_multi_line_selection_copies_the_block() {
        let selection = SelectionSet::single(Caret::new(1, 9));
        let tx = duplicate(THREE, &selection);
        assert_eq!(tx.edits.len(), 1);
        assert_eq!(
            tx.apply(THREE).unwrap(),
            "one\ntwo\nthree\none\ntwo\nthree\n"
        );
    }

    #[test]
    fn a_selection_ending_at_a_line_start_does_not_take_that_line() {
        // 0..4 is "one\n" — the caret sits at the start of line 1, not in it.
        let selection = SelectionSet::single(Caret::new(0, 4));
        let tx = duplicate(THREE, &selection);
        assert_eq!(tx.apply(THREE).unwrap(), "one\none\ntwo\nthree\n");
    }

    #[test]
    fn duplicate_lands_on_char_boundaries_with_multi_byte_text() {
        let text = "héllo\n🙂 wörld\n";
        let tx = duplicate(text, &carets(&[8]));
        assert_eq!(tx.apply(text).unwrap(), "héllo\n🙂 wörld\n🙂 wörld\n");
    }

    // --- move ------------------------------------------------------------

    #[test]
    fn move_up_swaps_with_the_line_above() {
        let tx = move_up(THREE, &carets(&[5]));
        assert_eq!(tx.apply(THREE).unwrap(), "two\none\nthree\n");
    }

    #[test]
    fn move_up_on_the_first_line_is_a_no_op() {
        let tx = move_up(THREE, &carets(&[1]));
        assert!(tx.is_empty());
        assert_eq!(tx.apply(THREE).unwrap(), THREE);
    }

    #[test]
    fn move_up_of_the_last_line_without_a_newline_keeps_the_text_terminated() {
        let text = "one\ntwo";
        let tx = move_up(text, &carets(&[5]));
        assert_eq!(tx.apply(text).unwrap(), "two\none");
    }

    #[test]
    fn move_down_swaps_with_the_line_below() {
        let tx = move_down(THREE, &carets(&[1]));
        assert_eq!(tx.apply(THREE).unwrap(), "two\none\nthree\n");
    }

    #[test]
    fn move_down_on_the_last_line_is_a_no_op() {
        let tx = move_down(THREE, &carets(&[9]));
        assert!(tx.is_empty());
        let text = "one\ntwo";
        let tx = move_down(text, &carets(&[5]));
        assert!(tx.is_empty());
    }

    #[test]
    fn move_down_of_the_line_before_an_unterminated_last_line() {
        let text = "one\ntwo\nthree";
        let tx = move_down(text, &carets(&[5]));
        assert_eq!(tx.apply(text).unwrap(), "one\nthree\ntwo");
    }

    #[test]
    fn moving_a_block_moves_every_line_in_it() {
        let text = "a\nb\nc\nd\n";
        let selection = SelectionSet::single(Caret::new(2, 5));
        let tx = move_down(text, &selection);
        assert_eq!(tx.edits.len(), 1);
        assert_eq!(tx.apply(text).unwrap(), "a\nd\nb\nc\n");
    }

    #[test]
    fn two_separated_carets_move_without_fighting_over_the_line_between() {
        let text = "a\nb\nc\nd\ne\n";
        let tx = move_up(text, &carets(&[2, 8]));
        assert_eq!(tx.edits.len(), 2);
        assert_eq!(tx.apply(text).unwrap(), "b\na\nc\ne\nd\n");
    }

    #[test]
    fn carets_follow_the_line_they_moved_with() {
        let text = "one\ntwo\n";
        let selection = carets(&[5]);
        let tx = move_up(text, &selection);
        let after = map_carets(&selection, &tx);
        assert_eq!(tx.apply(text).unwrap(), "two\none\n");
        // A move rewrites the whole two-line region, so the caret follows the
        // line it was on to that region's start rather than keeping its
        // column. Documented rather than clever: a column-preserving mapping
        // is the view's job once it has the line geometry (F1-15).
        assert_eq!(after.primary(), Caret::at(0));
    }

    // --- delete ----------------------------------------------------------

    #[test]
    fn delete_removes_the_whole_line_including_its_newline() {
        let tx = delete(THREE, &carets(&[5]));
        assert_eq!(tx.apply(THREE).unwrap(), "one\nthree\n");
    }

    #[test]
    fn delete_with_three_carets_is_one_transaction() {
        let text = "a\nx\nb\nx\nc\n";
        let tx = delete(text, &carets(&[0, 4, 8]));
        assert_eq!(tx.edits.len(), 3);
        assert_eq!(tx.apply(text).unwrap(), "x\nx\n");
    }

    #[test]
    fn deleting_an_unterminated_last_line_takes_the_newline_before_it() {
        let text = "one\ntwo";
        let tx = delete(text, &carets(&[5]));
        assert_eq!(tx.apply(text).unwrap(), "one");
    }

    #[test]
    fn deleting_the_only_line_empties_the_text() {
        let text = "only";
        let tx = delete(text, &carets(&[2]));
        assert_eq!(tx.apply(text).unwrap(), "");
    }

    #[test]
    fn delete_lands_on_char_boundaries_with_multi_byte_text() {
        let text = "héllo\n🙂 wörld\nx\n";
        let tx = delete(text, &carets(&[8]));
        assert_eq!(tx.apply(text).unwrap(), "héllo\nx\n");
    }

    // --- join ------------------------------------------------------------

    #[test]
    fn join_pulls_the_next_line_up_with_one_space() {
        let text = "one\n    two\n";
        let tx = join(text, &carets(&[1]));
        assert_eq!(tx.apply(text).unwrap(), "one two\n");
    }

    #[test]
    fn join_absorbs_trailing_whitespace_on_the_first_line() {
        let text = "one   \n  two\n";
        let tx = join(text, &carets(&[1]));
        assert_eq!(tx.apply(text).unwrap(), "one two\n");
    }

    #[test]
    fn join_onto_an_empty_line_adds_no_space() {
        let text = "one\n\n";
        let tx = join(text, &carets(&[1]));
        assert_eq!(tx.apply(text).unwrap(), "one\n");
    }

    #[test]
    fn join_on_the_last_line_is_a_no_op() {
        let tx = join(THREE, &carets(&[9]));
        assert!(tx.is_empty());
        let text = "one\ntwo";
        let tx = join(text, &carets(&[5]));
        assert!(tx.is_empty());
    }

    #[test]
    fn join_over_a_selection_joins_every_line_it_covers() {
        let tx = join(THREE, &SelectionSet::single(Caret::new(0, 9)));
        assert_eq!(tx.edits.len(), 1);
        assert_eq!(tx.apply(THREE).unwrap(), "one two three\n");
    }

    #[test]
    fn join_over_a_blank_line_does_not_produce_overlapping_edits() {
        let text = "one\n   \ntwo\n";
        let tx = join(text, &SelectionSet::single(Caret::new(0, 9)));
        assert_eq!(tx.apply(text).unwrap(), "one two\n");
    }

    #[test]
    fn join_with_three_carets_is_one_transaction() {
        let text = "a\nb\nc\nd\ne\nf\n";
        let tx = join(text, &carets(&[0, 4, 8]));
        assert_eq!(tx.edits.len(), 3);
        assert_eq!(tx.apply(text).unwrap(), "a b\nc d\ne f\n");
    }

    #[test]
    fn join_lands_on_char_boundaries_with_multi_byte_text() {
        let text = "héllo\n  🙂 wörld\n";
        let tx = join(text, &carets(&[0]));
        assert_eq!(tx.apply(text).unwrap(), "héllo 🙂 wörld\n");
    }
}
