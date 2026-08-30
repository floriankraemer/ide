//! Find & replace matching over a plain text buffer.
//!
//! This is the *rule* half of the editor's Ctrl+F / Ctrl+R: what counts as
//! a match, and what text a match is replaced with. The Qt view only paints
//! the spans this module returns and applies the replacement strings it is
//! handed — no matching decision lives in C++.
//!
//! Offsets are **UTF-16 code units**, not bytes and not chars, because the
//! only consumer positions a `QTextCursor` with them and Qt indexes text in
//! UTF-16. The conversion happens here, once per call, rather than at the
//! FFI seam.
//!
//! Note the buffer is passed in by the caller: `Document`'s rope is stale
//! between saves (live keystrokes never reach it), so in-editor search must
//! run over the widget's current text, the same way `AppSession::save_tab`
//! takes the content to write.

use regex::{Regex, RegexBuilder};

use crate::offsets::Utf16Cursor;

/// How a pattern is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchOptions {
    /// `false` = the pattern is a literal substring, `true` = a regex in
    /// the `regex` crate's syntax (what Find in Files already uses).
    pub regex: bool,
    pub case_sensitive: bool,
}

/// One match, as a half-open `[start, end)` range of UTF-16 code units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMatch {
    pub start: usize,
    pub end: usize,
}

/// One match plus the text that should take its place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    /// The pattern did not compile; the string is the regex crate's own
    /// message, which is already user-readable.
    InvalidPattern(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::InvalidPattern(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SearchError {}

/// Every match of `pattern` in `hay`, in document order.
pub fn find_matches(
    hay: &str,
    pattern: &str,
    opts: SearchOptions,
) -> Result<Vec<TextMatch>, SearchError> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let re = compile(pattern, opts)?;

    let mut cursor = Utf16Cursor::new(hay);
    let mut out = Vec::new();
    for m in re.find_iter(hay) {
        // A zero-length match (`a*`, `^`, `\b`) has nothing to highlight or
        // replace, and stepping onto one would let Replace All insert text
        // at every position in the buffer.
        if m.start() == m.end() {
            continue;
        }
        out.push(TextMatch {
            start: cursor.utf16_at(m.start()),
            end: cursor.utf16_at(m.end()),
        });
    }
    Ok(out)
}

/// Every match of `pattern` in `hay` paired with its replacement text.
///
/// In regex mode `replacement` may reference capture groups (`$1`,
/// `${name}`); in literal mode it is used verbatim, so a user replacing
/// with `$1` gets the two characters they typed.
pub fn replacements(
    hay: &str,
    pattern: &str,
    replacement: &str,
    opts: SearchOptions,
) -> Result<Vec<Replacement>, SearchError> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let re = compile(pattern, opts)?;

    let mut cursor = Utf16Cursor::new(hay);
    let mut out = Vec::new();
    for caps in re.captures_iter(hay) {
        let whole = caps.get(0).expect("group 0 always participates");
        if whole.start() == whole.end() {
            continue;
        }
        let mut text = String::new();
        if opts.regex {
            caps.expand(&brace_numbered_groups(replacement), &mut text);
        } else {
            text.push_str(replacement);
        }
        out.push(Replacement {
            start: cursor.utf16_at(whole.start()),
            end: cursor.utf16_at(whole.end()),
            text,
        });
    }
    Ok(out)
}

/// The replacements one Replace or Replace All gesture splices, in the
/// order the view must apply them: **descending**.
///
/// Descending is load-bearing. The view re-resolves every span against the
/// document as it splices, so an ascending list would have each replacement
/// after the first land at an offset its predecessor already moved.
///
/// `only` names a single match by its position in document order — the
/// Replace-this-one gesture, whose index is the one the match counter
/// shows; `None` takes every match. An index past the end selects nothing,
/// which is what a stale counter should produce rather than a panic.
pub fn replacement_edits(
    hay: &str,
    pattern: &str,
    replacement: &str,
    opts: SearchOptions,
    only: Option<usize>,
) -> Result<Vec<Replacement>, SearchError> {
    let mut items = replacements(hay, pattern, replacement, opts)?;
    if let Some(index) = only {
        items = if index < items.len() {
            vec![items.remove(index)]
        } else {
            Vec::new()
        };
    }
    items.reverse();
    Ok(items)
}

/// Rewrites `$1` into `${1}` so a numbered group reference followed by
/// word characters expands the way every other editor's replace box does.
/// `regex`'s own `expand` would otherwise read `$1_new` as a reference to a
/// group *named* `1_new`, which never exists, and silently expand to "".
/// `$$` (a literal dollar) and `$name` are passed through untouched.
fn brace_numbered_groups(replacement: &str) -> String {
    let mut out = String::with_capacity(replacement.len());
    let mut chars = replacement.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('0'..='9') => {
                let mut digits = String::new();
                while let Some(d) = chars.peek().filter(|c| c.is_ascii_digit()) {
                    digits.push(*d);
                    chars.next();
                }
                out.push_str("${");
                out.push_str(&digits);
                out.push('}');
            }
            Some('$') => {
                out.push_str("$$");
                chars.next();
            }
            _ => out.push('$'),
        }
    }
    out
}

fn compile(pattern: &str, opts: SearchOptions) -> Result<Regex, SearchError> {
    let owned;
    let source = if opts.regex {
        pattern
    } else {
        owned = regex::escape(pattern);
        &owned
    };
    RegexBuilder::new(source)
        .case_insensitive(!opts.case_sensitive)
        .build()
        .map_err(|e| SearchError::InvalidPattern(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(regex: bool, case_sensitive: bool) -> SearchOptions {
        SearchOptions {
            regex,
            case_sensitive,
        }
    }

    #[test]
    fn replacement_edits_come_back_descending() {
        let edits = replacement_edits("a a a", "a", "bb", opts(false, true), None).unwrap();
        let starts: Vec<usize> = edits.iter().map(|r| r.start).collect();
        assert_eq!(starts, vec![4, 2, 0]);
    }

    #[test]
    fn replacement_edits_can_select_one_match_by_document_order() {
        let edits = replacement_edits("a a a", "a", "bb", opts(false, true), Some(1)).unwrap();
        assert_eq!(
            edits,
            vec![Replacement {
                start: 2,
                end: 3,
                text: "bb".to_string(),
            }]
        );
    }

    #[test]
    fn replacement_edits_select_nothing_past_the_last_match() {
        let edits = replacement_edits("a a", "a", "b", opts(false, true), Some(7)).unwrap();
        assert!(edits.is_empty());
    }

    #[test]
    fn replacement_edits_of_a_pattern_that_matches_nothing_are_empty() {
        let edits = replacement_edits("abc", "zzz", "b", opts(false, true), None).unwrap();
        assert!(edits.is_empty());
    }

    #[test]
    fn literal_search_is_not_a_regex() {
        let matches = find_matches("a.c abc", "a.c", opts(false, true)).unwrap();
        assert_eq!(matches, vec![TextMatch { start: 0, end: 3 }]);
    }

    #[test]
    fn regex_mode_matches_every_occurrence() {
        let matches = find_matches("foo1 foo2", r"foo\d", opts(true, true)).unwrap();
        assert_eq!(
            matches,
            vec![
                TextMatch { start: 0, end: 4 },
                TextMatch { start: 5, end: 9 }
            ]
        );
    }

    #[test]
    fn case_sensitivity_is_honoured_both_ways() {
        assert!(find_matches("Foo", "foo", opts(false, true))
            .unwrap()
            .is_empty());
        assert_eq!(
            find_matches("Foo", "foo", opts(false, false)).unwrap(),
            vec![TextMatch { start: 0, end: 3 }]
        );
    }

    #[test]
    fn empty_pattern_matches_nothing() {
        assert!(find_matches("anything", "", opts(false, true))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn zero_length_matches_are_skipped() {
        // Without the guard this yields a match at every position.
        assert!(find_matches("abc", "x*", opts(true, true))
            .unwrap()
            .is_empty());
        assert_eq!(
            find_matches("aab", "a*", opts(true, true)).unwrap(),
            vec![TextMatch { start: 0, end: 2 }]
        );
    }

    #[test]
    fn offsets_are_utf16_code_units() {
        // "é" is 2 bytes / 1 UTF-16 unit; "😀" is 4 bytes / 2 UTF-16 units.
        let matches = find_matches("é😀target", "target", opts(false, true)).unwrap();
        assert_eq!(matches, vec![TextMatch { start: 3, end: 9 }]);
    }

    #[test]
    fn utf16_offsets_stay_correct_across_several_matches() {
        let matches = find_matches("😀x😀x", "x", opts(false, true)).unwrap();
        assert_eq!(
            matches,
            vec![
                TextMatch { start: 2, end: 3 },
                TextMatch { start: 5, end: 6 }
            ]
        );
    }

    #[test]
    fn invalid_regex_is_reported_not_panicked() {
        let err = find_matches("text", "(unclosed", opts(true, true)).unwrap_err();
        assert!(matches!(err, SearchError::InvalidPattern(_)));
    }

    #[test]
    fn invalid_regex_is_literal_when_regex_mode_is_off() {
        assert_eq!(
            find_matches("a(unclosed", "(unclosed", opts(false, true)).unwrap(),
            vec![TextMatch { start: 1, end: 10 }]
        );
    }

    #[test]
    fn replacement_expands_captures_in_regex_mode() {
        let out =
            replacements("foo_old bar_old", r"(\w+)_old", "$1_new", opts(true, true)).unwrap();
        assert_eq!(
            out,
            vec![
                Replacement {
                    start: 0,
                    end: 7,
                    text: "foo_new".into()
                },
                Replacement {
                    start: 8,
                    end: 15,
                    text: "bar_new".into()
                }
            ]
        );
    }

    #[test]
    fn replacement_accepts_explicitly_braced_groups_too() {
        let out = replacements("foo_old", r"(\w+)_old", "${1}_new", opts(true, true)).unwrap();
        assert_eq!(out[0].text, "foo_new");
    }

    #[test]
    fn replacement_keeps_an_escaped_dollar() {
        let out = replacements("price", "price", "$$5", opts(true, true)).unwrap();
        assert_eq!(out[0].text, "$5");
    }

    #[test]
    fn replacement_is_verbatim_in_literal_mode() {
        let out = replacements("a b", "a", "$1", opts(false, true)).unwrap();
        assert_eq!(
            out,
            vec![Replacement {
                start: 0,
                end: 1,
                text: "$1".into()
            }]
        );
    }

    #[test]
    fn replacement_offsets_are_utf16() {
        let out = replacements("😀old", "old", "new", opts(false, true)).unwrap();
        assert_eq!(
            out,
            vec![Replacement {
                start: 2,
                end: 5,
                text: "new".into()
            }]
        );
    }
}
