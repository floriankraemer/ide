; Make highlights.scm — adapted from tree-sitter-make 1.1.1's own
; queries/highlights.scm (MIT,
; https://github.com/tree-sitter-grammars/tree-sitter-make).
;
; Three systematic differences from upstream; go/highlights.scm
; documents the predicate rules they follow:
;
;   * every pattern guarded by a `#match?` predicate is still absent —
;     `#match?` is evaluated (see queries/go/highlights.scm), so
;     upstream's well-known-variable and standard-target lists would work
;     as written; nobody has ported them back yet;
;   * `(variable_assignment (word) @string)` is dropped: the `(word)` it
;     matches is the assignment's *name*, which the pattern below already
;     captures as `@constant`, so shipping both stacks two spans on it;
;   * Neovim-flavoured capture names are rewritten to the standard ones in
;     `syntax_core::SCOPES` (`@conditional`/`@repeat`/`@include` ->
;     `@keyword`, `@keyword.function` -> `@function.builtin`, `@exception`
;     -> `@keyword`), and `@error`/`@text.*` are dropped — they are
;     diagnostics, not syntax.

[
 "("
 ")"
 "{"
 "}"
] @punctuation.bracket

[
 ":"
 "&:"
 "::"
 "|"
 ";"
 "\""
 "'"
 ","
] @punctuation.delimiter

[
 "$"
 "$$"
] @punctuation.special

(automatic_variable
 [ "@" "%" "<" "?" "^" "+" "/" "*" "D" "F"] @punctuation.special)

[
 "="
 ":="
 "::="
 "?="
 "+="
 "!="
 "@"
 "-"
 "+"
] @operator

[
 (text)
 (string)
 (raw_text)
] @string

[
 "ifeq"
 "ifneq"
 "ifdef"
 "ifndef"
 "else"
 "endif"
 "if"
 "or"  ; boolean functions are conditional in make grammar
 "and"
 "foreach"
 "define"
 "endef"
 "vpath"
 "undefine"
 "export"
 "unexport"
 "override"
 "private"
 "include"
 "sinclude"
 "-include"
 "error"
 "warning"
 "info"
] @keyword

[
 "subst"
 "patsubst"
 "strip"
 "findstring"
 "filter"
 "filter-out"
 "sort"
 "word"
 "words"
 "wordlist"
 "firstword"
 "lastword"
 "dir"
 "notdir"
 "suffix"
 "basename"
 "addsuffix"
 "addprefix"
 "join"
 "wildcard"
 "realpath"
 "abspath"
 "call"
 "eval"
 "file"
 "value"
 "shell"
] @function.builtin

;; Variables
(variable_assignment
  name: (word) @constant)

(variable_reference
  (word) @constant)

[
 "VPATH"
 ".RECIPEPREFIX"
] @constant.builtin

(comment) @comment
