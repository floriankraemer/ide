; Ruby highlights.scm — adapted from tree-sitter-ruby 0.23.1's own
; queries/highlights.scm (MIT, (c) 2016 Rob Rix).
;
; Upstream carries five predicate-guarded patterns. Four are ported below
; (`#match?` on `private|protected|public`, `#eq?` on `require`, `#match?`
; on `__FILE__`/`__LINE__`/`__ENCODING__`, `#match?` on SCREAMING_CASE
; constants); each sits above the broader pattern it has to beat, because
; same-node captures resolve first-pattern-wins here.
;
; The fifth stays out **by design**, not by omission — do not "fix" it in:
;
;     ((identifier) @function.method (#is-not? local))
;
; `#is-not? local` is a property predicate tree-sitter parses but never
; evaluates, so `spans_from_tree` drops the whole pattern rather than let
; it run unguarded. That failing-closed rule is documented in
; `syntax_core`'s `pattern_is_guarded_by_an_unevaluated_predicate`, which
; cites this exact pattern as its motivating case: unguarded it paints
; *every* identifier in the file as a method call. Pasting it in as-is
; buys nothing, and re-adding it without the guard would be actively
; wrong.
;
; Two further differences from upstream:
;
;   * `@function.method.builtin` is rewritten to `@function.builtin`, the
;     standard name in `syntax_core::SCOPES`;
;   * the catch-all `(identifier) @variable` is restored, but at the very
;     end of the file: upstream relies on last-match-wins and puts it
;     first, this crate resolves first-pattern-wins, so the equivalent
;     position is last — it claims only identifiers no specific pattern
;     took.

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

; `private`/`protected`/`public` parse as ordinary calls, so this has to
; sit above the `(call method: ...)` pattern below.
((identifier) @keyword
  (#match? @keyword "^(private|protected|public)$"))

; SCREAMING_CASE constants before the generic `(constant) @constructor`.
((constant) @constant
  (#match? @constant "^[A-Z\\d_]+$"))

(constant) @constructor

; Function calls

"defined?" @function.builtin

((identifier) @function.builtin
  (#eq? @function.builtin "require"))

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

((identifier) @constant.builtin
  (#match? @constant.builtin "^__(FILE|LINE|ENCODING)__$"))

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

; --- Catch-all --------------------------------------------------------
;
; Last on purpose: every specific capture above wins the node, so this
; only paints identifiers nothing else claimed.
(identifier) @variable
