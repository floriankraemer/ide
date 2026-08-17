//! Tree-sitter-backed syntax highlighting foundation.
//!
//! Qt-free by design (mirrors `editor-core`/`project-model`) — `ui-shell`
//! wraps [`highlight`] behind a `QSyntaxHighlighter` adapter later. This
//! crate only classifies bytes of already-loaded text into spans.

use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

/// Languages this crate can highlight. Two starter grammars per the
/// syntax-highlighting-foundation plan (Rust, JSON) plus a no-op fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Json,
    PlainText,
}

/// Map a file extension (no leading dot, e.g. `"rs"`, as returned by
/// `Path::extension()`) to the [`Language`] used to highlight it. Anything
/// unrecognized falls back to `PlainText`.
pub fn language_for_extension(extension: &str) -> Language {
    match extension {
        "rs" => Language::Rust,
        "json" => Language::Json,
        _ => Language::PlainText,
    }
}

/// Human-readable name for `language` (status bar, L3).
pub fn language_name(language: Language) -> &'static str {
    match language {
        Language::Rust => "Rust",
        Language::Json => "JSON",
        Language::PlainText => "Plain Text",
    }
}

/// Coarse token categories, kept to what tree-sitter's stock Rust/JSON
/// grammars naturally distinguish — not a general-purpose taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
    Other,
}

/// A classified byte range within the highlighted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

/// A `highlights.scm` query, compiled once per language and reused across
/// calls. Grammar + query text are bundled into the binary via
/// `include_str!`/`LANGUAGE` constants so highlighting works identically
/// under `cargo test` and in the packaged app — no runtime file loading.
struct QueryLanguage {
    grammar: tree_sitter::Language,
    query: Query,
}

fn rust_query_language() -> &'static QueryLanguage {
    static RUST: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(
            &grammar,
            include_str!("../queries/rust/highlights.scm"),
        )
        .expect("rust highlights.scm must compile against tree-sitter-rust's grammar");
        QueryLanguage { grammar, query }
    });
    &RUST
}

fn json_query_language() -> &'static QueryLanguage {
    static JSON: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_json::LANGUAGE.into();
        let query = Query::new(
            &grammar,
            include_str!("../queries/json/highlights.scm"),
        )
        .expect("json highlights.scm must compile against tree-sitter-json's grammar");
        QueryLanguage { grammar, query }
    });
    &JSON
}

fn query_language_for(language: Language) -> Option<&'static QueryLanguage> {
    match language {
        Language::Rust => Some(rust_query_language()),
        Language::Json => Some(json_query_language()),
        Language::PlainText => None,
    }
}

/// Map a query capture name (`@keyword`, `@string`, ...) onto the existing
/// `TokenKind` taxonomy. Captures with no mapping (there are none today,
/// since every capture in our `.scm` files is one of these six) are
/// dropped, mirroring the old hand-matcher's `_ => None` arm.
fn token_kind_for_capture(capture_name: &str) -> Option<TokenKind> {
    match capture_name {
        "keyword" => Some(TokenKind::Keyword),
        "string" => Some(TokenKind::String),
        "comment" => Some(TokenKind::Comment),
        "number" => Some(TokenKind::Number),
        "function" => Some(TokenKind::Function),
        "type" => Some(TokenKind::Type),
        _ => None,
    }
}

/// Highlight `text` as `language`, returning spans in document order.
///
/// Stateless one-shot convenience wrapper for tests/simple callers: builds
/// a throwaway [`Highlighter`], does one full parse, and discards it. For
/// repeated highlighting of an evolving document (the real editor use
/// case), construct a [`Highlighter`] once and call [`Highlighter::edit`]
/// per change instead — that keeps the persistent tree and reparses
/// incrementally rather than re-parsing the whole buffer every time.
pub fn highlight(language: Language, text: &str) -> Vec<HighlightSpan> {
    Highlighter::new(language).set_text(text)
}

fn spans_from_tree(
    ql: &QueryLanguage,
    tree: &tree_sitter::Tree,
    text: &str,
) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&ql.query, tree.root_node(), text.as_bytes());
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let capture_name = ql.query.capture_names()[capture.index as usize];
            if let Some(kind) = token_kind_for_capture(capture_name) {
                spans.push(HighlightSpan {
                    start: capture.node.start_byte(),
                    end: capture.node.end_byte(),
                    kind,
                });
            }
        }
    }
    spans.sort_by_key(|span| (span.start, span.end));
    spans
}

/// Byte offset `offset` within `text`, expressed as a tree-sitter
/// [`tree_sitter::Point`] (row, byte-column-within-row) — the coordinate
/// `InputEdit` needs alongside byte offsets. `offset` is clamped to
/// `text.len()`.
fn point_at(text: &str, offset: usize) -> tree_sitter::Point {
    let bytes = text.as_bytes();
    let offset = offset.min(bytes.len());
    let mut row = 0usize;
    let mut line_start = 0usize;
    for (i, &b) in bytes[..offset].iter().enumerate() {
        if b == b'\n' {
            row += 1;
            line_start = i + 1;
        }
    }
    tree_sitter::Point {
        row,
        column: offset - line_start,
    }
}

/// Stateful, incremental syntax highlighter: keeps a persistent
/// `tree_sitter::Tree` per instance (one per open document/tab, on the
/// caller's side) and reparses incrementally via `Tree::edit` +
/// `Parser::parse(text, Some(&old_tree))` instead of a fresh whole-buffer
/// parse on every change (upgrade over the v1 ceiling documented on
/// [`highlight`]'s predecessor — decision A6/A1).
///
/// [`Language::PlainText`] is a valid, cheap no-op: `set_text`/`edit` just
/// track the text and return no spans, so callers don't need to special
/// case unrecognized extensions.
pub struct Highlighter {
    language: Language,
    parser: Option<Parser>,
    tree: Option<tree_sitter::Tree>,
    text: String,
}

impl Highlighter {
    /// Create a highlighter for `language`. Cheap: the query/grammar are
    /// process-wide statics (see [`query_language_for`]), so this only
    /// allocates a `Parser` and an empty text buffer.
    pub fn new(language: Language) -> Self {
        let parser = query_language_for(language).and_then(|ql| {
            let mut parser = Parser::new();
            parser.set_language(&ql.grammar).ok()?;
            Some(parser)
        });
        Self {
            language,
            parser,
            tree: None,
            text: String::new(),
        }
    }

    /// Full (re)parse of `text`, discarding any previous incremental tree.
    /// Use for initial load; use [`Highlighter::edit`] for subsequent
    /// changes to get incremental reparsing.
    pub fn set_text(&mut self, text: &str) -> Vec<HighlightSpan> {
        self.tree = None;
        self.text = text.to_string();
        self.reparse()
    }

    /// Apply one contiguous byte-range replace and reparse incrementally.
    ///
    /// `new_text` is the *entire* new document text. `start_byte..
    /// old_end_byte` is the byte range being replaced in the *previous*
    /// text (as passed to the last `set_text`/`edit` call); `start_byte..
    /// new_end_byte` is the corresponding range in `new_text`. This is the
    /// standard tree-sitter `InputEdit` shape, expressed as byte offsets
    /// only — row/column `Point`s are derived internally from the old and
    /// new text so callers don't need to track them.
    pub fn edit(
        &mut self,
        new_text: &str,
        start_byte: usize,
        old_end_byte: usize,
        new_end_byte: usize,
    ) -> Vec<HighlightSpan> {
        if let Some(tree) = self.tree.as_mut() {
            let edit = tree_sitter::InputEdit {
                start_byte,
                old_end_byte,
                new_end_byte,
                start_position: point_at(&self.text, start_byte),
                old_end_position: point_at(&self.text, old_end_byte),
                new_end_position: point_at(new_text, new_end_byte),
            };
            tree.edit(&edit);
        }
        self.text = new_text.to_string();
        self.reparse()
    }

    fn reparse(&mut self) -> Vec<HighlightSpan> {
        let Some(ql) = query_language_for(self.language) else {
            return Vec::new();
        };
        let Some(parser) = self.parser.as_mut() else {
            return Vec::new();
        };
        let Some(new_tree) = parser.parse(&self.text, self.tree.as_ref()) else {
            return Vec::new();
        };
        let spans = spans_from_tree(ql, &new_tree, &self.text);
        self.tree = Some(new_tree);
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(spans: &[HighlightSpan], kind: TokenKind) -> Option<&HighlightSpan> {
        spans.iter().find(|s| s.kind == kind)
    }

    #[test]
    fn extension_maps_to_language() {
        assert_eq!(language_for_extension("rs"), Language::Rust);
        assert_eq!(language_for_extension("json"), Language::Json);
        assert_eq!(language_for_extension("txt"), Language::PlainText);
        assert_eq!(language_for_extension(""), Language::PlainText);
    }

    #[test]
    fn language_name_covers_every_language() {
        assert_eq!(language_name(Language::Rust), "Rust");
        assert_eq!(language_name(Language::Json), "JSON");
        assert_eq!(language_name(Language::PlainText), "Plain Text");
    }

    #[test]
    fn plain_text_yields_no_spans() {
        assert!(highlight(Language::PlainText, "fn foo() {}").is_empty());
    }

    #[test]
    fn rust_fn_keyword_is_highlighted() {
        let text = "fn foo() {}";
        let spans = highlight(Language::Rust, text);
        let span = find(&spans, TokenKind::Keyword).expect("expected a Keyword span");
        assert_eq!(&text[span.start..span.end], "fn");
    }

    #[test]
    fn rust_function_name_is_highlighted() {
        let text = "fn foo() {}";
        let spans = highlight(Language::Rust, text);
        let span = find(&spans, TokenKind::Function).expect("expected a Function span");
        assert_eq!(&text[span.start..span.end], "foo");
    }

    #[test]
    fn rust_string_literal_is_highlighted() {
        let text = "fn foo() { let s = \"hi\"; }";
        let spans = highlight(Language::Rust, text);
        let span = find(&spans, TokenKind::String).expect("expected a String span");
        assert_eq!(&text[span.start..span.end], "\"hi\"");
    }

    #[test]
    fn rust_comment_is_highlighted() {
        let text = "fn foo() { // hello\n}";
        let spans = highlight(Language::Rust, text);
        let span = find(&spans, TokenKind::Comment).expect("expected a Comment span");
        assert!(&text[span.start..span.end].starts_with("// hello"));
    }

    #[test]
    fn rust_number_is_highlighted() {
        let text = "fn foo() { let x = 42; }";
        let spans = highlight(Language::Rust, text);
        let span = find(&spans, TokenKind::Number).expect("expected a Number span");
        assert_eq!(&text[span.start..span.end], "42");
    }

    #[test]
    fn rust_type_is_highlighted() {
        let text = "fn foo() { let x: i32 = 42; }";
        let spans = highlight(Language::Rust, text);
        let span = find(&spans, TokenKind::Type).expect("expected a Type span");
        assert_eq!(&text[span.start..span.end], "i32");
    }

    #[test]
    fn json_string_key_is_highlighted() {
        let text = "{\"key\": \"value\"}";
        let spans = highlight(Language::Json, text);
        let span = find(&spans, TokenKind::String).expect("expected a String span");
        assert_eq!(&text[span.start..span.end], "\"key\"");
    }

    #[test]
    fn json_number_is_highlighted() {
        let text = "{\"n\": 42}";
        let spans = highlight(Language::Json, text);
        let span = find(&spans, TokenKind::Number).expect("expected a Number span");
        assert_eq!(&text[span.start..span.end], "42");
    }

    #[test]
    fn json_boolean_is_highlighted_as_keyword() {
        let text = "{\"b\": true}";
        let spans = highlight(Language::Json, text);
        let span = find(&spans, TokenKind::Keyword).expect("expected a Keyword span");
        assert_eq!(&text[span.start..span.end], "true");
    }

    #[test]
    fn spans_are_within_text_bounds() {
        let text = "fn foo() { let x: i32 = 42; \"s\"; }";
        for span in highlight(Language::Rust, text) {
            assert!(span.start <= span.end);
            assert!(span.end <= text.len());
        }
    }

    #[test]
    fn incremental_edit_matches_a_fresh_full_reparse() {
        // "let x = 42;" -> "let xy = 42;": insert "y" after "x" (single
        // char, byte offset 8..8 -> 8..9).
        let old_text = "fn foo() { let x = 42; }";
        let new_text = "fn foo() { let xy = 42; }";

        let mut incremental = Highlighter::new(Language::Rust);
        incremental.set_text(old_text);
        let incremental_spans = incremental.edit(new_text, 16, 16, 17);

        let fresh_spans = highlight(Language::Rust, new_text);

        assert_eq!(incremental_spans, fresh_spans_sorted(fresh_spans.clone()));
        // The number literal, well away from the edit, is still classified
        // correctly and at its new (shifted) position.
        let number = find(&incremental_spans, TokenKind::Number)
            .expect("expected a Number span after the edit");
        assert_eq!(&new_text[number.start..number.end], "42");
    }

    fn fresh_spans_sorted(mut spans: Vec<HighlightSpan>) -> Vec<HighlightSpan> {
        spans.sort_by_key(|span| (span.start, span.end));
        spans
    }

    #[test]
    fn editing_inside_a_string_literal_does_not_reclassify_surrounding_code() {
        // Insert a character inside the string literal "hi" -> "hxi".
        let old_text = "fn foo() { let s = \"hi\"; let n = 1; }";
        let new_text = "fn foo() { let s = \"hxi\"; let n = 1; }";

        let mut highlighter = Highlighter::new(Language::Rust);
        highlighter.set_text(old_text);
        // Byte 21 is right after the opening quote + "h": edit "i" -> "xi".
        let spans = highlighter.edit(new_text, 21, 21, 22);

        let keyword = find(&spans, TokenKind::Keyword).expect("expected a Keyword span");
        assert_eq!(&new_text[keyword.start..keyword.end], "fn");

        let function = find(&spans, TokenKind::Function).expect("expected a Function span");
        assert_eq!(&new_text[function.start..function.end], "foo");

        let string = find(&spans, TokenKind::String).expect("expected a String span");
        assert_eq!(&new_text[string.start..string.end], "\"hxi\"");

        let number = find(&spans, TokenKind::Number).expect("expected a Number span");
        assert_eq!(&new_text[number.start..number.end], "1");
    }

    #[test]
    fn highlighter_handles_plain_text_as_a_no_op() {
        let mut highlighter = Highlighter::new(Language::PlainText);
        assert!(highlighter.set_text("hello").is_empty());
        assert!(highlighter.edit("hello world", 5, 5, 11).is_empty());
    }
}
