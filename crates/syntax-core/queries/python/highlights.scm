; Python highlights.scm — adapted from tree-sitter-python's own
; queries/highlights.scm (MIT, Copyright (c) 2016 Max Brunsfeld).
;
; Departures from upstream:
;   * the `#match?`-driven naming-convention patterns live in the block at
;     the end of this file rather than up here, and CamelCase maps to
;     `@type` instead of upstream's `@constructor`. Text predicates are
;     evaluated (see queries/go/highlights.scm) and same-node captures
;     resolve first-pattern-wins, so sitting last they only paint what no
;     specific pattern claimed.
;   * the catch-all `(identifier) @variable` is still absent — placing it
;     last would cost nothing now, it has just not been ported back.
;
; Upstream's builtin-function `#match?` regex IS ported, above the generic
; `(call function: (identifier))` pattern rather than below it: same-node
; captures resolve first-pattern-wins here, the opposite of the convention
; upstream's own ordering assumes, so pasting it in at upstream's position
; would leave `len` painted as an ordinary call and do nothing.

(comment) @comment
(string) @string
(escape_sequence) @string.escape

[
  (none)
  (true)
  (false)
] @constant.builtin

[
  (integer)
  (float)
] @number

(decorator) @attribute

(function_definition
  name: (identifier) @function)
(class_definition
  name: (identifier) @type)

; Builtin functions, ported from upstream tree-sitter-python 0.25.0.
; Above the generic call pattern below, which would otherwise claim them.

((call
  function: (identifier) @function.builtin)
 (#match?
   @function.builtin
   "^(abs|all|any|ascii|bin|bool|breakpoint|bytearray|bytes|callable|chr|classmethod|compile|complex|delattr|dict|dir|divmod|enumerate|eval|exec|filter|float|format|frozenset|getattr|globals|hasattr|hash|help|hex|id|input|int|isinstance|issubclass|iter|len|list|locals|map|max|memoryview|min|next|object|oct|open|ord|pow|print|property|range|repr|reversed|round|set|setattr|slice|sorted|staticmethod|str|sum|super|tuple|type|vars|zip|__import__)$"))

(call
  function: (identifier) @function.call)
(call
  function: (attribute attribute: (identifier) @function.method))

(attribute attribute: (identifier) @property)
(type (identifier) @type)

(parameters (identifier) @variable.parameter)
(default_parameter name: (identifier) @variable.parameter)
(typed_parameter (identifier) @variable.parameter)

(interpolation
  "{" @punctuation.special
  "}" @punctuation.special)

[
  "-" "-=" "!=" "*" "**" "**=" "*=" "/" "//" "//=" "/=" "&" "&=" "%" "%="
  "^" "^=" "+" "->" "+=" "<" "<<" "<<=" "<=" "<>" "=" ":=" "==" ">" ">="
  ">>" ">>=" "|" "|=" "~" "@="
  "and" "in" "is" "not" "or" "is not" "not in"
] @operator

[
  "as" "assert" "async" "await" "break" "class" "continue" "def" "del"
  "elif" "else" "except" "exec" "finally" "for" "from" "global" "if"
  "import" "lambda" "nonlocal" "pass" "print" "raise" "return" "try"
  "while" "with" "yield" "match" "case"
] @keyword

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
