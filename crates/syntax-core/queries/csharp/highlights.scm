; C# highlights.scm — same capture-name convention as rust/json
; (@keyword, @string, @comment, @number, @function, @type; see lib.rs's
; token_kind_for_capture). Trimmed from tree-sitter-c-sharp's own shipped
; queries/highlights.scm down to the six TokenKind captures this crate
; understands (no @variable/@property/@operator/@punctuation — those have
; no TokenKind and are silently dropped).

(comment) @comment

(string_literal) @string
(raw_string_literal) @string
(verbatim_string_literal) @string
(character_literal) @string
(interpolated_string_expression) @string

(integer_literal) @number
(real_literal) @number

(predefined_type) @type
(class_declaration name: (identifier) @type)
(interface_declaration name: (identifier) @type)
(struct_declaration (identifier) @type)
(enum_declaration name: (identifier) @type)
(record_declaration (identifier) @type)
(parameter type: (identifier) @type)
(variable_declaration type: (identifier) @type)
(object_creation_expression type: (identifier) @type)

(method_declaration name: (identifier) @function)
(constructor_declaration name: (identifier) @function)
(local_function_statement name: (identifier) @function)
(invocation_expression (member_access_expression name: (identifier) @function))

(modifier) @keyword

[
  "class"
  "namespace"
  "using"
  "interface"
  "struct"
  "enum"
  "record"
  "return"
  "new"
  "if"
  "else"
  "for"
  "foreach"
  "while"
  "do"
  "switch"
  "case"
  "default"
  "break"
  "continue"
  "try"
  "catch"
  "finally"
  "throw"
  "as"
  "is"
  "in"
  "out"
  "ref"
  "params"
  "base"
  "this"
  "typeof"
  "await"
  "yield"
  "get"
  "set"
  "where"
] @keyword
