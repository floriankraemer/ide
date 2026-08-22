; Make highlights.scm — adapted from tree-sitter-make 1.1.1's own
; queries/highlights.scm (MIT,
; https://github.com/tree-sitter-grammars/tree-sitter-make).
;
; Upstream carries six `#match?`-guarded patterns, not the two the old
; header implied. All six are ported below (the well-known-variable list
; is written twice upstream, once for `variable_assignment` and once for
; `variable_reference`, and the standard-target list is duplicated
; verbatim — that exact duplicate is the one pattern dropped).
; Two systematic differences from upstream remain; go/highlights.scm
; documents the predicate rules they follow:
;
;   * `(variable_assignment (word) @string)` is dropped: the `(word)` it
;     matches is the assignment's *name*, which the pattern below already
;     captures as `@constant`, so shipping both stacks two spans on it;
;   * Neovim-flavoured capture names are rewritten to the standard ones in
;     `syntax_core::SCOPES` (`@conditional`/`@repeat`/`@include` ->
;     `@keyword`, `@keyword.function` -> `@function.builtin`, `@exception`
;     -> `@keyword`, `@string.regex` -> `@string.regexp`,
;     `@constant.macro` -> `@constant.builtin` for the target lists —
;     `constant.macro` is not in `SCOPES` and would only fall back to the
;     plain `@constant` the unguarded pattern already gives them, so that
;     port would have been invisible). `@error` and `@text.danger` /
;     `@text.warning` / `@text.note` are dropped rather than rewritten —
;     they are diagnostics, not syntax, and neither name is in `SCOPES`.
;
; Upstream's helper `@clean` capture (`@clean @string.regex`, `@clean
; @constant.builtin`) exists only to give the predicate something to
; reference; `clean` is not a scope, so the second capture name is used
; directly instead.

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
;;
;; Order matters: the well-known-variable and standard-target lists have
;; to sit above the unguarded `(word) @constant` captures, because
;; same-node captures resolve first-pattern-wins.

;; Variables Used by Implicit Rules
(variable_assignment
  name: (word) @constant.builtin
  (#match? @constant.builtin "^(AR|AS|CC|CXX|CPP|FC|M2C|PC|CO|GET|LEX|YACC|LINT|MAKEINFO|TEX|TEXI2DVI|WEAVE|CWEAVE|TANGLE|CTANGLE|RM|ARFLAGS|ASFLAGS|CFLAGS|CXXFLAGS|COFLAGS|CPPFLAGS|FFLAGS|GFLAGS|LDFLAGS|LDLIBS|LFLAGS|YFLAGS|PFLAGS|RFLAGS|LINTFLAGS|PRE_INSTALL|POST_INSTALL|NORMAL_INSTALL|PRE_UNINSTALL|POST_UNINSTALL|NORMAL_UNINSTALL|MAKEFILE_LIST|MAKE_RESTARTS|MAKE_TERMOUT|MAKE_TERMERR|\.DEFAULT_GOAL|\.RECIPEPREFIX|\.EXTRA_PREREQS)$"))

(variable_reference
  (word) @constant.builtin
  (#match? @constant.builtin "^(AR|AS|CC|CXX|CPP|FC|M2C|PC|CO|GET|LEX|YACC|LINT|MAKEINFO|TEX|TEXI2DVI|WEAVE|CWEAVE|TANGLE|CTANGLE|RM|ARFLAGS|ASFLAGS|CFLAGS|CXXFLAGS|COFLAGS|CPPFLAGS|FFLAGS|GFLAGS|LDFLAGS|LDLIBS|LFLAGS|YFLAGS|PFLAGS|RFLAGS|LINTFLAGS|PRE_INSTALL|POST_INSTALL|NORMAL_INSTALL|PRE_UNINSTALL|POST_UNINSTALL|NORMAL_UNINSTALL|MAKEFILE_LIST|MAKE_RESTARTS|MAKE_TERMOUT|MAKE_TERMERR|\.DEFAULT_GOAL|\.RECIPEPREFIX|\.EXTRA_PREREQS|\.VARIABLES|\.FEATURES|\.INCLUDE_DIRS|\.LOADED)$"))

;; Standard targets
(targets
  (word) @constant.builtin
  (#match? @constant.builtin "^(all|install|install-html|install-dvi|install-pdf|install-ps|uninstall|install-strip|clean|distclean|mostlyclean|maintainer-clean|TAGS|info|dvi|html|pdf|ps|dist|check|installcheck|installdirs)$"))

;; Builtin targets
(targets
  (word) @constant.builtin
  (#match? @constant.builtin "^\.(PHONY|SUFFIXES|DEFAULT|PRECIOUS|INTERMEDIATE|SECONDARY|SECONDEXPANSION|DELETE_ON_ERROR|IGNORE|LOW_RESOLUTION_TIME|SILENT|EXPORT_ALL_VARIABLES|NOTPARALLEL|ONESHELL|POSIX)$"))

;; A word carrying a make wildcard (`%.o`, `*.c`, `?`) is a pattern.
((word) @string.regexp
  (#match? @string.regexp "[%*?]"))

(variable_assignment
  name: (word) @constant)

(variable_reference
  (word) @constant)

[
 "VPATH"
 ".RECIPEPREFIX"
] @constant.builtin

(comment) @comment
