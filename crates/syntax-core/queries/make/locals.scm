; Identifier-occurrence query for A2 (`identifier_occurrences`), Make.
;
; A make variable is a `(word)` both where it is assigned and where it is
; expanded, so the assignment pattern captures `@definition` and the
; expansion pattern `@reference` — the same shape bash/locals.scm uses for
; shell variables. Targets are named definitions too, and a prerequisite is
; a reference to one.

(variable_assignment name: (word) @definition)
(targets (word) @definition)

(variable_reference (word) @reference)
(prerequisites (word) @reference)
