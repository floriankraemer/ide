; Lua highlights.scm — adapted from tree-sitter-lua 0.5.0's own
; queries/highlights.scm (MIT,
; https://github.com/tree-sitter-grammars/tree-sitter-lua).
;
; Upstream carries three predicate-guarded patterns (`#eq?`, `#match?`,
; `#any-of?`); all three are ported. Two systematic differences remain;
; go/highlights.scm documents the predicate rules they follow:
;
;   * the catch-all `(identifier) @variable` is still absent — span
;     extraction resolves same-node captures first-pattern-wins now, so a
;     catch-all placed last would lose to every function name, field and
;     parameter instead of stacking under them; it has simply not been
;     ported back;
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

; Variables
;
; Upstream's only `@variable.builtin` is `self`; it has no `_G` pattern,
; so none is invented here. It sits above every other identifier capture
; so first-pattern-wins keeps `self` builtin-coloured.
((identifier) @variable.builtin
  (#eq? @variable.builtin "self"))

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

; Above the generic call pattern below: same-node captures resolve
; first-pattern-wins, so `print` has to be claimed as a builtin before
; the call pattern claims it as an ordinary function.
(function_call
  (identifier) @function.builtin
  (#any-of? @function.builtin
    ; built-in functions in Lua 5.1
    "assert" "collectgarbage" "dofile" "error" "getfenv" "getmetatable" "ipairs" "load" "loadfile"
    "loadstring" "module" "next" "pairs" "pcall" "print" "rawequal" "rawget" "rawset" "require"
    "select" "setfenv" "setmetatable" "tonumber" "tostring" "type" "unpack" "xpcall"))

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
