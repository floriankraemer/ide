//! One user-visible change to a buffer, however many carets made it.
//!
//! A [`Transaction`] is the whole of what a keystroke does: N carets typing
//! one character is one transaction carrying N edits, not N edits applied one
//! after another. That is the point of the type — it is what makes the change
//! one undo entry at the seam, and it is why edit offsets never shift under
//! each other.
//!
//! Application is **descending by offset and all-or-nothing**, deliberately
//! the same discipline `lsp_core::workspace_edit::apply_to_text` already uses:
//! every edit is validated before any text is written, then applied last
//! first so earlier offsets stay valid. A transaction that would fail partway
//! leaves the text untouched.
//!
//! Like the rest of the caret machinery, every entry point takes `text: &str`
//! rather than a [`crate::Document`] — the rope is only refreshed on save and
//! is one save behind the live buffer.

use std::ops::Range;

use crate::offsets::clamp_to_boundary;
use crate::selection::{Caret, SelectionSet};

/// A byte range and the text that replaces it. An empty range is a pure
/// insertion, which is what typing at a collapsed caret produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
}

impl TextEdit {
    pub fn new(range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }

    /// A pure insertion at `offset`.
    pub fn insert(offset: usize, text: impl Into<String>) -> Self {
        Self::new(offset..offset, text)
    }

    /// A pure deletion of `range`.
    pub fn delete(range: Range<usize>) -> Self {
        Self::new(range, "")
    }

    /// How much longer the text becomes when this edit is applied.
    fn delta(&self) -> isize {
        self.text.len() as isize - (self.range.end as isize - self.range.start as isize)
    }
}

/// Why a transaction could not be applied. Every variant refuses the whole
/// transaction: half a multi-caret edit is a corrupted buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionError {
    /// An edit names an offset past the end of the text, or a range whose
    /// end precedes its start.
    RangeOutOfBounds,
    /// An edit's offset falls inside a multi-byte character. Applying it
    /// would produce text that is not valid UTF-8.
    NotOnCharBoundary,
    /// Two edits overlap, so the result would depend on the order they were
    /// applied in. Two insertions at the same point do not overlap.
    OverlappingEdits,
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionError::RangeOutOfBounds => {
                write!(f, "the edit does not fit the text — it changed underneath")
            }
            TransactionError::NotOnCharBoundary => {
                write!(f, "the edit would split a character")
            }
            TransactionError::OverlappingEdits => write!(f, "two of the edits overlap"),
        }
    }
}

impl std::error::Error for TransactionError {}

/// Every edit one user action makes, applied as a unit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transaction {
    pub edits: Vec<TextEdit>,
}

impl Transaction {
    pub fn new(edits: Vec<TextEdit>) -> Self {
        Self { edits }
    }

    /// A transaction that changes nothing — what a line operation returns
    /// when it has nothing to do (moving the first line up, joining the last
    /// line). A no-op, never a panic and never a partial edit.
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Apply to `text`, returning the new text.
    ///
    /// Validates every edit first, then applies them descending, so a
    /// transaction that cannot be applied changes nothing at all.
    pub fn apply(&self, text: &str) -> Result<String, TransactionError> {
        let mut order: Vec<usize> = (0..self.edits.len()).collect();
        for edit in &self.edits {
            if edit.range.start > edit.range.end || edit.range.end > text.len() {
                return Err(TransactionError::RangeOutOfBounds);
            }
            if !text.is_char_boundary(edit.range.start) || !text.is_char_boundary(edit.range.end) {
                return Err(TransactionError::NotOnCharBoundary);
            }
        }

        // Last first, so earlier offsets stay valid as we go. Ties keep the
        // order they were given in, which is the order N carets were sorted
        // into — two insertions at one point are not an error.
        order.sort_by(|a, b| {
            self.edits[*b]
                .range
                .start
                .cmp(&self.edits[*a].range.start)
                // Ties are two insertions at one point: applying the later
                // one first leaves them in the order they were given.
                .then(b.cmp(a))
        });
        for pair in order.windows(2) {
            let later = &self.edits[pair[0]];
            let earlier = &self.edits[pair[1]];
            if earlier.range.end > later.range.start {
                return Err(TransactionError::OverlappingEdits);
            }
        }

        let mut out = text.to_string();
        for index in order {
            let edit = &self.edits[index];
            out.replace_range(edit.range.clone(), &edit.text);
        }
        Ok(out)
    }

    /// Typing `typed` at every caret: each caret's selection (if any) is
    /// replaced, each collapsed caret gets an insertion.
    pub fn type_text(selection: &SelectionSet, typed: &str) -> Self {
        Self::new(
            selection
                .carets()
                .iter()
                .map(|caret| TextEdit::new(caret.range(), typed))
                .collect(),
        )
    }

    /// Backspace at every caret: a caret with a selection deletes it, a
    /// collapsed caret deletes the character before it.
    ///
    /// Because carets are normalised first, two carets one byte apart delete
    /// two different characters rather than the same one twice — the classic
    /// multi-caret double-delete, which only appears when carets are handled
    /// one at a time against a text that is already changing.
    pub fn backspace(text: &str, selection: &SelectionSet) -> Self {
        let mut edits = Vec::with_capacity(selection.len());
        for caret in selection.carets() {
            if !caret.is_empty() {
                edits.push(TextEdit::delete(caret.range()));
                continue;
            }
            let at = clamp_to_boundary(text, caret.head);
            let Some(prev) = text[..at].chars().next_back() else {
                continue;
            };
            edits.push(TextEdit::delete(at - prev.len_utf8()..at));
        }
        Self::new(edits)
    }
}

/// Where the carets end up once `transaction` has been applied.
///
/// The rules, in the order they matter:
///
/// - an offset before every edit is unchanged;
/// - an offset at or after an edit shifts by that edit's length change;
/// - a **collapsed** caret at a pure insertion rides to the end of the
///   inserted text, which is what makes typing across N carets feel like
///   typing;
/// - a caret whose selection an edit replaced wholesale **collapses to the
///   end of the replacement** — typing over a selection leaves a cursor after
///   what was typed, not a selection around it;
/// - a caret **inside** a range some other caret deleted collapses onto that
///   range's start, where normalisation merges it with whatever is already
///   there. It therefore disappears from the set rather than surviving as a
///   caret pointing into text that no longer exists.
///
/// The set is normalised on the way out, so the result is sorted, merged and
/// still holds at least one caret.
pub fn map_carets(selection: &SelectionSet, transaction: &Transaction) -> SelectionSet {
    let primary = selection.primary_index();
    let mapped: Vec<Caret> = selection
        .carets()
        .iter()
        .map(|caret| match replacement_of(caret, transaction) {
            Some(edit) => {
                let at = map_offset(caret.start(), transaction) + edit.text.len();
                Caret::at(at)
            }
            None => Caret::new(
                map_offset(caret.anchor, transaction),
                map_offset(caret.head, transaction),
            ),
        })
        .collect();
    SelectionSet::from_carets(mapped, primary)
        .expect("mapping never adds a caret, so the ceiling cannot be crossed")
}

/// The edit that replaced this caret's selection outright, if there is one.
fn replacement_of<'a>(caret: &Caret, transaction: &'a Transaction) -> Option<&'a TextEdit> {
    if caret.is_empty() {
        return None;
    }
    transaction
        .edits
        .iter()
        .find(|edit| edit.range == caret.range())
}

fn map_offset(offset: usize, transaction: &Transaction) -> usize {
    let mut shifted = offset as isize;
    for edit in &transaction.edits {
        let (start, end) = (edit.range.start, edit.range.end);
        if offset < start {
            continue;
        }
        if offset == start && start == end {
            // Pure insertion at the caret: ride to the end of what was typed.
            shifted += edit.text.len() as isize;
            continue;
        }
        if offset >= end {
            shifted += edit.delta();
            continue;
        }
        // Strictly inside a replaced range: collapse to its start, undoing
        // any shift the edits before it in this loop applied to it.
        shifted -= offset as isize - start as isize;
    }
    shifted.max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carets(offsets: &[usize]) -> SelectionSet {
        SelectionSet::from_carets(offsets.iter().copied().map(Caret::at).collect(), 0).unwrap()
    }

    fn spans(s: &SelectionSet) -> Vec<(usize, usize)> {
        s.carets().iter().map(|c| (c.start(), c.end())).collect()
    }

    #[test]
    fn three_carets_typing_is_one_transaction() {
        let text = "aa bb cc";
        let selection = carets(&[0, 3, 6]);
        let tx = Transaction::type_text(&selection, "X");
        assert_eq!(tx.edits.len(), 3);
        assert_eq!(tx.apply(text).unwrap(), "Xaa Xbb Xcc");
    }

    #[test]
    fn carets_survive_their_own_insert_and_land_after_it() {
        let text = "aa bb cc";
        let selection = carets(&[0, 3, 6]);
        let tx = Transaction::type_text(&selection, "X");
        let after = map_carets(&selection, &tx);
        assert_eq!(spans(&after), vec![(1, 1), (5, 5), (9, 9)]);
        let new_text = tx.apply(text).unwrap();
        for caret in after.carets() {
            assert_eq!(&new_text[caret.head - 1..caret.head], "X");
        }
    }

    #[test]
    fn typing_over_selections_replaces_each_one() {
        let text = "one two three";
        let selection =
            SelectionSet::from_carets(vec![Caret::new(0, 3), Caret::new(4, 7)], 0).unwrap();
        let tx = Transaction::type_text(&selection, "X");
        assert_eq!(tx.apply(text).unwrap(), "X X three");
        assert_eq!(spans(&map_carets(&selection, &tx)), vec![(1, 1), (3, 3)]);
    }

    #[test]
    fn adjacent_carets_backspacing_do_not_double_delete() {
        // The classic: carets at 5 and 6 must delete two different
        // characters, leaving the text two shorter, not three.
        let text = "abcdefgh";
        let selection = carets(&[5, 6]);
        let tx = Transaction::backspace(text, &selection);
        assert_eq!(tx.edits.len(), 2);
        assert_eq!(tx.apply(text).unwrap(), "abcdgh");
        // Both carets end up in the same place, so they merge — which is the
        // point: one caret, one character deleted each, nothing doubled.
        assert_eq!(spans(&map_carets(&selection, &tx)), vec![(4, 4)]);
    }

    #[test]
    fn backspace_at_the_start_of_the_text_does_nothing_for_that_caret() {
        let text = "abc";
        let selection = carets(&[0, 2]);
        let tx = Transaction::backspace(text, &selection);
        assert_eq!(tx.edits.len(), 1);
        assert_eq!(tx.apply(text).unwrap(), "ac");
    }

    #[test]
    fn backspace_removes_a_whole_multi_byte_character() {
        let text = "a🙂b";
        let selection = carets(&[5]);
        let tx = Transaction::backspace(text, &selection);
        assert_eq!(tx.apply(text).unwrap(), "ab");
    }

    #[test]
    fn edits_apply_right_to_left_so_earlier_offsets_stay_valid() {
        // Given in ascending order on purpose: if they were applied in that
        // order the second range would already have moved.
        let text = "0123456789";
        let tx = Transaction::new(vec![
            TextEdit::new(1..3, "LONGER"),
            TextEdit::new(6..8, "X"),
        ]);
        assert_eq!(tx.apply(text).unwrap(), "0LONGER345X89");
    }

    #[test]
    fn a_caret_inside_a_deleted_range_is_dropped() {
        let text = "abcdefghij";
        let selection = carets(&[2, 5, 8]);
        // One caret deletes 2..8, swallowing the caret at 5.
        let tx = Transaction::new(vec![TextEdit::delete(2..8)]);
        let after = map_carets(&selection, &tx);
        assert_eq!(spans(&after), vec![(2, 2)]);
        assert_eq!(tx.apply(text).unwrap(), "abij");
    }

    #[test]
    fn overlapping_edits_are_refused_and_the_text_is_untouched() {
        let text = "abcdefgh";
        let tx = Transaction::new(vec![TextEdit::new(1..5, "X"), TextEdit::new(3..7, "Y")]);
        assert_eq!(tx.apply(text), Err(TransactionError::OverlappingEdits));
        assert_eq!(text, "abcdefgh");
    }

    #[test]
    fn two_insertions_at_the_same_point_are_not_an_overlap() {
        let tx = Transaction::new(vec![TextEdit::insert(2, "X"), TextEdit::insert(2, "Y")]);
        assert_eq!(tx.apply("abcd").unwrap(), "abXYcd");
    }

    #[test]
    fn a_transaction_that_would_fail_partway_leaves_the_text_untouched() {
        let text = "abcdef";
        // The first edit is fine; the second is past the end. Nothing is
        // written, so the caller still holds the original text.
        let tx = Transaction::new(vec![TextEdit::new(0..1, "X"), TextEdit::new(10..12, "Y")]);
        assert_eq!(tx.apply(text), Err(TransactionError::RangeOutOfBounds));
        assert_eq!(tx.apply(text), Err(TransactionError::RangeOutOfBounds));
        assert_eq!(text, "abcdef");
    }

    #[test]
    fn an_edit_splitting_a_character_is_refused() {
        let tx = Transaction::new(vec![TextEdit::new(1..2, "x")]);
        assert_eq!(tx.apply("a🙂"), Err(TransactionError::NotOnCharBoundary));
    }

    #[test]
    fn a_reversed_range_is_refused() {
        // Built by hand: `4..2` as a literal is a clippy error, and the
        // point is exactly that a caller can still hand us one.
        let reversed = TextEdit {
            range: Range { start: 4, end: 2 },
            text: "x".into(),
        };
        let tx = Transaction::new(vec![reversed]);
        assert_eq!(tx.apply("abcdef"), Err(TransactionError::RangeOutOfBounds));
    }

    #[test]
    fn an_empty_transaction_is_the_identity() {
        assert!(Transaction::empty().is_empty());
        assert_eq!(Transaction::empty().apply("abc").unwrap(), "abc");
    }

    #[test]
    fn carets_after_an_edit_they_did_not_make_shift_by_its_delta() {
        let selection = carets(&[0, 9]);
        let tx = Transaction::new(vec![TextEdit::new(2..4, "LONGER")]);
        assert_eq!(spans(&map_carets(&selection, &tx)), vec![(0, 0), (13, 13)]);
    }

    #[test]
    fn a_selection_keeps_its_direction_through_a_mapping() {
        let selection = SelectionSet::single(Caret::new(9, 4));
        let tx = Transaction::new(vec![TextEdit::insert(0, "ab")]);
        assert_eq!(map_carets(&selection, &tx).primary(), Caret::new(11, 6));
    }
}
