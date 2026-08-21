; C highlights.scm — adapted from tree-sitter-c's own
; queries/highlights.scm (MIT, Copyright (c) 2014 Max Brunsfeld).
;
; Departures from upstream: the `#match?`-driven SCREAMING_CASE ->
; @constant pattern lives in the naming-conventions block at the end of
; this file, where first-pattern-wins keeps it from overriding anything
; more specific (text predicates are evaluated — see
; queries/go/highlights.scm); the catch-all `(identifier) @variable` is
; still absent, though placing it last would now cost nothing, because
; nobody has ported it back; and upstream's `@delimiter` is spelled
; `@punctuation.delimiter`, which is the name this crate's scope taxonomy
; knows.

(comment) @comment

(string_literal) @string
(system_lib_string) @string
(char_literal) @character
(escape_sequence) @string.escape

(number_literal) @number
(null) @constant.builtin
[
  (true)
  (false)
] @constant.builtin

(type_identifier) @type
(primitive_type) @type.builtin
(sized_type_specifier) @type.builtin

(field_identifier) @property
(statement_identifier) @label

(call_expression
  function: (identifier) @function.call)
(call_expression
  function: (field_expression field: (field_identifier) @function.call))
(function_declarator
  declarator: (identifier) @function)
(preproc_function_def
  name: (identifier) @function.macro)

[
  "break" "case" "const" "continue" "default" "do" "else" "enum" "extern"
  "for" "goto" "if" "inline" "return" "sizeof" "static" "struct" "switch"
  "typedef" "union" "volatile" "while"
] @keyword

[
  "#define" "#elif" "#else" "#endif" "#if" "#ifdef" "#ifndef" "#include"
] @keyword
(preproc_directive) @keyword

[
  "--" "-" "-=" "->" "=" "!=" "*" "&" "&&" "+" "++" "+=" "<" "==" ">" "||"
] @operator

[
  "."
  ";"
] @punctuation.delimiter

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
