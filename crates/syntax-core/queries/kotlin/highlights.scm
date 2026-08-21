; Kotlin highlights.scm — hand-written against tree-sitter-kotlin-ng 1.1.0,
; which (unlike the other grammars in this catalog) ships no `queries/`
; directory of its own, so there is nothing upstream to adapt.
;
; Written to the same two rules the adapted files follow: no `#match?`
; predicates, because this crate's highlighter does not evaluate them, and
; no catch-all `(identifier) @variable`, because span extraction has no
; first-wins dedup and a catch-all would stack a span under every specific
; capture.

; Comments

[
  (line_comment)
  (block_comment)
] @comment

; Literals

(string_literal) @string
(character_literal) @character
(escape_sequence) @string.escape
(number_literal) @number

; Definitions

(class_declaration
  name: (identifier) @type)

(object_declaration
  name: (identifier) @type)

(type_alias
  type: (identifier) @type.definition)

(function_declaration
  name: (identifier) @function)

(call_expression
  (identifier) @function.call)

(call_expression
  (navigation_expression
    (identifier) @function.call
    .))

(parameter
  (identifier) @variable.parameter)

(class_parameter
  (identifier) @variable.parameter)

(user_type
  (identifier) @type)

(annotation) @attribute
(label) @label

; Keywords

[
  "abstract"
  "actual"
  "annotation"
  "as"
  "as?"
  "by"
  "catch"
  "class"
  "companion"
  "const"
  "constructor"
  "crossinline"
  "data"
  "delegate"
  "do"
  "dynamic"
  "else"
  "enum"
  "expect"
  "external"
  "field"
  "file"
  "final"
  "finally"
  "for"
  "fun"
  "get"
  "if"
  "import"
  "in"
  "infix"
  "init"
  "inline"
  "inner"
  "interface"
  "internal"
  "is"
  "lateinit"
  "noinline"
  "object"
  "open"
  "operator"
  "out"
  "override"
  "package"
  "param"
  "private"
  "property"
  "protected"
  "public"
  "receiver"
  "return"
  "return@"
  "sealed"
  "set"
  "setparam"
  "suspend"
  "tailrec"
  "throw"
  "try"
  "typealias"
  "val"
  "value"
  "var"
  "vararg"
  "when"
  "where"
  "while"
] @keyword

[
  "super"
  "super@"
  "this"
  "this@"
] @variable.builtin

; Operators and punctuation

[
  "!"
  "!!"
  "!="
  "!=="
  "!in"
  "!is"
  "%"
  "%="
  "&&"
  "*"
  "*="
  "+"
  "++"
  "+="
  "-"
  "--"
  "-="
  "->"
  ".."
  "..<"
  "/"
  "/="
  "<"
  "<="
  "="
  "=="
  "==="
  ">"
  ">="
  "?."
  "?:"
  "||"
] @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  "."
  ","
  ":"
  "::"
  ";"
] @punctuation.delimiter

[
  "$"
  "${"
  "@"
] @punctuation.special
