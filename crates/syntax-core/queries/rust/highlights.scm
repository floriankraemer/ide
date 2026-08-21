; Behavior-preserving port of the old hand-rolled `classify_rust` matcher
; (see git history) to a tree-sitter query. Capture names map onto
; `TokenKind` in lib.rs: @keyword, @string, @comment, @number, @function,
; @type. Anything not captured here yields no span, same as the old
; matcher's `_ => None` arm.

(line_comment) @comment
(block_comment) @comment

(string_literal) @string
(raw_string_literal) @string
(char_literal) @string

(integer_literal) @number
(float_literal) @number

(type_identifier) @type
(primitive_type) @type

(function_item name: (identifier) @function)
(call_expression function: (identifier) @function)

"fn" @keyword
"let" @keyword
"pub" @keyword
"struct" @keyword
"enum" @keyword
"impl" @keyword
"trait" @keyword
"use" @keyword
"mod" @keyword
"return" @keyword
"if" @keyword
"else" @keyword
"match" @keyword
"for" @keyword
"while" @keyword
"loop" @keyword
"break" @keyword
"continue" @keyword
"const" @keyword
"static" @keyword
"async" @keyword
"await" @keyword
"move" @keyword
"ref" @keyword
"as" @keyword
"where" @keyword
"unsafe" @keyword
"dyn" @keyword
"extern" @keyword
"in" @keyword
"true" @keyword
"false" @keyword
(self) @keyword
(super) @keyword
(crate) @keyword

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

; CamelCase is a constructor.
((identifier) @constructor
  (#match? @constructor "^[A-Z]"))
