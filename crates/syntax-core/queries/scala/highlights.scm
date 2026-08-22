; Scala highlights.scm — adapted from tree-sitter-scala 0.26.2's own
; queries/highlights.scm (MIT, https://github.com/tree-sitter/tree-sitter-scala).
; Upstream carries ten `#match?`-guarded patterns. Six are ported below:
; the capitalised import/export paths, the capitalised call target, and the
; `this`/`super` identifiers. Four are deliberately left out — upstream's
; `(field_expression value: ...)`, its `(namespace_selectors ...)` and its
; trailing bare-identifier rule all produce `@type` on nodes that the
; naming-conventions block at the end of this file already reaches, and its
; `(stable_identifier ...)` type pattern is written out twice.
;
; Capture names follow this crate's SCOPES taxonomy rather than upstream's
; nvim-treesitter flavour: parameter -> variable.parameter, namespace ->
; module, method -> function.method, method.call -> function.call, float ->
; number.float, none -> constant.builtin, and conditional / repeat / include
; / exception / storageclass -> keyword.

; CREDITS @stumash (stuart.mashaal@gmail.com)

(field_expression field: (identifier) @property)

(type_identifier) @type

(class_definition
  name: (identifier) @type)

(enum_definition
  name: (identifier) @type)

(object_definition
  name: (identifier) @type)

(trait_definition
  name: (identifier) @type)

(full_enum_case
  name: (identifier) @type)

(simple_enum_case
  name: (identifier) @type)

;; variables

(class_parameter
  name: (identifier) @variable.parameter)

(self_type (identifier) @variable.parameter)

(interpolation (identifier) @constant.builtin)
(interpolation (block) @constant.builtin)

;; types

(type_definition
  name: (type_identifier) @type.definition)

;; val/var definitions/declarations

(val_definition
  pattern: (identifier) @variable)

(var_definition
  pattern: (identifier) @variable)

(val_declaration
  name: (identifier) @variable)

(var_declaration
  name: (identifier) @variable)

; imports/exports
;
; A capitalised path segment names a type, not a package. These sit above the
; `@module` patterns because captures on one node resolve first-pattern-wins.

((import_declaration
  path: (identifier) @type) (#match? @type "^[A-Z]"))
((stable_identifier (identifier) @type) (#match? @type "^[A-Z]"))

((export_declaration
  path: (identifier) @type) (#match? @type "^[A-Z]"))

(import_declaration
  path: (identifier) @module)
((stable_identifier (identifier) @module))

(export_declaration
  path: (identifier) @module)
((stable_identifier (identifier) @module))

; method invocation

; A capitalised callee is a constructor application; it has to precede the
; `@function.call` patterns below to win the node.
((call_expression
  function: (identifier) @constructor)
 (#match? @constructor "^[A-Z]"))

(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (operator_identifier) @function.call)

(call_expression
  function: (field_expression
    field: (identifier) @function.call))

(generic_function
  function: (identifier) @function.call)

(interpolated_string_expression
  interpolator: (identifier) @function.call)

; function definitions

(function_definition
  name: (identifier) @function)

(parameter
  name: (identifier) @variable.parameter)

(binding
  name: (identifier) @variable.parameter)

; method definition

(function_declaration
      name: (identifier) @function.method)

(function_definition
      name: (identifier) @function.method)

; expressions

(infix_expression operator: (identifier) @operator)
(infix_expression operator: (operator_identifier) @operator)
; An operator taking a colon argument parses as a postfix expression call.
(call_expression
  function: (postfix_expression (identifier) @operator .)
  arguments: (colon_argument))
(infix_type operator: (operator_identifier) @operator)
(infix_type operator: (operator_identifier) @operator)

; literals

(boolean_literal) @boolean
(integer_literal) @number
(floating_point_literal) @number.float

[
  (string)
  (character_literal)
  (interpolated_string_expression)
] @string

(interpolation "$" @punctuation.special)

;; keywords

(opaque_modifier) @type.qualifier
(infix_modifier) @keyword
(transparent_modifier) @type.qualifier
(open_modifier) @type.qualifier

[
  "case"
  "class"
  "enum"
  "extends"
  "derives"
  "finally"
  "forSome"
;; `macro` not implemented yet
  "object"
  "override"
  "package"
  "trait"
  "type"
  "val"
  "var"
  "with"
  "given"
  "using"
  "implicit"
  "with"
] @keyword

; `end` is scanner-lexed, so the marker node is the only thing to match.
(end_marker) @keyword

; `extension` is a soft keyword. Highlight it only where it starts an
; extension definition, not when used as a plain identifier.
(extension_definition "extension" @keyword)

[
  "abstract"
  "final"
  "lazy"
  "sealed"
  "private"
  "protected"
] @type.qualifier

(inline_modifier) @keyword

(null_literal) @constant.builtin

(wildcard) @variable.parameter

(annotation) @attribute

;; special keywords

"new" @keyword.operator

[
  "else"
  "if"
  "match"
  "then"
] @keyword

[
 "("
 ")"
 "["
 "]"
 "{"
 "}"
]  @punctuation.bracket

[
 "."
 ","
] @punctuation.delimiter

[
  "do"
  "for"
  "while"
  "yield"
] @keyword

"def" @keyword.function

[
 "=>"
 "<-"
 "@"
] @operator

["import" "export"] @keyword

[
  "try"
  "catch"
  "throw"
] @keyword

"return" @keyword.return

(comment) @spell @comment
(block_comment) @spell @comment

;; `case` is a conditional keyword in case_block

(case_block
  (case_clause ("case") @keyword))
(indented_cases
  (case_clause ("case") @keyword))

(operator_identifier) @operator

;; Scala CLI using directives
(using_directive_key) @variable.parameter
(using_directive_value) @string

;; XML literals
(xml_name) @tag
(xml_attribute key: (xml_name) @tag.attribute)
(xml_string) @string
(xml_text) @spell
(xml_comment) @spell @comment
(xml_cdata) @string
(xml_processing_instruction) @keyword.directive

; `this` and `super` are lexed as plain identifiers, so they need the text
; predicate to be told apart from any other name.
((identifier) @variable.builtin
  (#match? @variable.builtin "^this$"))

((identifier) @function.builtin
  (#match? @function.builtin "^super$"))

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
