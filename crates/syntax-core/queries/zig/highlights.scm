; Zig highlights.scm — adapted from tree-sitter-zig 1.1.2's own
; queries/highlights.scm (MIT, https://github.com/tree-sitter-grammars/tree-sitter-zig).
;
; Upstream carries five predicate-guarded patterns. Three are ported: the
; `#any-of?` on `@import`/`@cImport`, the `#eq?` on `_`, and the `//!`
; doc-comment pattern, which upstream guards with `#lua-match?` and which
; is re-authored here with `#match?` (see the Comments section).
;
; The other two are `#lua-match?` and stay out **by design**, not by
; omission:
;
;     ((identifier) @type     (#lua-match? @type "^[A-Z_][a-zA-Z0-9_]*"))
;     ((identifier) @constant (#lua-match? @constant "^[A-Z][A-Z_0-9]+$"))
;
; `#lua-match?` is a general predicate tree-sitter does not know, so
; `spans_from_tree` drops the whole pattern — pasting one in leaves it
; inert, not unguarded. They are also redundant: the naming-conventions
; block at the end of this file does exactly the same CamelCase ->
; `@type`, SCREAMING_CASE -> `@constant` job with `#match?`, which *is*
; evaluated. Do not "fix" them back in.
;
; One further difference from upstream: the catch-all `(identifier)
; @variable` is still absent — same-node captures resolve
; first-pattern-wins now, so a catch-all placed last would lose to every
; specific capture instead of stacking a span underneath it; it has simply
; not been ported back.

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

; Modules
;
; Above `(builtin_identifier) @function.builtin` on purpose: same-node
; captures resolve first-pattern-wins, so `@import` has to be claimed as
; an import keyword before the generic builtin pattern claims it.

(variable_declaration
  (identifier) @module
  (builtin_function
    (builtin_identifier) @keyword.import
    (#any-of? @keyword.import "@import" "@cImport")))

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

((identifier) @variable.builtin
  (#eq? @variable.builtin "_"))

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

; Doc comments first — same-node captures resolve first-pattern-wins, so
; this has to beat the plain `(comment)` capture below.
;
; Re-authored from upstream's `((comment) @comment.documentation
; (#lua-match? @comment.documentation "^//!"))`. `#lua-match?` is a
; general predicate, so the pattern would have been dropped whole; `^//!`
; is a trivially portable regex and `comment.documentation` is both in
; `SCOPES` and themed, so the pattern is worth converting rather than
; skipping.
((comment) @comment.documentation
  (#match? @comment.documentation "^//!"))

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
