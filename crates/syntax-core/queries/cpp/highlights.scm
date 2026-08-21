; C++ highlights.scm — the C ruleset plus C++-only syntax.
;
; Upstream tree-sitter-cpp's queries/highlights.scm opens with
; `; inherits: c` and then lists only the C++ additions. This crate's
; loader does not implement that directive (it compiles one file against
; one grammar, nothing else), so the C patterns are spelled out here
; directly — this file is `queries/c/highlights.scm` followed by the C++
; section. Keep the two halves in sync by hand when C changes.
;
; Adapted from tree-sitter-c and tree-sitter-cpp (MIT, Copyright (c) 2014
; Max Brunsfeld), minus the `#match?` and catch-all `(identifier)`
; patterns — this crate does not evaluate text predicates.

; --- inlined from queries/c/highlights.scm -------------------------------

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

; --- C++ only ------------------------------------------------------------

(raw_string_literal) @string
(auto) @type.builtin
(this) @variable.builtin

(call_expression
  function: (qualified_identifier name: (identifier) @function.call))
(template_function
  name: (identifier) @function.call)
(template_method
  name: (field_identifier) @function.method)
(function_declarator
  declarator: (qualified_identifier name: (identifier) @function))
(function_declarator
  declarator: (field_identifier) @function.method)

(namespace_identifier) @module

[
  "catch" "class" "co_await" "co_return" "co_yield" "concept" "constexpr"
  "consteval" "constinit" "delete" "explicit" "final" "friend" "mutable"
  "namespace" "new" "noexcept" "override" "private" "protected" "public"
  "requires" "template" "throw" "try" "typename" "using" "virtual"
] @keyword
