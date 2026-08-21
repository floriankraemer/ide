; Python highlights.scm — adapted from tree-sitter-python's own
; queries/highlights.scm (MIT, Copyright (c) 2016 Max Brunsfeld).
;
; Two deliberate departures from upstream:
;   * the `#match?`-driven naming-convention patterns (SCREAMING_CASE ->
;     @constant, CamelCase -> @constructor) are dropped — this crate does
;     not evaluate text predicates, so they would paint every identifier.
;   * the catch-all `(identifier) @variable` is dropped for the same
;     reason: it would emit a span under every more specific capture.

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
