; Lua highlights.scm — adapted from tree-sitter-lua 0.5.0's own
; queries/highlights.scm (MIT,
; https://github.com/tree-sitter-grammars/tree-sitter-lua).
;
; Three systematic differences from upstream; go/highlights.scm documents
; the predicate rules they follow:
;
;   * the catch-all `(identifier) @variable` is still absent — span
;     extraction resolves same-node captures first-pattern-wins now, so a
;     catch-all placed last would lose to every function name, field and
;     parameter instead of stacking under them; it has simply not been
;     ported back;
;   * the patterns guarded by `#eq?`, `#match?` or `#any-of?` are also
;     still absent, not because those predicates go unevaluated (they are
;     evaluated, see `spans_from_tree`) but because the upstream
;     builtin-function list has not been ported back yet;
;   * Neovim-flavoured capture names are rewritten to the standard ones in
;     `syntax_core::SCOPES` (`@conditional`/`@repeat` -> `@keyword`,
;     `@field` -> `@variable.member`, `@parameter` -> `@variable.parameter`,
;     `@method` -> `@function.method`, `@preproc` -> `@comment`).

; Keywords
"return" @keyword

[
  "goto"
  "in"
  "local"
  "global"
] @keyword

(label_statement) @label

(break_statement) @keyword

(do_statement
  [
    "do"
    "end"
  ] @keyword)

(while_statement
  [
    "while"
    "do"
    "end"
  ] @keyword)

(repeat_statement
  [
    "repeat"
    "until"
  ] @keyword)

(if_statement
  [
    "if"
    "elseif"
    "else"
    "then"
    "end"
  ] @keyword)

(elseif_statement
  [
    "elseif"
    "then"
    "end"
  ] @keyword)

(else_statement
  [
    "else"
    "end"
  ] @keyword)

(for_statement
  [
    "for"
    "do"
    "end"
  ] @keyword)

(function_declaration
  [
    "function"
    "end"
  ] @keyword)

(function_definition
  [
    "function"
    "end"
  ] @keyword)

; Operators
(binary_expression
  operator: _ @operator)

(unary_expression
  operator: _ @operator)

"=" @operator

[
  "and"
  "not"
  "or"
] @keyword

; Punctuations
[
  ";"
  ":"
  ","
  "."
] @punctuation.delimiter

; Brackets
[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

; Constants
(variable_list
  (attribute
    "<" @punctuation.bracket
    (identifier) @attribute
    ">" @punctuation.bracket))

(vararg_expression) @constant

(nil) @constant.builtin

[
  (false)
  (true)
] @boolean

; Tables
(field
  name: (identifier) @variable.member)

(dot_index_expression
  field: (identifier) @variable.member)

(table_constructor
  [
    "{"
    "}"
  ] @constructor)

; Functions
(parameters
  (identifier) @variable.parameter)

(function_declaration
  name: [
    (identifier) @function
    (dot_index_expression
      field: (identifier) @function)
  ])

(function_declaration
  name: (method_index_expression
    method: (identifier) @function.method))

(assignment_statement
  (variable_list
    .
    name: [
      (identifier) @function
      (dot_index_expression
        field: (identifier) @function)
    ])
  (expression_list
    .
    value: (function_definition)))

(table_constructor
  (field
    name: (identifier) @function
    value: (function_definition)))

(function_call
  name: [
    (identifier) @function.call
    (dot_index_expression
      field: (identifier) @function.call)
    (method_index_expression
      method: (identifier) @function.method)
  ])

; Others
(comment) @comment

(hash_bang_line) @comment

(number) @number

(string) @string

(escape_sequence) @string.escape

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
