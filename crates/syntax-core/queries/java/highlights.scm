; Java highlights.scm — same capture-name convention as rust/json
; (@keyword, @string, @comment, @number, @function, @type). Trimmed from
; tree-sitter-java's own shipped queries/highlights.scm down to the six
; TokenKind captures this crate understands.
;
; Class/method/constructor *names* are the grammar's plain `identifier`
; node (not `type_identifier`, which is reserved for type references like
; field/parameter types) — mirrored below the same way the upstream query
; does, including capturing the constructor name as @type since it's
; syntactically identical to the class name it names.

(line_comment) @comment
(block_comment) @comment

(character_literal) @string
(string_literal) @string

[
  (hex_integer_literal)
  (decimal_integer_literal)
  (octal_integer_literal)
  (decimal_floating_point_literal)
  (hex_floating_point_literal)
] @number

(type_identifier) @type
(class_declaration name: (identifier) @type)
(interface_declaration name: (identifier) @type)
(enum_declaration name: (identifier) @type)
(constructor_declaration name: (identifier) @type)

[
  (boolean_type)
  (integral_type)
  (floating_point_type)
  (void_type)
] @type

(method_declaration name: (identifier) @function)
(method_invocation name: (identifier) @function)

[
  "class"
  "interface"
  "enum"
  "extends"
  "implements"
  "return"
  "new"
  "if"
  "else"
  "for"
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
  "throws"
  "import"
  "package"
  "static"
  "public"
  "private"
  "protected"
  "final"
  "abstract"
  "synchronized"
  "instanceof"
  "assert"
  "yield"
] @keyword

[
  (this)
  (super)
] @keyword
