; Markdown (block grammar) highlights.scm — adapted from tree-sitter-md
; 0.5.3's own `tree-sitter-markdown/queries/highlights.scm` (MIT,
; https://github.com/tree-sitter-grammars/tree-sitter-markdown), which in
; turn comes from nvim-treesitter.
;
; The upstream `@text.*` capture names are spelled here as the current
; `@markup.*` family that `syntax_core::SCOPES` knows; the two are the old
; and new nvim-treesitter names for the same captures.
;
; Heading levels are captured separately (`markup.heading.1` ...) so a
; theme can size or colour an H1 differently; every level falls back to
; `markup.heading` through `Scope::resolve`.
;
; The capture sits on the whole heading node rather than on its `(inline)`
; content, because that content is handed to the inline grammar as an
; injection: an enclosing span survives the injection merge and is painted
; under the inline spans, a span equal to the injected region does not.
;
; Markdown produces no keyword, string or comment of its own. The harness
; still finds all three in markdown/sample.txt, because the fenced Rust
; block is injected and highlighted as Rust — see injections.scm.

(atx_heading (atx_h1_marker)) @markup.heading.1
(atx_heading (atx_h2_marker)) @markup.heading.2
(atx_heading (atx_h3_marker)) @markup.heading.3
(atx_heading (atx_h4_marker)) @markup.heading.4
(atx_heading (atx_h5_marker)) @markup.heading.5
(atx_heading (atx_h6_marker)) @markup.heading.6

(setext_heading (setext_h1_underline)) @markup.heading.1
(setext_heading (setext_h2_underline)) @markup.heading.2

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
  (indented_code_block)
  (fenced_code_block)
] @markup.raw.block

(fenced_code_block_delimiter) @punctuation.delimiter

(link_title) @markup.link.label
(link_destination) @markup.link.url
(link_label) @markup.link.label

[
  (list_marker_plus)
  (list_marker_minus)
  (list_marker_star)
  (list_marker_dot)
  (list_marker_parenthesis)
] @markup.list

(block_quote_marker) @markup.quote

[
  (thematic_break)
  (block_continuation)
] @punctuation.special

(backslash_escape) @string.escape
