; CSS highlights.scm — adapted from tree-sitter-css 0.25.0's own
; `queries/highlights.scm` (MIT,
; https://github.com/tree-sitter/tree-sitter-css).
;
; Two upstream patterns are dropped, for the reasons the earlier tranches
; recorded in bash/highlights.scm and go/highlights.scm:
;
;   ((property_name) @variable (#match? @variable "^--"))
;   ((plain_value)   @variable (#match? @variable "^--"))
;
; span extraction does not evaluate predicates, so these would ship
; unguarded and paint *every* property name and every plain value as a
; variable, stacking a `@variable` span under the `@property` one that
; follows. Custom properties therefore highlight as ordinary properties
; and values, which is wrong only in shade, not in kind.
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
