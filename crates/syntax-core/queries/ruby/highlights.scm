; Ruby highlights.scm — adapted from tree-sitter-ruby 0.23.1's own
; queries/highlights.scm (MIT, (c) 2016 Rob Rix).
;
; Four systematic differences from upstream; go/highlights.scm documents
; the predicate rules they follow:
;
;   * the catch-all `(identifier) @variable` is still absent — not for
;     want of dedup (span extraction resolves same-node captures
;     first-pattern-wins now, so a catch-all placed last would lose to
;     every method name, parameter and call site rather than stack a
;     second span under it), but because nobody has ported it back;
;   * the patterns guarded by `#match?` or `#eq?` are likewise still
;     absent, though those predicates *are* evaluated (see
;     `spans_from_tree`) and could be ported back as written;
;   * upstream's `(identifier) @method (#is-not? local)` stays out for a
;     harder reason: `#is-not? local` is a property predicate tree-sitter
;     does not evaluate, so `spans_from_tree` drops the whole pattern
;     rather than let it paint every identifier — pasting it in would buy
;     nothing;
;   * `@function.method.builtin` is rewritten to `@function.builtin`, the
;     standard name in `syntax_core::SCOPES`.

[
  "alias"
  "and"
  "begin"
  "break"
  "case"
  "class"
  "def"
  "do"
  "else"
  "elsif"
  "end"
  "ensure"
  "for"
  "if"
  "in"
  "module"
  "next"
  "or"
  "rescue"
  "retry"
  "return"
  "then"
  "unless"
  "until"
  "when"
  "while"
  "yield"
] @keyword

(constant) @constructor

; Function calls

"defined?" @function.builtin

(call
  method: [(identifier) (constant)] @function.method)

; Function definitions

(alias (identifier) @function.method)
(setter (identifier) @function.method)
(method name: [(identifier) (constant)] @function.method)
(singleton_method name: [(identifier) (constant)] @function.method)

; Identifiers

[
  (class_variable)
  (instance_variable)
] @property

(file) @constant.builtin
(line) @constant.builtin
(encoding) @constant.builtin

(hash_splat_nil
  "**" @operator) @constant.builtin

[
  (self)
  (super)
] @variable.builtin

(block_parameter (identifier) @variable.parameter)
(block_parameters (identifier) @variable.parameter)
(destructured_parameter (identifier) @variable.parameter)
(hash_splat_parameter (identifier) @variable.parameter)
(lambda_parameters (identifier) @variable.parameter)
(method_parameters (identifier) @variable.parameter)
(splat_parameter (identifier) @variable.parameter)

(keyword_parameter name: (identifier) @variable.parameter)
(optional_parameter name: (identifier) @variable.parameter)

; Literals

[
  (string)
  (bare_string)
  (subshell)
  (heredoc_body)
  (heredoc_beginning)
] @string

[
  (simple_symbol)
  (delimited_symbol)
  (hash_key_symbol)
  (bare_symbol)
] @string.special.symbol

(regex) @string.regexp
(escape_sequence) @escape

[
  (integer)
  (float)
] @number

[
  (nil)
  (true)
  (false)
] @constant.builtin

(interpolation
  "#{" @punctuation.special
  "}" @punctuation.special) @embedded

(comment) @comment

; Operators

[
"="
"=>"
"->"
] @operator

[
  ","
  ";"
  "."
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
  "%w("
  "%i("
] @punctuation.bracket
