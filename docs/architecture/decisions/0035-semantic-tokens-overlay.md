# ADR-0035: `lsp-core` takes a normal dependency on `syntax-core` for the semantic-tokens overlay

Status: accepted
Date: 2026-09-01
Amends ADR-0018 (`syntax-core` stays a **dev**-dependency of `lsp-core` for language detection).

## Context

C9 asks `lsp-core` to decode `textDocument/semanticTokens/full`, map each token onto `syntax_core`'s scope taxonomy, and overlay the result onto the tree-sitter spans `syntax_core::Highlighter` already produces — semantic-token spans win where they cover a range, tree-sitter spans fill in everywhere else (F0-16: never let "waiting for the server" mean "no colour at all").

That mapping and that overlay both need `syntax_core::Scope`: resolving an LSP token type name to a scope reuses `Scope::resolve`'s own dotted-fallback walk rather than a second copy of it, and the overlay's inputs and output are `syntax_core::HighlightSpan`.

`layering.md`'s `lsp-core` row currently reads `syntax-core` as a **dev**-dependency only, per ADR-0018.
That ADR's reason was specific: two tables both answered "which language is this file?", they drifted apart (issue #20), and the fix was one source of truth for file-to-language detection, with `lsp-core` reaching zero normal edge to `syntax-core` to make the drift structurally impossible again.

## Decision

`lsp-core`'s `Cargo.toml` moves `syntax-core` from `[dev-dependencies]` to `[dependencies]`.
`crates/lsp-core/src/semantic_tokens.rs` is the only module that uses it: `Scope`, for resolving a mapped token type name, and `HighlightSpan`, for the overlay's input/output shape.

This is not the duplication ADR-0018 forbade.
That ADR is about **one interpretation of a file having two possible sources** — language detection is a single fact `syntax-core`'s registry alone must own.
Semantic-token mapping is a different kind of question: it combines two already-produced span lists (the server's tokens, tree-sitter's spans) into one, using a lookup — `Scope::resolve` — that `syntax-core` already exposes as public API for exactly this purpose (`theme::palette`'s own dotted walk is the same function).
No second scope table is introduced, no file-to-language decision is made or re-made here, and `syntax-core` gains no new dependency of its own — the edge is one-directional, `lsp-core -> syntax-core`, same as `index-core`'s existing edge in `layering.md`'s table.

`lsp-core`'s `cargo tree -e normal | grep -i qt` gate (`layering.md`'s verification block) is unaffected: `syntax-core` is itself Qt-free, so this adds no `qt`/`cxx-qt` edge to `lsp-core`'s build graph.

## Consequences

- `layering.md`'s `lsp-core` row changes from *"`syntax-core` as a dev-dependency only, ADR-0018"* to *"`syntax-core` (normal dependency, ADR-0035) for `semantic_tokens`'s scope mapping and overlay; ADR-0018's ban on `lsp-core` re-deciding file-to-language detection still holds — nothing here parses an extension or a language id."*
- The existing dev-dependency comment and regression tests ADR-0018 describes (`every_catalog_language_is_visible_to_the_server_lookup` and its reverse) are unaffected — a normal dependency is a superset of what a dev-dependency already permitted, so nothing that compiled before stops compiling.
- A future crate wanting the same overlay (say, a second adapter) can call `lsp_core::semantic_tokens::overlay` directly rather than re-deriving the merge rule, the same way `ui-shell` already reaches into `lsp_core::diff_preview` for refactor-preview diffing (`layering.md`'s existing note on that edge).
