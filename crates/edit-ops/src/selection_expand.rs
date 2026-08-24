//! Expand and shrink a selection along the node tree.
//!
//! Expanding is a pure function of the tree; **shrinking is not**. The node
//! tree cannot say which of a node's descendants the user came from, so
//! shrink retraces a stack of what expand returned rather than guessing.
//! That is why this module has a type and not just two functions.
//!
//! The stack is discarded the moment the selection stops being the one the
//! last expansion produced — a click, an arrow key or an edit all end the
//! sequence, and the next Ctrl+W starts a fresh one.
//!
//! Without a tree (plain text, a file past the highlight ceiling) the
//! sequence is **word → line → whole file**, which is the shape users
//! expect and is better than refusing.

use editor_core::offsets::{line_of, line_range, line_starts};
use editor_core::selection::{Caret, SelectionSet};
use syntax_core::Language;

use crate::syntax::Syntax;

/// The selection stack behind Ctrl+W / Ctrl+Shift+W.
///
/// One per editor tab, held by the caller: it is state about a gesture in
/// progress, not about the document.
#[derive(Debug, Default)]
pub struct SelectionHistory {
    stack: Vec<SelectionSet>,
    /// What the last expansion or shrink handed back, so a selection the
    /// user changed by other means can be recognised and the stack
    /// dropped.
    last: Option<SelectionSet>,
}

impl SelectionHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget the stack. The caller calls this on any edit — an expansion
    /// recorded against text that has since changed would restore a range
    /// that no longer means anything.
    pub fn clear(&mut self) {
        self.stack.clear();
        self.last = None;
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// The next larger selection. Returns `current` unchanged when there is
    /// nothing larger left (the whole file is already selected).
    pub fn expand(
        &mut self,
        language: Language,
        text: &str,
        current: &SelectionSet,
    ) -> SelectionSet {
        if self.last.as_ref() != Some(current) {
            self.clear();
        }
        let syntax = Syntax::parse(language, text);
        let starts = line_starts(text);
        let expanded = SelectionSet::from_carets(
            current
                .carets()
                .iter()
                .map(|caret| expand_caret(&syntax, text, &starts, *caret))
                .collect(),
            current.primary_index(),
        )
        .unwrap_or_else(|_| current.clone());

        if &expanded == current {
            return expanded;
        }
        self.stack.push(current.clone());
        self.last = Some(expanded.clone());
        expanded
    }

    /// The selection the last [`expand`](Self::expand) grew from, or `None`
    /// when this is not an expansion the history knows about — in which
    /// case the caller leaves the selection alone rather than inventing a
    /// smaller one.
    pub fn shrink(&mut self, current: &SelectionSet) -> Option<SelectionSet> {
        if self.last.as_ref() != Some(current) {
            self.clear();
            return None;
        }
        let previous = self.stack.pop()?;
        self.last = Some(previous.clone());
        Some(previous)
    }
}

fn expand_caret(syntax: &Syntax, text: &str, starts: &[usize], caret: Caret) -> Caret {
    let (start, end) = (caret.start(), caret.end());
    if let Some(range) = syntax.enclosing_range(start, end) {
        return Caret::new(range.start, range.end);
    }

    // No tree: word, then line, then everything.
    let word = word_range(text, start, end);
    if let Some(word) = word {
        if word.0 <= start && word.1 >= end && (word.0, word.1) != (start, end) {
            return Caret::new(word.0, word.1);
        }
    }
    let line = line_range(text, starts, line_of(starts, start));
    if line.start <= start && line.end >= end && (line.start, line.end) != (start, end) {
        return Caret::new(line.start, line.end);
    }
    Caret::new(0, text.len())
}

/// The word `start..end` sits in, if it sits in one.
fn word_range(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let left = text[..start]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map(|(index, _)| index)
        .unwrap_or(start);
    let right = text[end..]
        .char_indices()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map(|(index, c)| end + index + c.len_utf8())
        .unwrap_or(end);
    (left < right).then_some((left, right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use syntax_core::language_by_id;

    fn rust() -> Language {
        language_by_id("rust").expect("rust is in the catalog")
    }

    fn at(offset: usize) -> SelectionSet {
        SelectionSet::single(Caret::at(offset))
    }

    fn span(selection: &SelectionSet) -> (usize, usize) {
        let caret = selection.primary();
        (caret.start(), caret.end())
    }

    #[test]
    fn expanding_walks_out_through_the_node_tree() {
        let text = "fn main() { let value = 1 + 2; }";
        let mut history = SelectionHistory::new();
        let caret = at(text.find("value").expect("fixture") + 2);

        let mut selection = history.expand(rust(), text, &caret);
        assert_eq!(&text[selection.primary().range()], "value");

        let mut seen = vec![span(&selection)];
        for _ in 0..8 {
            selection = history.expand(rust(), text, &selection);
            seen.push(span(&selection));
        }
        // Every step is strictly larger than the one before, and the last
        // one is the whole file.
        for pair in seen.windows(2) {
            assert!(
                pair[1] == pair[0] || pair[1].0 <= pair[0].0 && pair[1].1 >= pair[0].1,
                "{pair:?} did not grow"
            );
        }
        assert_eq!(seen.last(), Some(&(0, text.len())));
    }

    #[test]
    fn shrinking_retraces_expansion_exactly() {
        let text = "fn main() { let value = 1 + 2; }";
        let mut history = SelectionHistory::new();
        let start = at(text.find("value").expect("fixture") + 2);

        let mut steps = vec![start.clone()];
        let mut selection = start.clone();
        loop {
            let expanded = history.expand(rust(), text, &selection);
            if expanded == selection {
                // The whole file is selected; there is nothing above it.
                break;
            }
            selection = expanded;
            steps.push(selection.clone());
        }
        assert!(steps.len() > 3, "the fixture should nest deeper than that");
        for previous in steps.iter().rev().skip(1) {
            selection = history.shrink(&selection).expect("stack has the step");
            assert_eq!(&selection, previous);
        }
        assert_eq!(selection, start);
        assert_eq!(history.depth(), 0);
    }

    #[test]
    fn shrinking_a_selection_the_history_did_not_make_is_refused() {
        let mut history = SelectionHistory::new();
        assert!(history.shrink(&at(3)).is_none());
    }

    #[test]
    fn moving_the_caret_between_expansions_starts_a_new_sequence() {
        let text = "fn main() { let value = 1 + 2; }";
        let mut history = SelectionHistory::new();
        let expanded = history.expand(rust(), text, &at(15));
        assert_eq!(history.depth(), 1);

        // The user clicked elsewhere; the old stack must not survive it.
        let elsewhere = history.expand(rust(), text, &at(3));
        assert_eq!(history.depth(), 1);
        assert_ne!(span(&expanded), span(&elsewhere));
    }

    #[test]
    fn without_a_tree_the_sequence_is_word_then_line_then_all() {
        let text = "alpha beta\ngamma\n";
        let mut history = SelectionHistory::new();
        let plain = Language::PLAIN_TEXT;

        let word = history.expand(plain, text, &at(7));
        assert_eq!(&text[word.primary().range()], "beta");

        let line = history.expand(plain, text, &word);
        assert_eq!(&text[line.primary().range()], "alpha beta");

        let all = history.expand(plain, text, &line);
        assert_eq!(span(&all), (0, text.len()));

        // And it stops there rather than looping.
        let again = history.expand(plain, text, &all);
        assert_eq!(span(&again), (0, text.len()));
    }

    #[test]
    fn every_caret_expands_in_a_multi_caret_set() {
        let text = "alpha beta\n";
        let mut history = SelectionHistory::new();
        let carets =
            SelectionSet::from_carets(vec![Caret::at(1), Caret::at(7)], 0).expect("two carets");
        let expanded = history.expand(Language::PLAIN_TEXT, text, &carets);
        assert_eq!(expanded.len(), 2);
        assert_eq!(&text[expanded.carets()[0].range()], "alpha");
        assert_eq!(&text[expanded.carets()[1].range()], "beta");
    }
}
