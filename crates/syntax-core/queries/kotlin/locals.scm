; Identifier-occurrence query for A2 (`identifier_occurrences`), Kotlin.
; Same convention as rust/locals.scm: `@definition` for identifiers in a
; declaration position, catch-all `(identifier) @reference` for every
; identifier.

(class_declaration name: (identifier) @definition)
(object_declaration name: (identifier) @definition)
(function_declaration name: (identifier) @definition)
(variable_declaration (identifier) @definition)
(parameter (identifier) @definition)
(class_parameter (identifier) @definition)
(enum_entry (identifier) @definition)

(identifier) @reference
