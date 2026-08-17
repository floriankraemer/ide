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
/// Stateless one-shot convenience wrapper: parses fresh and discards the
/// tree. For repeated highlighting of an evolving document, prefer
/// [`Highlighter`], which keeps a persistent tree and reparses
/// incrementally.
pub fn highlight(language: Language, text: &str) -> Vec<HighlightSpan> {
    let Some(ql) = query_language_for(language) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&ql.grammar).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };
    spans_from_tree(ql, &tree, text)
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
}
