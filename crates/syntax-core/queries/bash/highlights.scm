; Bash highlights.scm — adapted from tree-sitter-bash 0.25.1's own
; `queries/highlights.scm` (MIT, https://github.com/tree-sitter/tree-sitter-bash).
;
; Kept verbatim except for one omission: upstream captures
; `command_substitution`/`process_substitution`/`expansion` as `@embedded`,
; which paints one span over the whole `$(...)` and swallows the string and
; keyword spans nested inside it. This crate has no injection layer yet, so
; the nested spans are the more useful colouring — see the plan doc's
; injection task.

[
  (string)
  (raw_string)
  (heredoc_body)
  (heredoc_start)
] @string

(command_name) @function

(variable_name) @property

[
  "case"
  "do"
  "done"
  "elif"
  "else"
  "esac"
  "export"
  "fi"
  "for"
  "function"
  "if"
  "in"
  "select"
  "then"
  "unset"
  "until"
  "while"
] @keyword

(comment) @comment

(function_definition name: (word) @function)

(file_descriptor) @number

[
  "$"
  "&&"
  ">"
  ">>"
  "<"
  "|"
] @operator

(
  (command (_) @constant)
  (#match? @constant "^-")
)
