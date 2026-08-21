; TOML highlights.scm — adapted from tree-sitter-toml-ng 0.7.0's own
; `queries/highlights.scm` (MIT,
; https://github.com/tree-sitter-grammars/tree-sitter-toml).
;
; Two upstream patterns are changed. Upstream captures `(bare_key) @type`
; and then `(pair (bare_key)) @property` — the second captures the *whole
; pair* (key, `=` and value) as a property, which paints one span across
; the value and hides its string/number colouring. Both are replaced by a
; single `(bare_key) @property`: a TOML key is a property, not a type.
;
; There is no `@keyword` pattern and cannot be one — see toml/no-scopes.txt.

(bare_key) @property
(quoted_key) @string

(comment) @comment

(string) @string
(boolean) @boolean

[
  (integer)
  (float)
] @number

[
  (offset_date_time)
  (local_date_time)
  (local_date)
  (local_time)
] @string.special

[
  "."
  ","
] @punctuation.delimiter

"=" @operator

[
  "["
  "]"
  "[["
  "]]"
  "{"
  "}"
] @punctuation.bracket
