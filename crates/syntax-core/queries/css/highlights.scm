; CSS highlights.scm — adapted from tree-sitter-css 0.25.0's own
; `queries/highlights.scm` (MIT,
; https://github.com/tree-sitter/tree-sitter-css).
;
; Upstream's two `#match?`-guarded custom-property patterns are ported
; back below: `#match?` is evaluated (see queries/go/highlights.scm), and
; because same-node captures resolve first-pattern-wins they sit above
; the general `(property_name) @property` / `(plain_value)` rules so a
; `--custom-prop` paints as `@variable` rather than as an ordinary
; property or value.
;
; Upstream's anonymous at-rule tokens (`"@media"`, `"@import"`, …) are
; folded into one `[...] @keyword` list here. They are node literals, not
; captures — the leading `@` belongs to the CSS token, not to a scope
; name.
;
; At-rules are the one thing CSS has that reads as a keyword — `@media`,
; `@import`, `!important` — so there is no `no-scopes.txt` here.

(comment) @comment

(tag_name) @tag
(nesting_selector) @tag
(universal_selector) @tag

[
  "~"
  ">"
  "+"
  "-"
  "*"
  "/"
  "="
  "^="
  "|="
  "~="
  "$="
  "*="
] @operator

[
  "and"
  "or"
  "not"
  "only"
] @operator

(attribute_selector (plain_value) @string)

((property_name) @variable
  (#match? @variable "^--"))

((plain_value) @variable
  (#match? @variable "^--"))

(class_name) @property
(id_name) @property
(namespace_name) @property
(property_name) @property
(feature_name) @property

(pseudo_element_selector (tag_name) @attribute)
(pseudo_class_selector (class_name) @attribute)
(attribute_name) @attribute

(function_name) @function

[
  "@media"
  "@import"
  "@charset"
  "@namespace"
  "@supports"
  "@keyframes"
  (at_keyword)
  (to)
  (from)
  (important)
] @keyword

(string_value) @string
(color_value) @string.special

(integer_value) @number
(float_value) @number
(unit) @type

[
  "#"
  ","
  "."
  ":"
  "::"
  ";"
] @punctuation.delimiter

[
  "{"
  ")"
  "("
  "}"
] @punctuation.bracket
