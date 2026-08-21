; Identifier-occurrence query for A2 (`identifier_occurrences`), F#. Same
; convention as rust/locals.scm: `@definition` for identifiers in a
; declaration position, catch-all `(identifier) @reference` for every
; identifier.

(function_declaration_left (identifier) @definition)
(value_declaration_left (identifier_pattern (long_identifier_or_op (identifier) @definition)))
(type_name type_name: (identifier) @definition)
(union_type_case (identifier) @definition)

(identifier) @reference
