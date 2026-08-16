//! Tree-sitter-backed syntax highlighting foundation.
//!
//! Qt-free by design (mirrors `editor-core`/`project-model`) — `ui-shell`
//! wraps [`highlight`] behind a `QSyntaxHighlighter` adapter later. This
//! crate only classifies bytes of already-loaded text into spans.

use tree_sitter::{Node, Parser};

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

/// Highlight `text` as `language`, returning spans in document order.
///
/// v1 ceiling: whole-buffer parse on every call, no incremental
/// `InputEdit` reparse — see decision A6 for the upgrade path.
pub fn highlight(language: Language, text: &str) -> Vec<HighlightSpan> {
    let grammar = match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Json => tree_sitter_json::LANGUAGE.into(),
        Language::PlainText => return Vec::new(),
    };

    let mut parser = Parser::new();
    if parser.set_language(&grammar).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };

    let mut spans = Vec::new();
    collect_spans(language, tree.root_node(), &mut spans);
    spans
}

fn collect_spans(language: Language, node: Node, spans: &mut Vec<HighlightSpan>) {
    if let Some(kind) = classify(language, node) {
        spans.push(HighlightSpan {
            start: node.start_byte(),
            end: node.end_byte(),
            kind,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_spans(language, child, spans);
    }
}

fn classify(language: Language, node: Node) -> Option<TokenKind> {
    match language {
        Language::Rust => classify_rust(node),
        Language::Json => classify_json(node),
        Language::PlainText => None,
    }
}

/// Node-kind classification for the `tree-sitter-rust` grammar. Driven by
/// inspecting the grammar's own node kinds rather than a bundled
/// `highlights.scm` query, since this is a foundation-level v1 (see A6).
fn classify_rust(node: Node) -> Option<TokenKind> {
    match node.kind() {
        "line_comment" | "block_comment" => Some(TokenKind::Comment),
        "string_literal" | "raw_string_literal" | "char_literal" => Some(TokenKind::String),
        "integer_literal" | "float_literal" => Some(TokenKind::Number),
        "type_identifier" | "primitive_type" => Some(TokenKind::Type),
        "identifier" => match node.parent()?.kind() {
            "function_item" | "call_expression" => Some(TokenKind::Function),
            _ => None,
        },
        "fn" | "let" | "mut" | "pub" | "struct" | "enum" | "impl" | "trait" | "use" | "mod"
        | "return" | "if" | "else" | "match" | "for" | "while" | "loop" | "break"
        | "continue" | "const" | "static" | "async" | "await" | "move" | "ref" | "as"
        | "where" | "unsafe" | "dyn" | "extern" | "in" | "self" | "super" | "crate" | "true"
        | "false" => Some(TokenKind::Keyword),
        _ => None,
    }
}

/// Node-kind classification for the `tree-sitter-json` grammar. Standard
/// JSON has no comments, so `TokenKind::Comment` is never produced here.
fn classify_json(node: Node) -> Option<TokenKind> {
    match node.kind() {
        "string" => Some(TokenKind::String),
        "number" => Some(TokenKind::Number),
        "true" | "false" | "null" => Some(TokenKind::Keyword),
        _ => None,
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
}
