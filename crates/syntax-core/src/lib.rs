//! Tree-sitter-backed syntax highlighting foundation.
//!
//! Qt-free by design (mirrors `editor-core`/`project-model`) — `ui-shell`
//! wraps [`highlight`] behind a `QSyntaxHighlighter` adapter later. This
//! crate only classifies bytes of already-loaded text into spans.

use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

/// Languages this crate can highlight. Rust/JSON from the original
/// syntax-highlighting-foundation plan, plus C#/Java/PHP (Task B), plus a
/// no-op fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Json,
    CSharp,
    Java,
    Php,
    PlainText,
}

/// Map a file extension (no leading dot, e.g. `"rs"`, as returned by
/// `Path::extension()`) to the [`Language`] used to highlight it. Anything
/// unrecognized falls back to `PlainText`.
pub fn language_for_extension(extension: &str) -> Language {
    match extension {
        "rs" => Language::Rust,
        "json" => Language::Json,
        "cs" => Language::CSharp,
        "java" => Language::Java,
        "php" => Language::Php,
        _ => Language::PlainText,
    }
}

/// Human-readable name for `language` (status bar, L3).
pub fn language_name(language: Language) -> &'static str {
    match language {
        Language::Rust => "Rust",
        Language::Json => "JSON",
        Language::CSharp => "C#",
        Language::Java => "Java",
        Language::Php => "PHP",
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

/// One occurrence of an identifier-like node (byte range `start..end` into
/// the source text `name` was read from), from `locals.scm`'s
/// `@definition`/`@reference` captures (A2). `is_definition` is true when
/// this occurrence is also a declaration site (function/struct/parameter/
/// `let`-binding name, ...) per the language's `locals.scm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub is_definition: bool,
}

/// A foldable region (Task C): byte range `start..end` of a block/body-like
/// node (function/method body, class/struct/enum body, object/array, ...)
/// from the language's `folds.scm` `@fold` capture. Emitted in document
/// order by [`Highlighter::fold_ranges`], which reads off the same
/// incrementally-maintained tree `set_text`/`edit` already keep current —
/// no second full-buffer parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldRange {
    pub start: usize,
    pub end: usize,
}

/// Structural kind of a symbol extracted by [`outline()`] (Task D). Not
/// every language uses every variant — e.g. Rust has no `Interface` in the
/// literal sense (its `trait`s map onto it), PHP's `trait`s also map onto
/// it (see `php/tags.scm`), and JSON uses none of them. Kept to what's
/// actually meaningful across Rust/C#/Java/PHP rather than a maximal
/// per-language taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Class,
    Struct,
    Enum,
    Interface,
    Method,
    Function,
    Field,
}

/// One definition site extracted by [`outline()`] (Task D): a name plus
/// its structural kind, the byte range of the whole definition (`start`/
/// `end` — used to jump-select or fold the definition) and of just its
/// name token (`name_start`/`name_end` — used to place the cursor exactly
/// on the identifier). `children` holds definitions nested inside this
/// one by AST byte-range containment (e.g. a class's methods/fields, an
/// `impl` block's methods) — see [`outline()`]'s doc comment for why
/// containment, not an explicit parent capture, decides nesting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolNode {
    pub name: String,
    pub kind: SymbolKind,
    pub start: usize,
    pub end: usize,
    pub name_start: usize,
    pub name_end: usize,
    pub children: Vec<SymbolNode>,
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

fn csharp_query_language() -> &'static QueryLanguage {
    static CSHARP: LazyLock<QueryLanguage> = LazyLock::new(|| {
        // Pinned to 0.21.3, not `LANGUAGE`/`.into()` like the other
        // grammars: see the version-pin comment on the `tree-sitter-c-sharp`
        // dependency in Cargo.toml. This older release's binding exposes a
        // `language()` fn returning `tree_sitter::Language` directly.
        let grammar: tree_sitter::Language = tree_sitter_c_sharp::language();
        let query = Query::new(
            &grammar,
            include_str!("../queries/csharp/highlights.scm"),
        )
        .expect("csharp highlights.scm must compile against tree-sitter-c-sharp's grammar");
        QueryLanguage { grammar, query }
    });
    &CSHARP
}

fn java_query_language() -> &'static QueryLanguage {
    static JAVA: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(
            &grammar,
            include_str!("../queries/java/highlights.scm"),
        )
        .expect("java highlights.scm must compile against tree-sitter-java's grammar");
        QueryLanguage { grammar, query }
    });
    &JAVA
}

fn php_query_language() -> &'static QueryLanguage {
    static PHP: LazyLock<QueryLanguage> = LazyLock::new(|| {
        // `php_only` (body-only grammar), not `LANGUAGE_PHP` (embedded
        // HTML) — v1 design decision, see the plan doc.
        let grammar: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP_ONLY.into();
        let query = Query::new(
            &grammar,
            include_str!("../queries/php/highlights.scm"),
        )
        .expect("php highlights.scm must compile against tree-sitter-php's php_only grammar");
        QueryLanguage { grammar, query }
    });
    &PHP
}

fn query_language_for(language: Language) -> Option<&'static QueryLanguage> {
    match language {
        Language::Rust => Some(rust_query_language()),
        Language::Json => Some(json_query_language()),
        Language::CSharp => Some(csharp_query_language()),
        Language::Java => Some(java_query_language()),
        Language::Php => Some(php_query_language()),
        Language::PlainText => None,
    }
}

fn rust_locals_query_language() -> &'static QueryLanguage {
    static RUST_LOCALS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(&grammar, include_str!("../queries/rust/locals.scm"))
            .expect("rust locals.scm must compile against tree-sitter-rust's grammar");
        QueryLanguage { grammar, query }
    });
    &RUST_LOCALS
}

fn json_locals_query_language() -> &'static QueryLanguage {
    static JSON_LOCALS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_json::LANGUAGE.into();
        let query = Query::new(&grammar, include_str!("../queries/json/locals.scm"))
            .expect("json locals.scm must compile against tree-sitter-json's grammar");
        QueryLanguage { grammar, query }
    });
    &JSON_LOCALS
}

fn csharp_locals_query_language() -> &'static QueryLanguage {
    static CSHARP_LOCALS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_c_sharp::language();
        let query = Query::new(&grammar, include_str!("../queries/csharp/locals.scm"))
            .expect("csharp locals.scm must compile against tree-sitter-c-sharp's grammar");
        QueryLanguage { grammar, query }
    });
    &CSHARP_LOCALS
}

fn java_locals_query_language() -> &'static QueryLanguage {
    static JAVA_LOCALS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(&grammar, include_str!("../queries/java/locals.scm"))
            .expect("java locals.scm must compile against tree-sitter-java's grammar");
        QueryLanguage { grammar, query }
    });
    &JAVA_LOCALS
}

fn php_locals_query_language() -> &'static QueryLanguage {
    static PHP_LOCALS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP_ONLY.into();
        let query = Query::new(&grammar, include_str!("../queries/php/locals.scm"))
            .expect("php locals.scm must compile against tree-sitter-php's php_only grammar");
        QueryLanguage { grammar, query }
    });
    &PHP_LOCALS
}

fn locals_query_language_for(language: Language) -> Option<&'static QueryLanguage> {
    match language {
        Language::Rust => Some(rust_locals_query_language()),
        Language::Json => Some(json_locals_query_language()),
        Language::CSharp => Some(csharp_locals_query_language()),
        Language::Java => Some(java_locals_query_language()),
        Language::Php => Some(php_locals_query_language()),
        Language::PlainText => None,
    }
}

fn rust_folds_query_language() -> &'static QueryLanguage {
    static RUST_FOLDS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(&grammar, include_str!("../queries/rust/folds.scm"))
            .expect("rust folds.scm must compile against tree-sitter-rust's grammar");
        QueryLanguage { grammar, query }
    });
    &RUST_FOLDS
}

fn json_folds_query_language() -> &'static QueryLanguage {
    static JSON_FOLDS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_json::LANGUAGE.into();
        let query = Query::new(&grammar, include_str!("../queries/json/folds.scm"))
            .expect("json folds.scm must compile against tree-sitter-json's grammar");
        QueryLanguage { grammar, query }
    });
    &JSON_FOLDS
}

fn csharp_folds_query_language() -> &'static QueryLanguage {
    static CSHARP_FOLDS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_c_sharp::language();
        let query = Query::new(&grammar, include_str!("../queries/csharp/folds.scm"))
            .expect("csharp folds.scm must compile against tree-sitter-c-sharp's grammar");
        QueryLanguage { grammar, query }
    });
    &CSHARP_FOLDS
}

fn java_folds_query_language() -> &'static QueryLanguage {
    static JAVA_FOLDS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(&grammar, include_str!("../queries/java/folds.scm"))
            .expect("java folds.scm must compile against tree-sitter-java's grammar");
        QueryLanguage { grammar, query }
    });
    &JAVA_FOLDS
}

fn php_folds_query_language() -> &'static QueryLanguage {
    static PHP_FOLDS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP_ONLY.into();
        let query = Query::new(&grammar, include_str!("../queries/php/folds.scm"))
            .expect("php folds.scm must compile against tree-sitter-php's php_only grammar");
        QueryLanguage { grammar, query }
    });
    &PHP_FOLDS
}

fn folds_query_language_for(language: Language) -> Option<&'static QueryLanguage> {
    match language {
        Language::Rust => Some(rust_folds_query_language()),
        Language::Json => Some(json_folds_query_language()),
        Language::CSharp => Some(csharp_folds_query_language()),
        Language::Java => Some(java_folds_query_language()),
        Language::Php => Some(php_folds_query_language()),
        Language::PlainText => None,
    }
}

fn rust_tags_query_language() -> &'static QueryLanguage {
    static RUST_TAGS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(&grammar, include_str!("../queries/rust/tags.scm"))
            .expect("rust tags.scm must compile against tree-sitter-rust's grammar");
        QueryLanguage { grammar, query }
    });
    &RUST_TAGS
}

fn json_tags_query_language() -> &'static QueryLanguage {
    static JSON_TAGS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_json::LANGUAGE.into();
        let query = Query::new(&grammar, include_str!("../queries/json/tags.scm"))
            .expect("json tags.scm must compile against tree-sitter-json's grammar");
        QueryLanguage { grammar, query }
    });
    &JSON_TAGS
}

fn csharp_tags_query_language() -> &'static QueryLanguage {
    static CSHARP_TAGS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_c_sharp::language();
        let query = Query::new(&grammar, include_str!("../queries/csharp/tags.scm"))
            .expect("csharp tags.scm must compile against tree-sitter-c-sharp's grammar");
        QueryLanguage { grammar, query }
    });
    &CSHARP_TAGS
}

fn java_tags_query_language() -> &'static QueryLanguage {
    static JAVA_TAGS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(&grammar, include_str!("../queries/java/tags.scm"))
            .expect("java tags.scm must compile against tree-sitter-java's grammar");
        QueryLanguage { grammar, query }
    });
    &JAVA_TAGS
}

fn php_tags_query_language() -> &'static QueryLanguage {
    static PHP_TAGS: LazyLock<QueryLanguage> = LazyLock::new(|| {
        let grammar: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP_ONLY.into();
        let query = Query::new(&grammar, include_str!("../queries/php/tags.scm"))
            .expect("php tags.scm must compile against tree-sitter-php's php_only grammar");
        QueryLanguage { grammar, query }
    });
    &PHP_TAGS
}

fn tags_query_language_for(language: Language) -> Option<&'static QueryLanguage> {
    match language {
        Language::Rust => Some(rust_tags_query_language()),
        Language::Json => Some(json_tags_query_language()),
        Language::CSharp => Some(csharp_tags_query_language()),
        Language::Java => Some(java_tags_query_language()),
        Language::Php => Some(php_tags_query_language()),
        Language::PlainText => None,
    }
}

/// Map a `tags.scm` `@definition.<kind>` capture name onto [`SymbolKind`]
/// — the part after the dot is the kind name verbatim (lowercased).
fn symbol_kind_for_capture(capture_name: &str) -> Option<SymbolKind> {
    match capture_name.strip_prefix("definition.")? {
        "class" => Some(SymbolKind::Class),
        "struct" => Some(SymbolKind::Struct),
        "enum" => Some(SymbolKind::Enum),
        "interface" => Some(SymbolKind::Interface),
        "method" => Some(SymbolKind::Method),
        "function" => Some(SymbolKind::Function),
        "field" => Some(SymbolKind::Field),
        _ => None,
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

/// Every identifier-like node in `text`, parsed as `language`, in document
/// order — not just declaration sites (A2). Stateless one-shot, matching
/// [`highlight`]'s convention: does its own parse rather than reusing a
/// [`Highlighter`]'s persistent tree, since nothing needs this
/// incrementally yet.
///
/// Backed by a `locals.scm` per language (see `crates/syntax-core/queries/
/// */locals.scm`) with `@definition`/`@reference` captures. A node can
/// legitimately match both (e.g. a function name is a definition site and
/// also matches the catch-all reference pattern — see the comment atop
/// `rust/locals.scm`), so captures are folded by node byte-range with OR:
/// each identifier node appears exactly once in the result, with
/// `is_definition` true if any capture on it was `@definition`.
/// [`Language::PlainText`] (or a language with no `locals.scm`) yields an
/// empty vec.
pub fn identifier_occurrences(language: Language, text: &str) -> Vec<Occurrence> {
    let Some(ql) = locals_query_language_for(language) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&ql.grammar).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };

    let mut by_range: std::collections::BTreeMap<(usize, usize), bool> =
        std::collections::BTreeMap::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&ql.query, tree.root_node(), text.as_bytes());
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let capture_name = ql.query.capture_names()[capture.index as usize];
            let is_definition = match capture_name {
                "definition" => true,
                "reference" => false,
                _ => continue,
            };
            let range = (capture.node.start_byte(), capture.node.end_byte());
            let entry = by_range.entry(range).or_insert(false);
            *entry |= is_definition;
        }
    }

    by_range
        .into_iter()
        .map(|((start, end), is_definition)| Occurrence {
            name: text[start..end].to_string(),
            start,
            end,
            is_definition,
        })
        .collect()
}

/// One raw `tags.scm` match before nesting: a definition's kind and byte
/// range plus its name token's byte range, not yet organized into a tree.
struct RawSymbol {
    kind: SymbolKind,
    start: usize,
    end: usize,
    name_start: usize,
    name_end: usize,
}

/// Per-file symbol outline of `text`, parsed as `language` (Task D) — the
/// data source for Class View's per-file tier. Stateless one-shot, matching
/// [`highlight`]/[`identifier_occurrences`]'s convention: does its own
/// parse rather than reusing a [`Highlighter`]'s persistent tree. `outline`
/// is refreshed on save (a project-wide-scope panel doesn't need live
/// per-keystroke updates), so there's no incremental-tree benefit to chase
/// here the way there is for on-every-keystroke highlighting.
///
/// Backed by a `tags.scm` per language (see `crates/syntax-core/queries/
/// */tags.scm`), following the community `tree-sitter-tags` convention:
/// each pattern captures the whole definition node as `@definition.<kind>`
/// and its identifier as `@name`. Nesting (methods/fields under their
/// class, methods under an `impl` block, ...) is derived here from AST
/// byte-range containment rather than from an explicit parent capture —
/// simpler to get right than threading parent pointers through every
/// query, and correct by construction since a tree-sitter node's range
/// always fully contains its descendants' ranges. [`Language::PlainText`]
/// (or a language with an empty `tags.scm`, i.e. JSON) yields an empty vec.
pub fn outline(language: Language, text: &str) -> Vec<SymbolNode> {
    let Some(ql) = tags_query_language_for(language) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&ql.grammar).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };

    let mut raw: Vec<RawSymbol> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&ql.query, tree.root_node(), text.as_bytes());
    while let Some(m) = matches.next() {
        let mut definition: Option<(SymbolKind, usize, usize)> = None;
        let mut name: Option<(usize, usize)> = None;
        for capture in m.captures {
            let capture_name = ql.query.capture_names()[capture.index as usize];
            if capture_name == "name" {
                name = Some((capture.node.start_byte(), capture.node.end_byte()));
            } else if let Some(kind) = symbol_kind_for_capture(capture_name) {
                definition = Some((kind, capture.node.start_byte(), capture.node.end_byte()));
            }
        }
        if let (Some((kind, start, end)), Some((name_start, name_end))) = (definition, name) {
            raw.push(RawSymbol {
                kind,
                start,
                end,
                name_start,
                name_end,
            });
        }
    }

    build_symbol_tree(raw, text)
}

/// Nests `raw` definitions by AST byte-range containment: a classic
/// "build a tree from ranges" stack scan. `raw` is sorted by start byte
/// (ties broken by longest-range-first, so an outer definition is always
/// pushed before an inner one that starts at the same byte — e.g. a
/// struct with no leading trivia and its first field). While the next
/// definition still starts before the stack top's end, it nests inside;
/// once a definition starts at or past the top's end, the top is closed
/// out (attached to its own parent, or promoted to a root) and popped.
/// This is correct by construction — not a heuristic — because tree-sitter
/// node ranges never partially overlap, only nest or sit disjoint.
fn build_symbol_tree(mut raw: Vec<RawSymbol>, text: &str) -> Vec<SymbolNode> {
    raw.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));

    let mut roots: Vec<SymbolNode> = Vec::new();
    let mut open: Vec<SymbolNode> = Vec::new();

    fn attach(open: &mut Vec<SymbolNode>, roots: &mut Vec<SymbolNode>, node: SymbolNode) {
        match open.last_mut() {
            Some(parent) => parent.children.push(node),
            None => roots.push(node),
        }
    }

    for r in raw {
        while let Some(top) = open.last() {
            if top.end <= r.start {
                let done = open.pop().expect("just peeked Some");
                attach(&mut open, &mut roots, done);
            } else {
                break;
            }
        }
        open.push(SymbolNode {
            name: text[r.name_start..r.name_end].to_string(),
            kind: r.kind,
            start: r.start,
            end: r.end,
            name_start: r.name_start,
            name_end: r.name_end,
            children: Vec::new(),
        });
    }
    while let Some(done) = open.pop() {
        attach(&mut open, &mut roots, done);
    }

    roots
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

    /// Foldable regions (Task C) in document order, from the current
    /// incremental tree — i.e. whatever `set_text`/`edit` last left behind.
    /// Does not reparse: call after `set_text`/`edit`, not instead of it.
    /// Empty for [`Language::PlainText`], a language with no `folds.scm`,
    /// or before the first `set_text`/`edit` call.
    pub fn fold_ranges(&self) -> Vec<FoldRange> {
        let (Some(ql), Some(tree)) = (folds_query_language_for(self.language), self.tree.as_ref())
        else {
            return Vec::new();
        };
        let mut ranges = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&ql.query, tree.root_node(), self.text.as_bytes());
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let capture_name = ql.query.capture_names()[capture.index as usize];
                if capture_name == "fold" {
                    ranges.push(FoldRange {
                        start: capture.node.start_byte(),
                        end: capture.node.end_byte(),
                    });
                }
            }
        }
        ranges.sort_by_key(|r| (r.start, r.end));
        ranges.dedup();
        ranges
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
        assert_eq!(language_for_extension("cs"), Language::CSharp);
        assert_eq!(language_for_extension("java"), Language::Java);
        assert_eq!(language_for_extension("php"), Language::Php);
        assert_eq!(language_for_extension("txt"), Language::PlainText);
        assert_eq!(language_for_extension(""), Language::PlainText);
    }

    #[test]
    fn language_name_covers_every_language() {
        assert_eq!(language_name(Language::Rust), "Rust");
        assert_eq!(language_name(Language::Json), "JSON");
        assert_eq!(language_name(Language::CSharp), "C#");
        assert_eq!(language_name(Language::Java), "Java");
        assert_eq!(language_name(Language::Php), "PHP");
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

    #[test]
    fn plain_text_has_no_identifier_occurrences() {
        assert!(identifier_occurrences(Language::PlainText, "fn foo() {}").is_empty());
    }

    #[test]
    fn rust_function_and_parameter_are_definitions_used_twice_in_body() {
        let text = "fn add(x: i32) -> i32 { x + x }";
        let occurrences = identifier_occurrences(Language::Rust, text);

        let by_name = |name: &str| -> Vec<&Occurrence> {
            occurrences.iter().filter(|o| o.name == name).collect()
        };

        let foo = by_name("add");
        assert_eq!(foo.len(), 1, "function name should occur once: {foo:?}");
        assert!(foo[0].is_definition);
        assert_eq!(&text[foo[0].start..foo[0].end], "add");

        let xs = by_name("x");
        assert_eq!(xs.len(), 3, "1 definition + 2 references: {xs:?}");
        let definitions: Vec<_> = xs.iter().filter(|o| o.is_definition).collect();
        let references: Vec<_> = xs.iter().filter(|o| !o.is_definition).collect();
        assert_eq!(definitions.len(), 1, "exactly one `x` is the parameter");
        assert_eq!(references.len(), 2, "both body uses of `x` are references");

        // Occurrences are in document order and byte ranges point at the
        // right substrings.
        for occurrence in &occurrences {
            assert_eq!(
                &text[occurrence.start..occurrence.end],
                occurrence.name,
                "byte range must point at the occurrence's own text"
            );
        }
        let mut starts: Vec<usize> = occurrences.iter().map(|o| o.start).collect();
        let mut sorted_starts = starts.clone();
        sorted_starts.sort_unstable();
        assert_eq!(starts, sorted_starts, "occurrences must be in document order");
        starts.dedup();
    }

    #[test]
    fn rust_struct_name_is_a_definition() {
        let text = "struct Point { x: i32 }";
        let occurrences = identifier_occurrences(Language::Rust, text);
        let point = occurrences
            .iter()
            .find(|o| o.name == "Point")
            .expect("expected an occurrence for the struct name");
        assert!(point.is_definition);
        assert_eq!(&text[point.start..point.end], "Point");
    }

    // --- C# (Task B) ---

    const CSHARP_SNIPPET: &str = "class Greeter {\n    public string Name;\n\n    public Greeter(string name) {\n        Name = name;\n    }\n\n    public string Greet() {\n        // say hi\n        return \"Hello, \" + Name;\n    }\n}\n";

    #[test]
    fn csharp_class_keyword_is_highlighted() {
        let spans = highlight(Language::CSharp, CSHARP_SNIPPET);
        let span = find(&spans, TokenKind::Keyword).expect("expected a Keyword span");
        assert_eq!(&CSHARP_SNIPPET[span.start..span.end], "class");
    }

    #[test]
    fn csharp_string_literal_is_highlighted() {
        let spans = highlight(Language::CSharp, CSHARP_SNIPPET);
        let span = find(&spans, TokenKind::String).expect("expected a String span");
        assert_eq!(&CSHARP_SNIPPET[span.start..span.end], "\"Hello, \"");
    }

    #[test]
    fn csharp_comment_is_highlighted() {
        let spans = highlight(Language::CSharp, CSHARP_SNIPPET);
        let span = find(&spans, TokenKind::Comment).expect("expected a Comment span");
        assert!(&CSHARP_SNIPPET[span.start..span.end].starts_with("// say hi"));
    }

    #[test]
    fn csharp_class_name_is_highlighted_as_type() {
        let spans = highlight(Language::CSharp, CSHARP_SNIPPET);
        let type_span = spans
            .iter()
            .find(|s| s.kind == TokenKind::Type && &CSHARP_SNIPPET[s.start..s.end] == "Greeter");
        assert!(type_span.is_some(), "expected `Greeter` highlighted as a Type");
    }

    #[test]
    fn csharp_method_definition_is_recognized() {
        let occurrences = identifier_occurrences(Language::CSharp, CSHARP_SNIPPET);
        let greet = occurrences
            .iter()
            .find(|o| o.name == "Greet")
            .expect("expected an occurrence for the method name");
        assert!(greet.is_definition);
    }

    #[test]
    fn csharp_parameter_used_twice_is_one_definition_and_one_reference() {
        let text = "class C { void M(string name) { name = name; } }";
        let occurrences = identifier_occurrences(Language::CSharp, text);
        let names: Vec<&Occurrence> = occurrences.iter().filter(|o| o.name == "name").collect();
        assert_eq!(names.len(), 3, "1 definition + 2 references: {names:?}");
        assert_eq!(names.iter().filter(|o| o.is_definition).count(), 1);
        assert_eq!(names.iter().filter(|o| !o.is_definition).count(), 2);
    }

    // --- Java (Task B) ---

    const JAVA_SNIPPET: &str = "public class Greeter {\n    private String name;\n\n    public Greeter(String name) {\n        this.name = name;\n    }\n\n    public String greet() {\n        // say hi\n        return \"Hello, \" + name;\n    }\n}\n";

    #[test]
    fn java_class_keyword_is_highlighted() {
        let spans = highlight(Language::Java, JAVA_SNIPPET);
        let span = find(&spans, TokenKind::Keyword).expect("expected a Keyword span");
        assert_eq!(&JAVA_SNIPPET[span.start..span.end], "public");
    }

    #[test]
    fn java_string_literal_is_highlighted() {
        let spans = highlight(Language::Java, JAVA_SNIPPET);
        let span = find(&spans, TokenKind::String).expect("expected a String span");
        assert_eq!(&JAVA_SNIPPET[span.start..span.end], "\"Hello, \"");
    }

    #[test]
    fn java_comment_is_highlighted() {
        let spans = highlight(Language::Java, JAVA_SNIPPET);
        let span = find(&spans, TokenKind::Comment).expect("expected a Comment span");
        assert!(&JAVA_SNIPPET[span.start..span.end].starts_with("// say hi"));
    }

    #[test]
    fn java_class_name_is_highlighted_as_type() {
        let spans = highlight(Language::Java, JAVA_SNIPPET);
        let type_span = spans
            .iter()
            .find(|s| s.kind == TokenKind::Type && &JAVA_SNIPPET[s.start..s.end] == "Greeter");
        assert!(type_span.is_some(), "expected `Greeter` highlighted as a Type");
    }

    #[test]
    fn java_method_definition_is_recognized() {
        let occurrences = identifier_occurrences(Language::Java, JAVA_SNIPPET);
        let greet = occurrences
            .iter()
            .find(|o| o.name == "greet")
            .expect("expected an occurrence for the method name");
        assert!(greet.is_definition);
    }

    #[test]
    fn java_parameter_used_twice_is_one_definition_and_one_reference() {
        let text = "class C { void m(String name) { name = name; } }";
        let occurrences = identifier_occurrences(Language::Java, text);
        let names: Vec<&Occurrence> = occurrences.iter().filter(|o| o.name == "name").collect();
        assert_eq!(names.len(), 3, "1 definition + 2 references: {names:?}");
        assert_eq!(names.iter().filter(|o| o.is_definition).count(), 1);
        assert_eq!(names.iter().filter(|o| !o.is_definition).count(), 2);
    }

    // --- PHP (Task B) ---

    const PHP_SNIPPET: &str = "class Greeter {\n    public string $name;\n\n    public function __construct(string $name) {\n        $this->name = $name;\n    }\n\n    public function greet(): string {\n        // say hi\n        return \"Hello, \" . $this->name;\n    }\n}\n";

    #[test]
    fn php_class_keyword_is_highlighted() {
        let spans = highlight(Language::Php, PHP_SNIPPET);
        let span = find(&spans, TokenKind::Keyword).expect("expected a Keyword span");
        assert_eq!(&PHP_SNIPPET[span.start..span.end], "class");
    }

    #[test]
    fn php_string_literal_is_highlighted() {
        let spans = highlight(Language::Php, PHP_SNIPPET);
        let span = find(&spans, TokenKind::String).expect("expected a String span");
        assert_eq!(&PHP_SNIPPET[span.start..span.end], "\"Hello, \"");
    }

    #[test]
    fn php_comment_is_highlighted() {
        let spans = highlight(Language::Php, PHP_SNIPPET);
        let span = find(&spans, TokenKind::Comment).expect("expected a Comment span");
        assert!(&PHP_SNIPPET[span.start..span.end].starts_with("// say hi"));
    }

    #[test]
    fn php_class_name_is_highlighted_as_type() {
        let spans = highlight(Language::Php, PHP_SNIPPET);
        let type_span = spans
            .iter()
            .find(|s| s.kind == TokenKind::Type && &PHP_SNIPPET[s.start..s.end] == "Greeter");
        assert!(type_span.is_some(), "expected `Greeter` highlighted as a Type");
    }

    #[test]
    fn php_method_definition_is_recognized() {
        let occurrences = identifier_occurrences(Language::Php, PHP_SNIPPET);
        let greet = occurrences
            .iter()
            .find(|o| o.name == "greet")
            .expect("expected an occurrence for the method name");
        assert!(greet.is_definition);
    }

    #[test]
    fn php_parameter_used_twice_is_one_definition_and_one_reference() {
        let text = "function add($x) { return $x + $x; }";
        let occurrences = identifier_occurrences(Language::Php, text);
        let names: Vec<&Occurrence> = occurrences.iter().filter(|o| o.name == "$x").collect();
        assert_eq!(names.len(), 3, "1 definition + 2 references: {names:?}");
        assert_eq!(names.iter().filter(|o| o.is_definition).count(), 1);
        assert_eq!(names.iter().filter(|o| !o.is_definition).count(), 2);
    }

    // --- fold_ranges (Task C) ---

    #[test]
    fn plain_text_has_no_fold_ranges() {
        let mut highlighter = Highlighter::new(Language::PlainText);
        highlighter.set_text("hello");
        assert!(highlighter.fold_ranges().is_empty());
    }

    #[test]
    fn fold_ranges_are_empty_before_any_parse() {
        assert!(Highlighter::new(Language::Rust).fold_ranges().is_empty());
    }

    #[test]
    fn rust_function_body_is_foldable() {
        let text = "fn add(x: i32, y: i32) -> i32 {\n    x + y\n}";
        let mut highlighter = Highlighter::new(Language::Rust);
        highlighter.set_text(text);
        let ranges = highlighter.fold_ranges();
        let body = ranges
            .iter()
            .find(|r| &text[r.start..r.end] == "{\n    x + y\n}")
            .expect("expected the function body to be foldable");
        assert_eq!(&text[body.start..body.end], "{\n    x + y\n}");
    }

    #[test]
    fn rust_struct_body_is_foldable() {
        let text = "struct Point {\n    x: i32,\n    y: i32,\n}";
        let mut highlighter = Highlighter::new(Language::Rust);
        highlighter.set_text(text);
        let ranges = highlighter.fold_ranges();
        assert!(
            ranges
                .iter()
                .any(|r| &text[r.start..r.end] == "{\n    x: i32,\n    y: i32,\n}"),
            "expected the struct body to be foldable: {ranges:?}"
        );
    }

    #[test]
    fn json_object_is_foldable() {
        let text = "{\"a\": 1, \"b\": [1, 2, 3]}";
        let mut highlighter = Highlighter::new(Language::Json);
        highlighter.set_text(text);
        let ranges = highlighter.fold_ranges();
        assert!(
            ranges.iter().any(|r| &text[r.start..r.end] == text),
            "expected the whole object to be foldable: {ranges:?}"
        );
        assert!(
            ranges.iter().any(|r| &text[r.start..r.end] == "[1, 2, 3]"),
            "expected the nested array to be foldable: {ranges:?}"
        );
    }

    #[test]
    fn csharp_method_body_is_foldable() {
        let mut highlighter = Highlighter::new(Language::CSharp);
        highlighter.set_text(CSHARP_SNIPPET);
        let ranges = highlighter.fold_ranges();
        assert!(
            ranges
                .iter()
                .any(|r| CSHARP_SNIPPET[r.start..r.end].starts_with("{\n        // say hi")),
            "expected the Greet() body to be foldable: {ranges:?}"
        );
    }

    #[test]
    fn csharp_class_body_is_foldable() {
        let mut highlighter = Highlighter::new(Language::CSharp);
        highlighter.set_text(CSHARP_SNIPPET);
        let ranges = highlighter.fold_ranges();
        assert!(
            ranges.iter().any(|r| r.start == CSHARP_SNIPPET.find('{').unwrap()
                && r.end == CSHARP_SNIPPET.rfind('}').unwrap() + 1),
            "expected the class body to be foldable: {ranges:?}"
        );
    }

    #[test]
    fn java_method_body_is_foldable() {
        let mut highlighter = Highlighter::new(Language::Java);
        highlighter.set_text(JAVA_SNIPPET);
        let ranges = highlighter.fold_ranges();
        assert!(
            ranges
                .iter()
                .any(|r| JAVA_SNIPPET[r.start..r.end].starts_with("{\n        // say hi")),
            "expected the greet() body to be foldable: {ranges:?}"
        );
    }

    #[test]
    fn java_class_body_is_foldable() {
        let mut highlighter = Highlighter::new(Language::Java);
        highlighter.set_text(JAVA_SNIPPET);
        let ranges = highlighter.fold_ranges();
        assert!(
            ranges.iter().any(|r| r.start == JAVA_SNIPPET.find('{').unwrap()
                && r.end == JAVA_SNIPPET.rfind('}').unwrap() + 1),
            "expected the class body to be foldable: {ranges:?}"
        );
    }

    #[test]
    fn php_method_body_is_foldable() {
        let mut highlighter = Highlighter::new(Language::Php);
        highlighter.set_text(PHP_SNIPPET);
        let ranges = highlighter.fold_ranges();
        assert!(
            ranges
                .iter()
                .any(|r| PHP_SNIPPET[r.start..r.end].starts_with("{\n        // say hi")),
            "expected the greet() body to be foldable: {ranges:?}"
        );
    }

    #[test]
    fn php_class_body_is_foldable() {
        let mut highlighter = Highlighter::new(Language::Php);
        highlighter.set_text(PHP_SNIPPET);
        let ranges = highlighter.fold_ranges();
        assert!(
            ranges.iter().any(|r| r.start == PHP_SNIPPET.find('{').unwrap()
                && r.end == PHP_SNIPPET.rfind('}').unwrap() + 1),
            "expected the class body to be foldable: {ranges:?}"
        );
    }

    #[test]
    fn fold_ranges_reflect_incremental_edits() {
        let old_text = "fn foo() {\n    1\n}";
        let new_text = "fn foo() {\n    1 + 2\n}";

        let mut highlighter = Highlighter::new(Language::Rust);
        highlighter.set_text(old_text);
        // Insert " + 2" right after "1" (byte offset 15..15 -> 15..19).
        highlighter.edit(new_text, 15, 15, 19);

        let ranges = highlighter.fold_ranges();
        assert!(
            ranges
                .iter()
                .any(|r| &new_text[r.start..r.end] == "{\n    1 + 2\n}"),
            "expected the fold range to track the edit: {ranges:?}"
        );
    }

    #[test]
    fn json_object_keys_are_references_not_definitions() {
        let text = "{\"key\": \"value\", \"other\": 1}";
        let occurrences = identifier_occurrences(Language::Json, text);

        assert!(
            occurrences.iter().all(|o| !o.is_definition),
            "JSON has no definition sites: {occurrences:?}"
        );
        let names: Vec<&str> = occurrences.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, vec!["\"key\"", "\"other\""]);
    }

    // --- outline() (Task D) ---

    #[test]
    fn plain_text_has_no_outline() {
        assert!(outline(Language::PlainText, "anything").is_empty());
    }

    #[test]
    fn json_has_no_outline() {
        let text = "{\"key\": \"value\", \"nested\": {\"a\": 1}}";
        assert!(outline(Language::Json, text).is_empty());
    }

    #[test]
    fn rust_outline_nests_fields_under_struct_and_methods_under_impl() {
        let text = "struct Point {\n    x: i32,\n    y: i32,\n}\n\nimpl Point {\n    fn new() -> Point { Point { x: 0, y: 0 } }\n    fn dist(&self) -> f64 { 0.0 }\n}\n";
        let roots = outline(Language::Rust, text);

        let point_struct = roots
            .iter()
            .find(|s| s.kind == SymbolKind::Struct && s.name == "Point")
            .expect("expected a Point struct root");
        let field_names: Vec<&str> = point_struct.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(field_names, vec!["x", "y"]);
        assert!(point_struct.children.iter().all(|c| c.kind == SymbolKind::Field));

        let point_impl = roots
            .iter()
            .find(|s| s.kind == SymbolKind::Class && s.name == "Point")
            .expect("expected a Point impl root");
        let method_names: Vec<&str> = point_impl.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(method_names, vec!["new", "dist"]);
        assert!(point_impl.children.iter().all(|c| c.kind == SymbolKind::Function));

        // Name byte ranges point at just the identifier, not the whole
        // definition.
        assert_eq!(&text[point_struct.name_start..point_struct.name_end], "Point");
    }

    #[test]
    fn csharp_outline_nests_methods_under_class() {
        let occurrences = outline(Language::CSharp, CSHARP_SNIPPET);
        let greeter = occurrences
            .iter()
            .find(|s| s.kind == SymbolKind::Class && s.name == "Greeter")
            .expect("expected a Greeter class root");
        let names: Vec<&str> = greeter.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Name", "Greeter", "Greet"]);
        assert_eq!(greeter.children[0].kind, SymbolKind::Field);
        assert_eq!(greeter.children[1].kind, SymbolKind::Method); // constructor
        assert_eq!(greeter.children[2].kind, SymbolKind::Method);
    }

    #[test]
    fn csharp_outline_captures_auto_properties_as_fields() {
        const SNIPPET: &str = "class Person {\n    public string Name { get; set; }\n}\n";
        let roots = outline(Language::CSharp, SNIPPET);
        let person = roots
            .iter()
            .find(|s| s.kind == SymbolKind::Class && s.name == "Person")
            .expect("expected a Person class root");
        assert_eq!(person.children.len(), 1);
        assert_eq!(person.children[0].kind, SymbolKind::Field);
        assert_eq!(person.children[0].name, "Name");
    }

    #[test]
    fn java_outline_nests_methods_under_class() {
        let roots = outline(Language::Java, JAVA_SNIPPET);
        let greeter = roots
            .iter()
            .find(|s| s.kind == SymbolKind::Class && s.name == "Greeter")
            .expect("expected a Greeter class root");
        let names: Vec<&str> = greeter.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["name", "Greeter", "greet"]);
        assert_eq!(greeter.children[0].kind, SymbolKind::Field);
        assert_eq!(greeter.children[1].kind, SymbolKind::Method); // constructor
        assert_eq!(greeter.children[2].kind, SymbolKind::Method);
    }

    #[test]
    fn php_outline_nests_methods_under_class() {
        let roots = outline(Language::Php, PHP_SNIPPET);
        let greeter = roots
            .iter()
            .find(|s| s.kind == SymbolKind::Class && s.name == "Greeter")
            .expect("expected a Greeter class root");
        let names: Vec<&str> = greeter.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["$name", "__construct", "greet"]);
        assert_eq!(greeter.children[0].kind, SymbolKind::Field);
        assert_eq!(greeter.children[1].kind, SymbolKind::Method);
        assert_eq!(greeter.children[2].kind, SymbolKind::Method);
    }
}
