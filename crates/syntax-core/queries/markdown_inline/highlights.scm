; Markdown inline grammar highlights.scm — adapted from tree-sitter-md
; 0.5.3's own `tree-sitter-markdown-inline/queries/highlights.scm` (MIT).
;
; As in markdown/highlights.scm, the upstream `@text.*` capture names are
; spelled as the current `@markup.*` family that `syntax_core::SCOPES`
; knows — the same captures under their newer names.

(code_span) @markup.raw
(link_title) @markup.link.label

[
  (emphasis_delimiter)
  (code_span_delimiter)
] @punctuation.delimiter

(emphasis) @markup.italic

(strong_emphasis) @markup.bold

(strikethrough) @markup.strikethrough

[
  (link_destination)
  (uri_autolink)
] @markup.link.url

[
  (link_label)
  (link_text)
  (image_description)
] @markup.link.label

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
