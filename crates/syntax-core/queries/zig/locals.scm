; Identifier-occurrence query for A2 (`identifier_occurrences`), Zig. Same
; convention as rust/locals.scm: `@definition` for identifiers in a
; declaration position, catch-all `(identifier) @reference` for every
; identifier.

(function_declaration name: (identifier) @definition)
(parameter name: (identifier) @definition)
(variable_declaration (identifier) @definition)
(container_field name: (identifier) @definition)

(identifier) @reference
