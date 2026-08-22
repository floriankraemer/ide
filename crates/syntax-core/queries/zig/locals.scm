; Identifier-occurrence query for A2 (`identifier_occurrences`), Zig. Same
; convention as rust/locals.scm: `@definition` for identifiers in a
; declaration position, catch-all `(identifier) @reference` for every
; identifier.

(function_declaration name: (identifier) @definition)
(parameter name: (identifier) @definition)
; Anchored: only the *name* (the first named child) is the definition.
; Unanchored, this also matched the type in `var s: Shape = ...`, so a
; type usage was misreported as a definition of the type's name.
(variable_declaration . (identifier) @definition)
(container_field name: (identifier) @definition)

(identifier) @reference
