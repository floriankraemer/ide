; Markdown inline grammar highlights.scm — tree-sitter-md 0.5.3's own
; `tree-sitter-markdown-inline/queries/highlights.scm` (MIT), unchanged.
;
; As in markdown/highlights.scm, the `@text.*` capture names have no entry
; in `syntax_core::SCOPES` and therefore yield no spans today; they are
; left as upstream wrote them so that adding a `markup`/`text` family to
; the taxonomy later is a table edit and not a rewrite of every query.

[
  (code_span)
  (link_title)
] @text.literal

[
  (emphasis_delimiter)
  (code_span_delimiter)
] @punctuation.delimiter

(emphasis) @text.emphasis

(strong_emphasis) @text.strong

[
  (link_destination)
  (uri_autolink)
] @text.uri

[
  (link_label)
  (link_text)
  (image_description)
] @text.reference

[
  (backslash_escape)
  (hard_line_break)
] @string.escape

(image
  [
    "!"
    "["
    "]"
    "("
    ")"
  ] @punctuation.delimiter)

(inline_link
  [
    "["
    "]"
    "("
    ")"
  ] @punctuation.delimiter)

(shortcut_link
  [
    "["
    "]"
  ] @punctuation.delimiter)
