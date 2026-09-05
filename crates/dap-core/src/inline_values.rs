//! Which of a stopped frame's variables belong on which line of the source
//! (D3-7).
//!
//! The Variables view answers "what is in scope"; inline values answer a
//! narrower question — "what is the value of the thing I am looking at" —
//! and the difference is entirely about the *source text*, which is why
//! this is a rule rather than a paint routine. DAP itself says nothing
//! about it: an adapter may implement the optional `inlineValues` request,
//! but none of the three this IDE ships does, so the association is made
//! here from the two things that are known — the frame's variables and the
//! text of the file it stopped in.
//!
//! The rule, deliberately simple and stated rather than tuned: a variable
//! is shown at the end of the **last line at or above the stopped line**
//! that mentions its name as a whole word. That is where the reader last
//! saw it, which is where they look for it.

/// One line's worth of inline values, ready to paint at the end of it.
/// `line` is 1-based, like every line number that crosses this crate's
/// seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineValue {
    pub line: u32,
    pub text: String,
}

/// A value longer than this is elided. An inline value competes with the
/// code it sits beside; a whole serialised struct on one line hides the
/// program the user is reading.
const MAX_VALUE_LEN: usize = 60;

/// Values for `source`, given the current frame's variables in the order
/// the adapter reported them and the 1-based line it stopped on.
///
/// Names that are not identifiers are skipped: an adapter also reports
/// element indices (`[0]`), synthetic groups ("special variables") and
/// expressions, and none of those is a word to find in the text.
pub fn inline_values(
    source: &str,
    variables: &[(String, String)],
    stopped_line: u32,
) -> Vec<InlineValue> {
    let lines: Vec<&str> = source.lines().collect();
    let last = (stopped_line as usize).min(lines.len());

    let mut per_line: Vec<(u32, Vec<String>)> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();

    for (name, value) in variables {
        if !is_identifier(name) || seen.contains(&name.as_str()) {
            continue;
        }
        seen.push(name.as_str());

        let Some(index) = (0..last).rev().find(|&i| mentions_word(lines[i], name)) else {
            continue;
        };
        let line = index as u32 + 1;
        let label = format!("{name} = {}", elide(value));
        match per_line.iter_mut().find(|(at, _)| *at == line) {
            Some((_, labels)) => labels.push(label),
            None => per_line.push((line, vec![label])),
        }
    }

    per_line.sort_by_key(|(line, _)| *line);
    per_line
        .into_iter()
        .map(|(line, labels)| InlineValue {
            line,
            text: labels.join(", "),
        })
        .collect()
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Whether `line` mentions `word` with no identifier character either side
/// — so `count` does not match inside `counter` or `line_count`.
fn mentions_word(line: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(found) = line[from..].find(word) {
        let start = from + found;
        let end = start + word.len();
        let before_ok = start == 0 || !is_word_byte(line.as_bytes()[start - 1]);
        let after_ok = end == line.len() || !is_word_byte(line.as_bytes()[end]);
        if before_ok && after_ok {
            return true;
        }
        from = start + word.len().max(1);
        if from >= line.len() {
            break;
        }
    }
    false
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn elide(value: &str) -> String {
    let flat = value.replace(['\n', '\r'], " ");
    if flat.chars().count() <= MAX_VALUE_LEN {
        return flat;
    }
    let cut: String = flat.chars().take(MAX_VALUE_LEN).collect();
    format!("{cut}\u{2026}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    const SOURCE: &str =
        "fn main() {\n    let total = 1;\n    let counter = 2;\n    println!(\"{total}\");\n}\n";

    #[test]
    fn a_variable_lands_on_the_last_line_that_mentions_it() {
        let values = inline_values(SOURCE, &vars(&[("total", "1")]), 5);
        assert_eq!(
            values,
            vec![InlineValue {
                line: 4,
                text: "total = 1".to_string()
            }]
        );
    }

    #[test]
    fn nothing_below_the_stopped_line_is_considered() {
        // Line 4 mentions `total`, but execution has not reached it — a
        // value shown there would be a claim about the future.
        let values = inline_values(SOURCE, &vars(&[("total", "1")]), 3);
        assert_eq!(values[0].line, 2);
    }

    #[test]
    fn a_name_inside_a_longer_identifier_is_not_a_mention() {
        // `counter` contains `count`; only whole words count.
        let values = inline_values(SOURCE, &vars(&[("count", "9")]), 5);
        assert!(values.is_empty());
    }

    #[test]
    fn two_variables_on_one_line_share_it() {
        let source = "let a = 1; let b = 2;\n";
        let values = inline_values(source, &vars(&[("a", "1"), ("b", "2")]), 1);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].text, "a = 1, b = 2");
    }

    #[test]
    fn names_that_are_not_identifiers_are_skipped() {
        // What an adapter reports for a list's elements and its synthetic
        // groups: neither is a word to look for in the source.
        let source = "let items = vec![1];\n";
        let values = inline_values(
            source,
            &vars(&[("[0]", "1"), ("special variables", "{...}")]),
            1,
        );
        assert!(values.is_empty());
    }

    #[test]
    fn a_long_value_is_elided_rather_than_pushing_the_code_off_screen() {
        let source = "let big = load();\n";
        let long = "x".repeat(200);
        let values = inline_values(source, &vars(&[("big", &long)]), 1);
        assert!(values[0].text.ends_with('\u{2026}'));
        assert!(values[0].text.chars().count() < 80);
    }

    #[test]
    fn a_multi_line_value_is_flattened_onto_one_line() {
        let source = "let point = make();\n";
        let values = inline_values(source, &vars(&[("point", "Point {\n  x: 1\n}")]), 1);
        assert!(!values[0].text.contains('\n'));
    }

    #[test]
    fn a_variable_the_file_never_mentions_is_not_shown() {
        let values = inline_values(SOURCE, &vars(&[("elsewhere", "1")]), 5);
        assert!(values.is_empty());
    }
}
