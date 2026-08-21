; Identifier-occurrence query for A2 (`identifier_occurrences`), Bash.
;
; A shell variable is `(variable_name)` both where it is assigned and where
; it is expanded, so the catch-all `@reference` below also matches every
; assignment site; the assignment patterns then re-capture those same nodes
; as `@definition` and lib.rs folds the two by byte range — the same shape
; php/locals.scm uses for `$foo`.

(variable_assignment name: (variable_name) @definition)
(declaration_command (variable_assignment name: (variable_name) @definition))
(function_definition name: (word) @definition)

(variable_name) @reference
(command_name (word) @reference)
