; Identifier-occurrence query for A2 (`identifier_occurrences`), Haskell.
; Same convention as rust/locals.scm: `@definition` for names in a
; declaration position, catch-all references for every variable and name.

(function name: (variable) @definition)
(bind name: (variable) @definition)
(data_type name: (name) @definition)
(newtype name: (name) @definition)
(type_synomym name: (name) @definition)
(class name: (name) @definition)
(patterns (variable) @definition)

(variable) @reference
(name) @reference
