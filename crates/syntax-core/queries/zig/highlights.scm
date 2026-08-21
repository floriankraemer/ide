; Zig highlights.scm — adapted from tree-sitter-zig 1.1.2's own
; queries/highlights.scm (MIT, https://github.com/tree-sitter-grammars/tree-sitter-zig).
; Two classes of upstream pattern are still absent:
;
;   * five guarded by `#lua-match?`/`#eq?` predicates. The `#eq?` ones are
;     evaluated (see queries/go/highlights.scm) and have just not been
;     ported back; `#lua-match?` is a general predicate tree-sitter does
;     not know, so `spans_from_tree` drops the whole pattern — pasting one
;     of those in would leave it inert rather than unguarded;
;   * the catch-all `(identifier) @variable` — same-node captures resolve
;     first-pattern-wins now, so a catch-all placed last would lose to
;     every specific capture instead of stacking a span underneath it; it
;     has simply not been ported back.

; Variables

; Parameters

(parameter
  name: (identifier) @variable.parameter)

; Types

(parameter
  type: (identifier) @type)

(variable_declaration
  (identifier) @type
  "="
  [
    (struct_declaration)
    (enum_declaration)
    (union_declaration)
    (opaque_declaration)
  ])

[
  (builtin_type)
  "anyframe"
] @type.builtin

; Constants

[
  "null"
  "unreachable"
  "undefined"
] @constant.builtin

(field_expression
  .
  member: (identifier) @constant)

(enum_declaration
  (container_field
    type: (identifier) @constant))

; Labels

(block_label (identifier) @label)

(break_label (identifier) @label)

; Fields

(field_initializer
  .
  (identifier) @variable.member)

(field_expression
  (_)
  member: (identifier) @variable.member)

(container_field
  name: (identifier) @variable.member)

(initializer_list
  (assignment_expression
      left: (field_expression
              .
              member: (identifier) @variable.member)))

; Functions

(builtin_identifier) @function.builtin

(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (field_expression
    member: (identifier) @function.call))

(function_declaration
  name: (identifier) @function)

; Modules

; Builtins

[
  "c"
  "..."
] @variable.builtin

(calling_convention
  (identifier) @variable.builtin)

; Keywords

[
  "asm"
  "defer"
  "errdefer"
  "test"
  "error"
  "const"
  "var"
] @keyword

[
  "struct"
  "union"
  "enum"
  "opaque"
] @keyword.type

[
  "async"
  "await"
  "suspend"
  "nosuspend"
  "resume"
] @keyword.coroutine

"fn" @keyword.function

[
  "and"
  "or"
  "orelse"
] @keyword.operator

"return" @keyword.return

[
  "if"
  "else"
  "switch"
] @keyword.conditional

[
  "for"
  "while"
  "break"
  "continue"
] @keyword.repeat

[
  "usingnamespace"
  "export"
] @keyword.import

[
  "try"
  "catch"
] @keyword.exception

[
  "volatile"
  "allowzero"
  "noalias"
  "addrspace"
  "align"
  "callconv"
  "linksection"
  "pub"
  "inline"
  "noinline"
  "extern"
  "comptime"
  "packed"
  "threadlocal"
] @keyword.modifier

; Operator

[
  "="
  "*="
  "*%="
  "*|="
  "/="
  "%="
  "+="
  "+%="
  "+|="
  "-="
  "-%="
  "-|="
  "<<="
  "<<|="
  ">>="
  "&="
  "^="
  "|="
  "!"
  "~"
  "-"
  "-%"
  "&"
  "=="
  "!="
  ">"
  ">="
  "<="
  "<"
  "&"
  "^"
  "|"
  "<<"
  ">>"
  "<<|"
  "+"
  "++"
  "+%"
  "-%"
  "+|"
  "-|"
  "*"
  "/"
  "%"
  "**"
  "*%"
  "*|"
  "||"
  ".*"
  ".?"
  "?"
  ".."
] @operator

; Literals

(character) @character

(integer) @number

(float) @number.float

(boolean) @boolean

; Upstream writes this as `(... ) @string (#set! "priority" 95)`; the
; `#set!` is a highlighter directive, not a predicate, and simply has no
; meaning here, so only the directive is dropped.
[
  (string)
  (multiline_string)
] @string

(escape_sequence) @string.escape

; Punctuation

[
  "["
  "]"
  "("
  ")"
  "{"
  "}"
] @punctuation.bracket

[
  ";"
  "."
  ","
  ":"
  "=>"
  "->"
] @punctuation.delimiter

(payload "|" @punctuation.bracket)

; Comments

(comment) @comment @spell

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
