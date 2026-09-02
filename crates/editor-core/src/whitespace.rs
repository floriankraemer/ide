//! Classifies the space/tab characters of a single line into leading,
//! inner, and trailing whitespace — the categories JetBrains-style "show
//! whitespaces" toggles independently (task: show-whitespace-characters).
//!
//! Only the ASCII space and tab characters count as whitespace here: that
//! matches the JetBrains feature this mirrors (line-ending whitespace like
//! `\r` never reaches this module, since callers split text into lines
//! first) and keeps the rule simple — no Unicode whitespace-category table
//! to keep in sync with anything.

/// Which part of the line a whitespace character sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhitespaceCategory {
    /// Before the first non-whitespace character.
    Leading,
    /// Between two non-whitespace characters.
    Inner,
    /// After the last non-whitespace character.
    Trailing,
}

/// One space or tab character on a line, classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhitespaceChar {
    /// 0-based character (and, since space/tab are both one UTF-16 code
    /// unit, UTF-16) offset within the line.
    pub column: usize,
    pub is_tab: bool,
    pub category: WhitespaceCategory,
}

/// Classifies every space/tab character in `line` (no trailing newline).
///
/// A line with no non-whitespace character at all — a blank line, or one
/// that is only spaces/tabs — has no "content" for its whitespace to lead
/// up to or trail after, so every character on it is classified as
/// [`WhitespaceCategory::Trailing`]. This matches JetBrains IDEs, which
/// render an indented blank line's whitespace as trailing dots, not
/// leading ones.
pub fn classify_whitespace(line: &str) -> Vec<WhitespaceChar> {
    let is_ws = |c: char| c == ' ' || c == '\t';
    let chars: Vec<char> = line.chars().collect();
    let first_non_ws = chars.iter().position(|&c| !is_ws(c));
    let last_non_ws = chars.iter().rposition(|&c| !is_ws(c));

    // `leading_end` is the first index that is no longer leading;
    // `trailing_start` is the first index that is already trailing. A
    // whitespace-only (or empty) line has neither, so every index counts
    // as trailing per the doc comment above.
    let (leading_end, trailing_start) = match (first_non_ws, last_non_ws) {
        (Some(first), Some(last)) => (first, last + 1),
        _ => (0, 0),
    };

    chars
        .iter()
        .enumerate()
        .filter_map(|(column, &c)| {
            if !is_ws(c) {
                return None;
            }
            let category = if column < leading_end {
                WhitespaceCategory::Leading
            } else if column >= trailing_start {
                WhitespaceCategory::Trailing
            } else {
                WhitespaceCategory::Inner
            };
            Some(WhitespaceChar {
                column,
                is_tab: c == '\t',
                category,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use WhitespaceCategory::*;

    fn chars(spans: &[WhitespaceChar]) -> Vec<(usize, bool, WhitespaceCategory)> {
        spans
            .iter()
            .map(|s| (s.column, s.is_tab, s.category))
            .collect()
    }

    #[test]
    fn empty_line_has_no_whitespace_characters() {
        assert_eq!(classify_whitespace(""), Vec::new());
    }

    #[test]
    fn a_line_with_no_whitespace_at_all_is_untouched() {
        assert_eq!(classify_whitespace("foo"), Vec::new());
    }

    #[test]
    fn an_all_space_line_is_entirely_trailing() {
        assert_eq!(
            chars(&classify_whitespace("   ")),
            vec![
                (0, false, Trailing),
                (1, false, Trailing),
                (2, false, Trailing)
            ]
        );
    }

    #[test]
    fn an_all_tab_line_is_entirely_trailing() {
        assert_eq!(
            chars(&classify_whitespace("\t\t")),
            vec![(0, true, Trailing), (1, true, Trailing)]
        );
    }

    #[test]
    fn leading_whitespace_only() {
        assert_eq!(
            chars(&classify_whitespace("  foo")),
            vec![(0, false, Leading), (1, false, Leading)]
        );
    }

    #[test]
    fn trailing_whitespace_only() {
        assert_eq!(
            chars(&classify_whitespace("foo  ")),
            vec![(3, false, Trailing), (4, false, Trailing)]
        );
    }

    #[test]
    fn leading_and_trailing_together() {
        assert_eq!(
            chars(&classify_whitespace("  foo  ")),
            vec![
                (0, false, Leading),
                (1, false, Leading),
                (5, false, Trailing),
                (6, false, Trailing),
            ]
        );
    }

    #[test]
    fn inner_whitespace_between_words() {
        assert_eq!(
            chars(&classify_whitespace("foo   bar")),
            vec![(3, false, Inner), (4, false, Inner), (5, false, Inner)]
        );
    }

    #[test]
    fn mixed_tabs_and_spaces_in_a_leading_run() {
        assert_eq!(
            chars(&classify_whitespace("\t  foo")),
            vec![(0, true, Leading), (1, false, Leading), (2, false, Leading)]
        );
    }

    #[test]
    fn mixed_leading_inner_and_trailing_with_tabs() {
        assert_eq!(
            chars(&classify_whitespace("\tfoo\tbar \t")),
            vec![
                (0, true, Leading),
                (4, true, Inner),
                (8, false, Trailing),
                (9, true, Trailing),
            ]
        );
    }

    #[test]
    fn a_single_non_whitespace_character_has_no_leading_or_trailing_split() {
        // first_non_ws == last_non_ws == 0: leading_end and trailing_start
        // both land on 0, so nothing before or after it is misclassified.
        assert_eq!(
            chars(&classify_whitespace(" x ")),
            vec![(0, false, Leading), (2, false, Trailing)]
        );
    }

    // No separate "no trailing newline" case: this function only ever sees
    // one already-split line (never containing '\n'), so a file missing a
    // final newline changes nothing about how its last line classifies —
    // covered by every case above.
}
