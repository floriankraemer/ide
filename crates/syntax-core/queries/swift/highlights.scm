; Swift highlights.scm — adapted from tree-sitter-swift 0.7.3's own
; queries/highlights.scm (MIT, https://github.com/alex-pinkus/tree-sitter-swift).
;
; Upstream carries four `#match?`-guarded patterns. Three are ported (the
; `@comment.documentation` trio below, which has to sit above the plain
; `(comment)` capture because same-node captures resolve
; first-pattern-wins). The fourth,
;
;     ((navigation_expression (simple_identifier) @type) (#match? @type "^[A-Z]"))
;
; is deliberately left out as redundant, not unported: the
; naming-conventions block at the end of this file already paints any
; capitalised `simple_identifier` `@type`, and `SomeType` in
; `SomeType.method()` is claimed by no other pattern, so restoring it
; would change nothing. Do not "fix" it back in.
;
; The catch-all `(simple_identifier) @variable` is restored, at the very
; end of the file: upstream relies on last-match-wins and puts it first,
; this crate resolves first-pattern-wins, so the equivalent position is
; last — it claims only identifiers no specific pattern took.

[
  "."
  ";"
  ":"
  ","
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

; Identifiers
(type_identifier) @type

[
  (self_expression)
  (super_expression)
] @variable.builtin

; Declarations
[
  "func"
  "deinit"
] @keyword.function

[
  (visibility_modifier)
  (member_modifier)
  (function_modifier)
  (property_modifier)
  (parameter_modifier)
  (inheritance_modifier)
  (mutation_modifier)
] @keyword.modifier

(function_declaration
  (simple_identifier) @function.method)

(protocol_function_declaration
  name: (simple_identifier) @function.method)

(init_declaration
  "init" @constructor)

(parameter
  external_name: (simple_identifier) @variable.parameter)

(parameter
  name: (simple_identifier) @variable.parameter)

(type_parameter
  (type_identifier) @variable.parameter)

(inheritance_constraint
  (identifier
    (simple_identifier) @variable.parameter))

(equality_constraint
  (identifier
    (simple_identifier) @variable.parameter))

[
  "protocol"
  "extension"
  "indirect"
  "nonisolated"
  "override"
  "convenience"
  "required"
  "some"
  "any"
  "weak"
  "unowned"
  "didSet"
  "willSet"
  "subscript"
  "let"
  "var"
  (throws)
  (where_keyword)
  (getter_specifier)
  (setter_specifier)
  (modify_specifier)
  (else)
  (as_operator)
] @keyword

[
  "enum"
  "struct"
  "class"
  "typealias"
] @keyword.type

[
  "async"
  "await"
] @keyword.coroutine

(shebang_line) @keyword.directive

(class_body
  (property_declaration
    (pattern
      (simple_identifier) @variable.member)))

(protocol_property_declaration
  (pattern
    (simple_identifier) @variable.member))

(navigation_expression
  (navigation_suffix
    (simple_identifier) @variable.member))

(value_argument
  name: (value_argument_label
    (simple_identifier) @variable.member))

(import_declaration
  "import" @keyword.import)

(enum_entry
  "case" @keyword)

(modifiers
  (attribute
    "@" @attribute
    (user_type
      (type_identifier) @attribute)))

; Function calls
(call_expression
  (simple_identifier) @function.call) ; foo()

(call_expression
  ; foo.bar.baz(): highlight the baz()
  (navigation_expression
    (navigation_suffix
      (simple_identifier) @function.call)))

(call_expression
  (prefix_expression
    (simple_identifier) @function.call)) ; .foo()

(directive) @keyword.directive

; See https://docs.swift.org/swift-book/documentation/the-swift-programming-language/lexicalstructure/#Keywords-and-Punctuation
[
  (diagnostic)
  (availability_condition)
  (playground_literal)
  (key_path_string_expression)
  (selector_expression)
  (external_macro_definition)
] @function.macro

(special_literal) @constant.macro

; Statements
(for_statement
  "for" @keyword.repeat)

(for_statement
  "in" @keyword.repeat)

[
  "while"
  "repeat"
  "continue"
  "break"
] @keyword.repeat

(guard_statement
  "guard" @keyword.conditional)

(if_statement
  "if" @keyword.conditional)

(switch_statement
  "switch" @keyword.conditional)

(switch_entry
  "case" @keyword)

(switch_entry
  "fallthrough" @keyword)

(switch_entry
  (default_keyword) @keyword)

"return" @keyword.return

(ternary_expression
  [
    "?"
    ":"
  ] @keyword.conditional.ternary)

[
  (try_operator)
  "do"
  (throw_keyword)
  (catch_keyword)
] @keyword.exception

(statement_label) @label

; Comments
; Doc comments first — same-node captures resolve first-pattern-wins, so
; these have to beat the plain `(comment)` capture below.
((comment) @comment.documentation
  (#match? @comment.documentation "^///[^/]"))

((comment) @comment.documentation
  (#match? @comment.documentation "^///$"))

; `(?s)` added to upstream's regex: `#match?` compiles with the `regex`
; crate, where `.` does not cross a newline, and a multiline doc comment
; always does.
((multiline_comment) @comment.documentation
  (#match? @comment.documentation "(?s)^/[*][*][^*].*[*]/$"))

[
  (comment)
  (multiline_comment)
] @comment @spell

; String literals
(line_str_text) @string

(str_escaped_char) @string.escape

(multi_line_str_text) @string

(raw_str_part) @string

(raw_str_end_part) @string

(line_string_literal
  [
    "\\("
    ")"
  ] @punctuation.special)

(multi_line_string_literal
  [
    "\\("
    ")"
  ] @punctuation.special)

(raw_str_interpolation
  [
    (raw_str_interpolation_start)
    ")"
  ] @punctuation.special)

[
  "\""
  "\"\"\""
] @string

; Lambda literals
(lambda_literal
  "in" @keyword.operator)

; Basic literals
[
  (integer_literal)
  (hex_literal)
  (oct_literal)
  (bin_literal)
] @number

(real_literal) @number.float

(boolean_literal) @boolean

"nil" @constant.builtin

(wildcard_pattern) @character.special

; Regex literals
(regex_literal) @string.regexp

; Operators
(custom_operator) @operator

[
  "+"
  "-"
  "*"
  "/"
  "%"
  "="
  "+="
  "-="
  "*="
  "/="
  "<"
  ">"
  "<<"
  ">>"
  "<="
  ">="
  "++"
  "--"
  "^"
  "&"
  "&&"
  "|"
  "||"
  "~"
  "%="
  "!="
  "!=="
  "=="
  "==="
  "?"
  "??"
  "->"
  "..<"
  "..."
  (bang)
] @operator

(type_arguments
  [
    "<"
    ">"
  ] @punctuation.bracket)

; --- Naming conventions -----------------------------------------------
;
; Guarded by `#match?` text predicates, which `QueryCursor::matches` does
; evaluate (see `spans_from_tree`). They sit last on purpose: captures on
; the same node resolve first-pattern-wins, so every specific pattern
; above still beats these catch-alls.

; SCREAMING_CASE is a constant. Two characters minimum, so a bare
; `T` stays a type rather than becoming a constant.
((simple_identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))

; CamelCase is a type.
((simple_identifier) @type
  (#match? @type "^[A-Z]"))

; --- Catch-all --------------------------------------------------------
;
; Last on purpose: every specific capture above wins the node, so this
; only paints identifiers nothing else claimed.
(simple_identifier) @variable
