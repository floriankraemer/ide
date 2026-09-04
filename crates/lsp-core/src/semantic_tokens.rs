//! `textDocument/semanticTokens`: what a server advertises about it, how to
//! decode the delta-encoded wire format, how one decoded token maps onto
//! `syntax_core`'s closed scope taxonomy, and how the result is overlaid on
//! top of the tree-sitter spans the editor already has (C9, ADR-0035).
//!
//! Everything here is pure data-in-data-out — no request is sent from this
//! module, so it is independently unit-testable with no server running.
//! `manager.rs` owns the request/response plumbing and where the legend is
//! read from (`initialize`'s static capability, or a dynamic
//! `client/registerCapability` — csharp-ls uses the latter).
//!
//! F0-16's lesson applies here as much as it did to progress reporting:
//! tree-sitter still paints while the server is indexing, or has not
//! answered yet at all, so [`overlay`] never lets "waiting for the server"
//! mean "no colour at all" for text that already had some a moment ago.

use serde_json::{json, Value};
use syntax_core::{HighlightSpan, Scope};

use crate::manager::{LspError, SEMANTIC_TOKENS_TIMEOUT};

/// The LSP 3.17 standard semantic token types, in the order this client
/// advertises them in `textDocument.semanticTokens.tokenTypes`
/// (`manager::client_capabilities`) — kept next to [`base_scope_name`] since
/// that is the one place their spelling matters.
pub const STANDARD_TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "event",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
];

/// The LSP 3.17 standard semantic token modifiers, advertised the same way.
pub const STANDARD_TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "definition",
    "readonly",
    "static",
    "deprecated",
    "abstract",
    "async",
    "modification",
    "documentation",
    "defaultLibrary",
];

/// What a server said about `textDocument/semanticTokens` — either in its
/// `initialize` result (`capabilities.semanticTokensProvider`) or in a
/// dynamic `client/registerCapability` registration's `registerOptions`,
/// which is the same shape (`SemanticTokensRegistrationOptions` extends
/// `SemanticTokensOptions` with nothing this client reads).
///
/// The legend's two arrays are the server's own vocabulary: a decoded
/// token's `token_type`/`modifiers` are indices/bits into these, in the
/// order the server sent them — never sorted or deduplicated, since order is
/// the encoding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticTokensLegend {
    pub token_types: Vec<String>,
    pub token_modifiers: Vec<String>,
    /// `full: true` or `full: {delta: true}`.
    pub full: bool,
    /// Whether `textDocument/semanticTokens/full/delta` is also offered.
    /// Read and stored now so a later delta implementation is one method
    /// added to `manager.rs`, not a second capability-parsing pass; C9
    /// itself only ever sends the full request.
    pub full_delta: bool,
    pub range: bool,
}

/// Parses a `SemanticTokensOptions`-shaped value — either
/// `initialize`'s `capabilities.semanticTokensProvider` or a
/// `client/registerCapability` registration's `registerOptions` for
/// `textDocument/semanticTokens`, which share this shape.
pub fn parse_provider(provider: &Value) -> Option<SemanticTokensLegend> {
    let legend = provider.get("legend")?;
    let token_types = string_array(legend.get("tokenTypes"));
    let token_modifiers = string_array(legend.get("tokenModifiers"));
    let (full, full_delta) = match provider.get("full") {
        Some(Value::Bool(supported)) => (*supported, false),
        Some(Value::Object(options)) => (
            true,
            options
                .get("delta")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        _ => (false, false),
    };
    let range = matches!(
        provider.get("range"),
        Some(Value::Bool(true)) | Some(Value::Object(_))
    );
    Some(SemanticTokensLegend {
        token_types,
        token_modifiers,
        full,
        full_delta,
        range,
    })
}

/// Parses the legend from an `initialize` result, at
/// `/capabilities/semanticTokensProvider`.
pub fn parse_legend(init_result: &Value) -> Option<SemanticTokensLegend> {
    parse_provider(init_result.pointer("/capabilities/semanticTokensProvider")?)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// One decoded token, in absolute (not delta-encoded) terms: a 0-based
/// `line`/`start_char` position, counted in UTF-16 code units per the
/// protocol's own encoding, and indices/a bitmask into the legend that
/// decoded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticToken {
    pub line: u32,
    pub start_char: u32,
    pub length: u32,
    pub token_type: u32,
    pub modifiers: u32,
}

/// Decodes a `SemanticTokens.data` array: five integers per token,
/// `[deltaLine, deltaStartChar, length, tokenType, tokenModifiers]`, each
/// delta-encoded relative to the *previous* token (the first token is
/// relative to line 0, character 0).
///
/// `deltaLine == 0` means "same line as the previous token", in which case
/// `deltaStartChar` is relative to the previous token's start column rather
/// than to column 0 — the one rule in this encoding that is easy to get
/// backwards, so it has its own tests below with a same-line, multi-token
/// case.
///
/// A `data` array whose length is not a multiple of five is truncated to
/// the last complete token: a server that sends a malformed array should
/// lose the trailing partial entry, not panic this client.
pub fn decode(data: &[u64]) -> Vec<SemanticToken> {
    let mut tokens = Vec::with_capacity(data.len() / 5);
    let mut line = 0u32;
    let mut start_char = 0u32;
    for chunk in data.chunks_exact(5) {
        let delta_line = chunk[0] as u32;
        let delta_start_char = chunk[1] as u32;
        if delta_line > 0 {
            line += delta_line;
            start_char = delta_start_char;
        } else {
            start_char += delta_start_char;
        }
        tokens.push(SemanticToken {
            line,
            start_char,
            length: chunk[2] as u32,
            token_type: chunk[3] as u32,
            modifiers: chunk[4] as u32,
        });
    }
    tokens
}

/// Parses a `textDocument/semanticTokens/full` response:
/// `{resultId?: string, data: number[]}`. `None` for a response with no
/// `data` array at all (a server answering `null` because it has nothing
/// to say, same convention every other L-series method in this crate
/// follows).
pub fn parse_full_response(result: &Value) -> Option<(Option<String>, Vec<SemanticToken>)> {
    let data: Vec<u64> = result
        .get("data")?
        .as_array()?
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    let result_id = result
        .get("resultId")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((result_id, decode(&data)))
}

/// Maps one LSP standard token type name onto the dotted scope string
/// `syntax_core::Scope::resolve` should be asked to resolve.
///
/// Most names are already the taxonomy's own spelling (`"function"`,
/// `"keyword"`, `"string"`, ...) and pass straight through — `other =>
/// other` below, which is also what makes an LSP type name this table does
/// not special-case reach [`Scope::resolve`]'s own dotted-walk-then-drop
/// fallback unchanged, exactly as the plan requires: this function adds no
/// second fallback mechanism, it only supplies the string to walk.
///
/// The rest are the handful where the LSP vocabulary and this taxonomy
/// disagree on a name for the same concept: `class`/`enum`/`interface`/
/// `struct`/`typeParameter` all read as a `type` reference here, the same
/// way tree-sitter's own `@type` capture covers them; `enumMember` reads as
/// a `constant`, matching how a name declared once and never reassigned is
/// coloured everywhere else in this taxonomy; `event`/`modifier` have no
/// scope of their own and fall back to the nearest cousin they behave like
/// (`property`, `keyword`); `regexp`, `decorator`, `namespace`, `method` and
/// `macro` are simply spelled differently in the two vocabularies
/// (`string.regexp`, `attribute`, `module`, `function.method`,
/// `function.macro`).
fn base_scope_name(lsp_type: &str) -> &str {
    match lsp_type {
        "class" | "enum" | "interface" | "struct" | "typeParameter" => "type",
        "enumMember" => "constant",
        "event" => "property",
        "modifier" => "keyword",
        "regexp" => "string.regexp",
        "decorator" => "attribute",
        "namespace" => "module",
        "method" => "function.method",
        "macro" => "function.macro",
        "parameter" => "variable.parameter",
        other => other,
    }
}

/// Resolves one decoded token to a taxonomy [`Scope`], via `legend` for the
/// server's own type/modifier vocabulary. `None` when the token's type
/// index is out of range for the legend (a malformed response) or when
/// neither the refined nor the base scope name resolves to anything in the
/// taxonomy — dropped, same as an unrecognized tree-sitter capture.
///
/// The only modifier this reads is `defaultLibrary` (`bool`/`int`/`String`
/// in a language's standard library), refining e.g. `"function"` to
/// `"function.builtin"` — a distinction the taxonomy already draws for
/// tree-sitter's own `@function.builtin` capture. `Scope::resolve`'s own
/// dotted walk is what makes this safe to attempt unconditionally: a
/// `.builtin` refinement that does not exist for a given base (there is no
/// `"module.builtin"`) falls straight back to the base scope rather than
/// being dropped, so trying it first never loses information.
pub fn scope_for(legend: &SemanticTokensLegend, token: &SemanticToken) -> Option<Scope> {
    let type_name = legend.token_types.get(token.token_type as usize)?;
    let base = base_scope_name(type_name);
    let is_default_library = legend
        .token_modifiers
        .iter()
        .enumerate()
        .any(|(i, name)| name == "defaultLibrary" && i < 32 && token.modifiers & (1 << i) != 0);
    if is_default_library {
        if let Some(scope) = Scope::resolve(&format!("{base}.builtin")) {
            return Some(scope);
        }
    }
    Scope::resolve(base)
}

/// One semantic-token span, already resolved to a scope and converted to
/// the byte-range shape `HighlightSpan` uses (`editor_core::offsets` is
/// what a caller reaches for to get from the protocol's UTF-16 line/character
/// positions to this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappedSpan {
    pub start: usize,
    pub end: usize,
    pub scope: Scope,
}

/// Overlays `semantic` spans onto `tree_sitter` spans: wherever a semantic
/// span covers a byte range, it wins (the server is more accurate when it
/// has answered); everywhere else, the tree-sitter span underneath still
/// shows through.
///
/// This is the F0-16-lesson part of C9: a document with no semantic tokens
/// yet (server still starting, still indexing, or answered `null`) simply
/// returns `tree_sitter` unchanged rather than blanking the file, and a
/// document with semantic tokens for only part of its range (a server that
/// only covers the visible viewport, or one whose answer arrived mid-edit)
/// keeps tree-sitter's colouring for the rest.
///
/// Both inputs are assumed to already be in document order and
/// non-overlapping *within* each list, which both a real
/// `Highlighter::set_text`/`edit` and a real decoded token stream are —
/// this does not re-sort or validate that, since a caller passing spans
/// that vandalize their own invariant gets a merge that reflects it.
pub fn overlay(tree_sitter: &[HighlightSpan], semantic: &[MappedSpan]) -> Vec<HighlightSpan> {
    if semantic.is_empty() {
        return tree_sitter.to_vec();
    }
    if tree_sitter.is_empty() {
        return semantic
            .iter()
            .map(|s| HighlightSpan {
                start: s.start,
                end: s.end,
                scope: s.scope,
            })
            .collect();
    }

    let mut boundaries: Vec<usize> = Vec::with_capacity(tree_sitter.len() * 2 + semantic.len() * 2);
    for s in tree_sitter {
        boundaries.push(s.start);
        boundaries.push(s.end);
    }
    for s in semantic {
        boundaries.push(s.start);
        boundaries.push(s.end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut out: Vec<HighlightSpan> = Vec::new();
    for pair in boundaries.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end {
            continue;
        }
        let scope = semantic
            .iter()
            .find(|s| s.start <= start && s.end >= end)
            .map(|s| s.scope)
            .or_else(|| {
                tree_sitter
                    .iter()
                    .find(|s| s.start <= start && s.end >= end)
                    .map(|s| s.scope)
            });
        let Some(scope) = scope else { continue };
        match out.last_mut() {
            Some(last) if last.end == start && last.scope == scope => last.end = end,
            _ => out.push(HighlightSpan { start, end, scope }),
        }
    }
    out
}

// C4-followup (#162): request-sending `LspManager` methods for this feature, moved out of
// `manager.rs` once it crossed the file-size ceiling. This file already held the
// parse/rule layer; this is the request-sending half `manager.rs`'s own module doc
// pointed callers to.
impl crate::manager::LspManager {
    /// `textDocument/semanticTokens/full` for a whole open document, raw —
    /// decoding the response and mapping it onto `syntax_core`'s taxonomy is
    /// `crate::semantic_tokens`'s job, not this method's, the same
    /// convention [`Self::resolve_completion_item`] and
    /// [`Self::execute_command`] follow for a server's raw JSON.
    ///
    /// Whether it is worth calling at all is
    /// [`Self::semantic_tokens_legend`]'s answer, not this method's — C9
    /// only ever sends the full request, never
    /// `textDocument/semanticTokens/full/delta`, so there is no `previous_result_id`
    /// parameter to thread through yet.
    pub fn semantic_tokens(&self, language_id: &str, uri: &str) -> Result<Value, LspError> {
        self.request_with_timeout(
            language_id,
            "textDocument/semanticTokens/full",
            json!({"textDocument": {"uri": uri}}),
            SEMANTIC_TOKENS_TIMEOUT,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope(name: &str) -> Scope {
        Scope::resolve(name).unwrap_or_else(|| panic!("no such scope: {name}"))
    }

    // ---- legend parsing ---------------------------------------------

    #[test]
    fn parse_legend_reads_a_bool_full_provider() {
        let init_result = json!({"capabilities": {"semanticTokensProvider": {
            "legend": {"tokenTypes": ["namespace", "type"], "tokenModifiers": ["readonly"]},
            "full": true,
        }}});
        let legend = parse_legend(&init_result).expect("legend present");
        assert_eq!(legend.token_types, vec!["namespace", "type"]);
        assert_eq!(legend.token_modifiers, vec!["readonly"]);
        assert!(legend.full);
        assert!(!legend.full_delta);
        assert!(!legend.range);
    }

    #[test]
    fn parse_legend_reads_delta_and_range_support() {
        let init_result = json!({"capabilities": {"semanticTokensProvider": {
            "legend": {"tokenTypes": [], "tokenModifiers": []},
            "full": {"delta": true},
            "range": true,
        }}});
        let legend = parse_legend(&init_result).expect("legend present");
        assert!(legend.full);
        assert!(legend.full_delta);
        assert!(legend.range);
    }

    #[test]
    fn parse_legend_is_none_when_the_server_never_advertised_it() {
        assert!(parse_legend(&json!({"capabilities": {}})).is_none());
        assert!(parse_legend(&json!({})).is_none());
    }

    #[test]
    fn parse_provider_reads_a_dynamic_registration_s_register_options() {
        // `client/registerCapability`'s `registerOptions` for
        // `textDocument/semanticTokens` is the same shape as the static
        // capability — csharp-ls's own path (C4's dynamic registration).
        let register_options = json!({
            "legend": {"tokenTypes": ["keyword"], "tokenModifiers": []},
            "full": true,
        });
        let legend = parse_provider(&register_options).expect("legend present");
        assert_eq!(legend.token_types, vec!["keyword"]);
        assert!(legend.full);
    }

    // ---- delta decoding -----------------------------------------------

    #[test]
    fn decode_a_single_token_relative_to_the_document_origin() {
        let tokens = decode(&[2, 5, 3, 0, 0]);
        assert_eq!(
            tokens,
            vec![SemanticToken {
                line: 2,
                start_char: 5,
                length: 3,
                token_type: 0,
                modifiers: 0
            }]
        );
    }

    #[test]
    fn decode_multi_line_tokens_accumulate_the_line_delta() {
        // Token 1: line 0, char 0, len 3.
        // Token 2: two lines further down, char 4, len 5 — deltaLine > 0
        // resets the column to deltaStartChar rather than adding it.
        // Token 3: one more line down, char 0.
        let tokens = decode(&[0, 0, 3, 5, 0, 2, 4, 5, 8, 1, 1, 0, 2, 15, 0]);
        assert_eq!(
            tokens,
            vec![
                SemanticToken {
                    line: 0,
                    start_char: 0,
                    length: 3,
                    token_type: 5,
                    modifiers: 0
                },
                SemanticToken {
                    line: 2,
                    start_char: 4,
                    length: 5,
                    token_type: 8,
                    modifiers: 1
                },
                SemanticToken {
                    line: 3,
                    start_char: 0,
                    length: 2,
                    token_type: 15,
                    modifiers: 0
                },
            ]
        );
    }

    #[test]
    fn decode_same_line_tokens_add_the_column_delta_to_the_previous_start() {
        // Three tokens on line 0: columns 0, then +4 -> 4, then +2 -> 6.
        // `deltaLine == 0` for the second and third is what makes this the
        // "same line" case, where `deltaStartChar` is relative to the
        // previous token's own start column, not to column 0.
        let tokens = decode(&[0, 0, 3, 0, 0, 0, 4, 1, 1, 0, 0, 2, 1, 2, 0]);
        assert_eq!(tokens[0].start_char, 0);
        assert_eq!(tokens[1].start_char, 4);
        assert_eq!(tokens[2].start_char, 6);
        assert!(tokens.iter().all(|t| t.line == 0));
    }

    #[test]
    fn decode_ignores_a_trailing_partial_token() {
        let tokens = decode(&[0, 0, 3, 0, 0, 1, 2]);
        assert_eq!(tokens.len(), 1);
    }

    #[test]
    fn parse_full_response_reads_the_result_id_and_data() {
        let result = json!({"resultId": "1", "data": [0, 0, 3, 0, 0]});
        let (result_id, tokens) = parse_full_response(&result).expect("data present");
        assert_eq!(result_id, Some("1".to_string()));
        assert_eq!(tokens.len(), 1);
    }

    #[test]
    fn parse_full_response_is_none_without_a_data_array() {
        assert!(parse_full_response(&json!({"resultId": "1"})).is_none());
        assert!(parse_full_response(&Value::Null).is_none());
    }

    // ---- legend -> scope mapping ---------------------------------------

    fn legend() -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: STANDARD_TOKEN_TYPES.iter().map(|s| s.to_string()).collect(),
            token_modifiers: STANDARD_TOKEN_MODIFIERS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            full: true,
            full_delta: false,
            range: false,
        }
    }

    fn token_of(type_name: &str, modifiers: u32) -> SemanticToken {
        let index = STANDARD_TOKEN_TYPES
            .iter()
            .position(|t| *t == type_name)
            .expect("known standard type");
        SemanticToken {
            line: 0,
            start_char: 0,
            length: 1,
            token_type: index as u32,
            modifiers,
        }
    }

    #[test]
    fn known_lsp_types_resolve_to_their_taxonomy_scope() {
        let legend = legend();
        let cases: &[(&str, &str)] = &[
            ("function", "function"),
            ("method", "function.method"),
            ("macro", "function.macro"),
            ("keyword", "keyword"),
            ("string", "string"),
            ("number", "number"),
            ("comment", "comment"),
            ("variable", "variable"),
            ("parameter", "variable.parameter"),
            ("property", "property"),
            ("operator", "operator"),
            ("class", "type"),
            ("enum", "type"),
            ("interface", "type"),
            ("struct", "type"),
            ("typeParameter", "type"),
            ("enumMember", "constant"),
            ("event", "property"),
            ("modifier", "keyword"),
            ("regexp", "string.regexp"),
            ("decorator", "attribute"),
            ("namespace", "module"),
        ];
        for (lsp_type, expected) in cases {
            let resolved = scope_for(&legend, &token_of(lsp_type, 0));
            assert_eq!(
                resolved,
                Some(scope(expected)),
                "{lsp_type} should map to {expected}"
            );
        }
    }

    #[test]
    fn default_library_modifier_refines_to_the_builtin_scope() {
        let legend = legend();
        let default_library_bit = STANDARD_TOKEN_MODIFIERS
            .iter()
            .position(|m| *m == "defaultLibrary")
            .expect("defaultLibrary is standard");
        let token = token_of("function", 1 << default_library_bit);
        assert_eq!(scope_for(&legend, &token), Some(scope("function.builtin")));
    }

    #[test]
    fn default_library_modifier_falls_back_when_no_builtin_variant_exists() {
        // "module.builtin" is not in the taxonomy, so `Scope::resolve`'s own
        // dotted walk must land back on "module" rather than dropping the
        // token — this is what makes trying the refinement unconditionally
        // safe rather than a second fallback mechanism of this module's own.
        let legend = legend();
        let default_library_bit = STANDARD_TOKEN_MODIFIERS
            .iter()
            .position(|m| *m == "defaultLibrary")
            .expect("defaultLibrary is standard");
        let token = token_of("namespace", 1 << default_library_bit);
        assert_eq!(scope_for(&legend, &token), Some(scope("module")));
    }

    #[test]
    fn a_modifier_this_module_does_not_special_case_changes_nothing() {
        let legend = legend();
        let readonly_bit = STANDARD_TOKEN_MODIFIERS
            .iter()
            .position(|m| *m == "readonly")
            .expect("readonly is standard");
        let token = token_of("variable", 1 << readonly_bit);
        assert_eq!(scope_for(&legend, &token), Some(scope("variable")));
    }

    #[test]
    fn an_unknown_lsp_type_falls_through_the_taxonomy_s_own_dotted_walk() {
        let mut legend = legend();
        legend.token_types.push("selfParameter".to_string());
        let token = SemanticToken {
            line: 0,
            start_char: 0,
            length: 1,
            token_type: legend.token_types.len() as u32 - 1,
            modifiers: 0,
        };
        // Not a dotted name and not in the taxonomy at all: dropped, same
        // as an unrecognized tree-sitter capture would be.
        assert_eq!(scope_for(&legend, &token), None);
    }

    #[test]
    fn a_token_type_index_past_the_legend_is_dropped_not_a_panic() {
        let legend = legend();
        let token = SemanticToken {
            line: 0,
            start_char: 0,
            length: 1,
            token_type: 9999,
            modifiers: 0,
        };
        assert_eq!(scope_for(&legend, &token), None);
    }

    // ---- overlay ---------------------------------------------------------

    fn ts(start: usize, end: usize, name: &str) -> HighlightSpan {
        HighlightSpan {
            start,
            end,
            scope: scope(name),
        }
    }

    fn sem(start: usize, end: usize, name: &str) -> MappedSpan {
        MappedSpan {
            start,
            end,
            scope: scope(name),
        }
    }

    #[test]
    fn no_semantic_tokens_leaves_tree_sitter_spans_unchanged() {
        let base = vec![ts(0, 3, "keyword"), ts(4, 7, "variable")];
        assert_eq!(overlay(&base, &[]), base);
    }

    #[test]
    fn semantic_spans_override_the_overlapping_tree_sitter_span() {
        // Tree-sitter guessed `variable`; the server knows it is a type.
        let base = vec![ts(0, 4, "variable")];
        let semantic = vec![sem(0, 4, "type")];
        assert_eq!(overlay(&base, &semantic), vec![ts(0, 4, "type")]);
    }

    #[test]
    fn tree_sitter_fills_in_what_semantic_tokens_do_not_cover() {
        // The server only answered for byte 0..4; tree-sitter's colouring
        // for the rest of the document must still show through — this is
        // the F0-16 lesson: never blank text that already had colour.
        let base = vec![ts(0, 4, "variable"), ts(5, 9, "keyword")];
        let semantic = vec![sem(0, 4, "type")];
        assert_eq!(
            overlay(&base, &semantic),
            vec![ts(0, 4, "type"), ts(5, 9, "keyword")]
        );
    }

    #[test]
    fn a_semantic_span_with_no_tree_sitter_underneath_still_paints() {
        let base: Vec<HighlightSpan> = vec![];
        let semantic = vec![sem(0, 4, "type")];
        assert_eq!(overlay(&base, &semantic), vec![ts(0, 4, "type")]);
    }

    #[test]
    fn adjacent_equal_scope_output_spans_are_merged() {
        let base = vec![ts(0, 4, "type"), ts(4, 8, "type")];
        let semantic = vec![sem(0, 8, "type")];
        assert_eq!(overlay(&base, &semantic), vec![ts(0, 8, "type")]);
    }

    #[test]
    fn a_semantic_span_narrower_than_the_tree_sitter_span_it_overlaps_only_wins_its_own_range() {
        // Tree-sitter has one wide `function.method` call span; the server
        // only answered for its first half (e.g. a partial re-request).
        let base = vec![ts(0, 10, "function.method")];
        let semantic = vec![sem(0, 5, "keyword")];
        assert_eq!(
            overlay(&base, &semantic),
            vec![ts(0, 5, "keyword"), ts(5, 10, "function.method")]
        );
    }
}
