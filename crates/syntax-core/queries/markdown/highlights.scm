; Markdown (block grammar) highlights.scm — adapted from tree-sitter-md
; 0.5.3's own `tree-sitter-markdown/queries/highlights.scm` (MIT,
; https://github.com/tree-sitter-grammars/tree-sitter-markdown), which in
; turn comes from nvim-treesitter.
;
; Kept verbatim including the `@text.*` capture names, which
; `syntax_core::SCOPES` does not know: `text.title` etc. resolve to
; nothing and yield no spans, so headings, link destinations and code
; blocks are currently uncoloured. That is a taxonomy gap, not a query
; bug — fixing it means adding a `markup`/`text` family to `SCOPES` and to
; the view's format table, which is a change to every language at once and
; does not belong in this tranche. Leaving the upstream names in place is
; what makes that later change a one-line table edit.
;
; Markdown produces no keyword, string or comment of its own. The harness
; still finds all three in markdown/sample.txt, because the fenced Rust
; block is injected and highlighted as Rust — see injections.scm.

(atx_heading
  (inline) @text.title)

(setext_heading
  (paragraph) @text.title)

[
  (atx_h1_marker)
  (atx_h2_marker)
  (atx_h3_marker)
  (atx_h4_marker)
  (atx_h5_marker)
  (atx_h6_marker)
  (setext_h1_underline)
  (setext_h2_underline)
] @punctuation.special

[
  (link_title)
  (indented_code_block)
  (fenced_code_block)
] @text.literal

(fenced_code_block_delimiter) @punctuation.delimiter

(link_destination) @text.uri

(link_label) @text.reference

[
  (list_marker_plus)
  (list_marker_minus)
  (list_marker_star)
  (list_marker_dot)
  (list_marker_parenthesis)
  (thematic_break)
] @punctuation.special

[
  (block_continuation)
  (block_quote_marker)
] @punctuation.special

(backslash_escape) @string.escape
