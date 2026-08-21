; Ruby highlights.scm — adapted from tree-sitter-ruby 0.23.1's own
; queries/highlights.scm (MIT, (c) 2016 Rob Rix).
;
; Three systematic changes, the same ones go/highlights.scm documents:
;
;   * the catch-all `(identifier) @variable` is dropped — span extraction
;     has no first-wins dedup, so it would stack a second span under every
;     method name, parameter and call site;
;   * every pattern guarded by `#match?`, `#eq?` or `#is-not?` is dropped,
;     because `spans_from_tree` does not evaluate predicates and a shipped
;     guard would paint unconditionally (the upstream `#is-not? local`
;     pattern would make every identifier a method call);
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
