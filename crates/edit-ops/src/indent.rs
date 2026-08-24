//! Indentation: what a new line starts with, and what Tab and Shift+Tab do
//! to a selection.
//!
//! # The fallback is the important path
//!
//! **A new line starts with the previous line's indentation.** That is the
//! whole rule when there is no grammar — plain text, a file past the
//! highlight ceiling, a language whose queries would not compile — and it
//! is right often enough that the grammar-driven part is a refinement on
//! top of it, never a replacement for it. An editor that loses the
//! previous line's indent in a plain-text file is broken in a way no
//! clever `{`-handling makes up for.
//!
//! With a tree, two refinements apply, both suppressed inside strings and
//! comments (where a brace is just a character):
//!
//! - a positive bracket balance on the line before the caret adds one
//!   indent unit — the `{`, `(` and `[` case;
//! - a line whose last meaningful character is `:` adds one unit, which is
//!   Python's block opener and YAML's key, and is harmless in C-like
//!   languages where a line rarely ends in a colon.
//!
//! Tab width and spaces-vs-tabs arrive as [`IndentStyle`] rather than being
//! read here: this crate has no settings layer, and F1-10 supplies them
//! from the resolved per-language settings.

use editor_core::offsets::{line_of, line_range, line_starts};
use editor_core::selection::SelectionSet;
use editor_core::transaction::{TextEdit, Transaction};
use syntax_core::Language;

use crate::syntax::{Syntax, Tokens};

/// How this buffer indents. Supplied by the caller; F1-10 resolves it from
/// project, global and per-language settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndentStyle {
    pub tab_width: usize,
    pub use_spaces: bool,
}

impl Default for IndentStyle {
    fn default() -> Self {
        Self {
            tab_width: 4,
            use_spaces: true,
        }
    }
}

impl IndentStyle {
    /// One level of indentation as text.
    pub fn unit(&self) -> String {
        if self.use_spaces {
            " ".repeat(self.tab_width.max(1))
        } else {
            "\t".to_string()
        }
    }
}

/// What a line inserted at `offset` should begin with.
///
/// Returns the indentation only — the caller inserts the newline itself,
/// because whether a newline is `\n` or `\r\n` is the save path's business.
pub fn indent_for_new_line(
    language: Language,
    text: &str,
    offset: usize,
    style: IndentStyle,
) -> String {
    let starts = line_starts(text);
    let line = line_range(text, &starts, line_of(&starts, offset));
    let before = &text[line.start..offset.min(line.end).max(line.start)];
    let base: String = before
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();

    let syntax = Syntax::parse(language, text);
    if !syntax.has_tree() {
        return base;
    }

    let tokens = Tokens::of(language);
    if opens_a_block(&tokens, &syntax, text, line.start, before) {
        return base + &style.unit();
    }
    base
}

/// Indent every line any caret touches by one unit.
pub fn indent_selection(text: &str, selection: &SelectionSet, style: IndentStyle) -> Transaction {
    let starts = line_starts(text);
    let unit = style.unit();
    Transaction::new(
        covered_lines(text, &starts, selection)
            .into_iter()
            .filter(|line| !is_blank(text, &starts, *line))
            .map(|line| TextEdit::insert(starts[line], unit.clone()))
            .collect(),
    )
}

/// Remove up to one unit of leading whitespace from every line any caret
/// touches: one tab, or up to `tab_width` spaces. A line that is already
/// flush left is left alone rather than eating into its text.
pub fn unindent_selection(text: &str, selection: &SelectionSet, style: IndentStyle) -> Transaction {
    let starts = line_starts(text);
    let width = style.tab_width.max(1);
    let mut edits = Vec::new();
    for line in covered_lines(text, &starts, selection) {
        let range = line_range(text, &starts, line);
        let content = &text[range.clone()];
        let removed = if content.starts_with('\t') {
            1
        } else {
            content
                .bytes()
                .take(width)
                .take_while(|b| *b == b' ')
                .count()
        };
        if removed > 0 {
            edits.push(TextEdit::delete(range.start..range.start + removed));
        }
    }
    Transaction::new(edits)
}

/// Whether the text before the caret leaves a block open: an unmatched
/// opening bracket, or a trailing `:`. Positions inside strings and
/// comments do not count, which is the whole reason this needs a tree.
fn opens_a_block(
    tokens: &Tokens,
    syntax: &Syntax,
    text: &str,
    line_start: usize,
    before: &str,
) -> bool {
    let mut depth = 0i32;
    let mut last_meaningful = None;
    for (index, _) in before.char_indices() {
        let at = line_start + index;
        if syntax.in_literal_or_comment(at) {
            continue;
        }
        let rest = &text[at..];
        if let Some((open, _)) = tokens
            .brackets
            .iter()
            .find(|(open, _)| rest.starts_with(open))
        {
            depth += 1;
            last_meaningful = Some(open.clone());
            continue;
        }
        if let Some((_, close)) = tokens
            .brackets
            .iter()
            .find(|(_, close)| rest.starts_with(close))
        {
            depth -= 1;
            last_meaningful = Some(close.clone());
            continue;
        }
        let ch = rest.chars().next().unwrap_or(' ');
        if !ch.is_whitespace() {
            last_meaningful = Some(ch.to_string());
        }
    }
    depth > 0 || last_meaningful.as_deref() == Some(":")
}

fn covered_lines(text: &str, starts: &[usize], selection: &SelectionSet) -> Vec<usize> {
    let mut lines = std::collections::BTreeSet::new();
    for caret in selection.carets() {
        let first = line_of(starts, caret.start());
        let mut last = line_of(starts, caret.end().min(text.len()));
        if last > first && caret.end() == starts[last] {
            last -= 1;
        }
        lines.extend(first..=last);
    }
    lines.into_iter().collect()
}

fn is_blank(text: &str, starts: &[usize], line: usize) -> bool {
    text[line_range(text, starts, line)].trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::selection::{Caret, SelectionSet};
    use syntax_core::language_by_id;

    fn lang(id: &str) -> Language {
        language_by_id(id).unwrap_or_else(|| panic!("{id} is in the catalog"))
    }

    fn spaces() -> IndentStyle {
        IndentStyle::default()
    }

    fn set(carets: &[(usize, usize)]) -> SelectionSet {
        SelectionSet::from_carets(carets.iter().map(|(a, h)| Caret::new(*a, *h)).collect(), 0)
            .expect("within the caret ceiling")
    }

    /// The one that matters most: no grammar, so the previous line's
    /// indent is the entire answer.
    #[test]
    fn without_a_grammar_a_new_line_copies_the_previous_lines_indent() {
        let text = "    hello world\n";
        let at = text.len() - 1;
        assert_eq!(
            indent_for_new_line(Language::PLAIN_TEXT, text, at, spaces()),
            "    "
        );
        assert_eq!(
            indent_for_new_line(Language::PLAIN_TEXT, "no indent", 9, spaces()),
            ""
        );
    }

    #[test]
    fn tabs_are_preserved_verbatim_by_the_fallback() {
        let text = "\t\tvalue\n";
        assert_eq!(
            indent_for_new_line(Language::PLAIN_TEXT, text, 7, spaces()),
            "\t\t"
        );
    }

    #[test]
    fn an_open_brace_adds_one_unit() {
        let text = "fn main() {\n}\n";
        let at = text.find('\n').expect("fixture");
        assert_eq!(
            indent_for_new_line(lang("rust"), text, at, spaces()),
            "    "
        );
    }

    #[test]
    fn a_closed_pair_on_the_same_line_does_not_indent() {
        let text = "    foo(a, b);\n";
        let at = text.find('\n').expect("fixture");
        assert_eq!(
            indent_for_new_line(lang("rust"), text, at, spaces()),
            "    "
        );
    }

    #[test]
    fn a_brace_inside_a_string_or_comment_does_not_indent() {
        let text = "    let s = \"{\";\n";
        let at = text.find('\n').expect("fixture");
        assert_eq!(
            indent_for_new_line(lang("rust"), text, at, spaces()),
            "    "
        );

        let commented = "    // {\n";
        let at = commented.find('\n').expect("fixture");
        assert_eq!(
            indent_for_new_line(lang("rust"), commented, at, spaces()),
            "    "
        );
    }

    #[test]
    fn python_indents_after_a_colon() {
        let text = "def f():\n";
        let at = text.find('\n').expect("fixture");
        assert_eq!(
            indent_for_new_line(lang("python"), text, at, spaces()),
            "    "
        );
    }

    #[test]
    fn the_style_decides_what_one_unit_is() {
        let tabs = IndentStyle {
            tab_width: 8,
            use_spaces: false,
        };
        let text = "fn main() {\n}\n";
        let at = text.find('\n').expect("fixture");
        assert_eq!(indent_for_new_line(lang("rust"), text, at, tabs), "\t");

        let two = IndentStyle {
            tab_width: 2,
            use_spaces: true,
        };
        assert_eq!(indent_for_new_line(lang("rust"), text, at, two), "  ");
    }

    #[test]
    fn indenting_a_selection_adds_one_unit_per_line_and_skips_blanks() {
        let text = "a\n\nb\n";
        let transaction = indent_selection(text, &set(&[(0, text.len())]), spaces());
        assert_eq!(
            transaction.apply(text).expect("applies"),
            "    a\n\n    b\n"
        );
    }

    #[test]
    fn unindenting_removes_at_most_one_unit_and_never_eats_text() {
        let text = "        a\n  b\nc\n";
        let transaction = unindent_selection(text, &set(&[(0, text.len())]), spaces());
        assert_eq!(transaction.apply(text).expect("applies"), "    a\nb\nc\n");
    }

    #[test]
    fn unindenting_a_tab_indented_file_removes_one_tab() {
        let style = IndentStyle {
            tab_width: 4,
            use_spaces: false,
        };
        let text = "\t\ta\n";
        let transaction = unindent_selection(text, &set(&[(0, text.len())]), style);
        assert_eq!(transaction.apply(text).expect("applies"), "\ta\n");
    }

    #[test]
    fn three_carets_indent_in_one_transaction() {
        let text = "a\nb\nc\n";
        let transaction = indent_selection(text, &set(&[(0, 0), (2, 2), (4, 4)]), spaces());
        assert_eq!(transaction.edits.len(), 3);
        assert_eq!(
            transaction.apply(text).expect("applies"),
            "    a\n    b\n    c\n"
        );
    }
}
