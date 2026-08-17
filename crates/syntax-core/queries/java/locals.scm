; Identifier-occurrence query for A2 (`identifier_occurrences`), Java.
; Same convention as rust/locals.scm: `@definition` for identifiers in a
; declaration position, catch-all `(identifier) @reference` for every
; identifier (folded by byte-range with OR in lib.rs, so a definition site
; that also matches the catch-all collapses to `is_definition = true`).

(class_declaration name: (identifier) @definition)
(interface_declaration name: (identifier) @definition)
(enum_declaration name: (identifier) @definition)
(constructor_declaration name: (identifier) @definition)
(method_declaration name: (identifier) @definition)
(formal_parameter name: (identifier) @definition)
(variable_declarator name: (identifier) @definition)

(identifier) @reference
