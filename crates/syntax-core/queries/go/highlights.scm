; Go highlights.scm — adapted from tree-sitter-go 0.25.0's own
; queries/highlights.scm (MIT, (c) 2014 Max Brunsfeld). Upstream's
; `#match?`-guarded `@function.builtin` pattern is still absent here —
; not because predicates are unevaluated, but because nobody has ported
; it back yet.
;
; This note is the canonical account of how predicates are treated; the
; other adapted files point at it.
;
; Text predicates *are* evaluated: `QueryCursor::matches` filters matches
; on `#eq?`, `#not-eq?`, `#match?`, `#not-match?`, `#any-of?`,
; `#not-any-of?` and their `#any-*` variants (see `spans_from_tree`), so
; a pattern guarded by one of those is safe to ship and behaves as
; upstream intends. Two kinds are still not evaluated:
;
;   * property predicates — `#is? local` / `#is-not? local`, which need a
;     locals-scope resolver this crate does not have;
;   * general predicates — anything tree-sitter itself does not know,
;     notably nvim-treesitter's `#lua-match?`, `#has-ancestor?` and
;     `#has-parent?`.
;
; A pattern carrying one of those is *dropped whole* rather than shipped
; unguarded, so pasting one in would leave it inert. `#set!` is a
; directive, not a predicate, and is simply ignored.
;
; Span extraction also resolves captures on the same node
; first-pattern-wins, so a catch-all placed after the specific patterns
; loses to them instead of stacking a second span under each.

; Function calls

(call_expression
  function: (identifier) @function)

(call_expression
  function: (selector_expression
    field: (field_identifier) @function.method))

; Function definitions

(function_declaration
  name: (identifier) @function)

(method_declaration
  name: (field_identifier) @function.method)

; Identifiers

(type_identifier) @type
(field_identifier) @property

; Operators

[
  "--"
  "-"
  "-="
  ":="
  "!"
  "!="
  "..."
  "*"
  "*="
  "/"
  "/="
  "&"
  "&&"
  "&="
  "%"
  "%="
  "^"
  "^="
  "+"
  "++"
  "+="
  "<-"
  "<"
  "<<"
  "<<="
  "<="
  "="
  "=="
  ">"
  ">="
  ">>"
  ">>="
  "|"
  "|="
  "||"
  "~"
] @operator

; Keywords

[
  "break"
  "case"
  "chan"
  "const"
  "continue"
  "default"
  "defer"
  "else"
  "fallthrough"
  "for"
  "func"
  "go"
  "goto"
  "if"
  "import"
  "interface"
  "map"
  "package"
  "range"
  "return"
  "select"
  "struct"
  "switch"
  "type"
  "var"
] @keyword

; Literals

[
  (interpreted_string_literal)
  (raw_string_literal)
  (rune_literal)
] @string

(escape_sequence) @escape

[
  (int_literal)
  (float_literal)
  (imaginary_literal)
] @number

[
  (true)
  (false)
  (nil)
  (iota)
] @constant.builtin

(comment) @comment

; --- Naming conventions -----------------------------------------------
;
; Guarded by `#match?` text predicates, which `QueryCursor::matches` does
; evaluate (see `spans_from_tree`). They sit last on purpose: captures on
; the same node resolve first-pattern-wins, so every specific pattern
; above still beats these catch-alls.

; SCREAMING_CASE is a constant. Two characters minimum, so a bare
; `T` stays a type rather than becoming a constant.
((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))

; CamelCase is a type.
((identifier) @type
  (#match? @type "^[A-Z]"))

; The catch-all, last of all: with same-node captures resolving
; first-pattern-wins, every pattern above — including the two
; conventions — beats it, and it only reaches the identifiers nothing
; else claimed.
(identifier) @variable
