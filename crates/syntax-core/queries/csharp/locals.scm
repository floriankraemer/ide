; Identifier-occurrence query for A2 (`identifier_occurrences`), C#.
; Same convention as rust/locals.scm: `@definition` for identifiers in a
; declaration position, catch-all `(identifier) @reference` for every
; identifier (including definition sites, which also match the catch-all
; — `identifier_occurrences()` folds by byte-range with OR, so a node
; captured as both collapses into one `Occurrence` with
; `is_definition = true`).

(class_declaration name: (identifier) @definition)
(interface_declaration name: (identifier) @definition)
(struct_declaration (identifier) @definition)
(enum_declaration name: (identifier) @definition)
(record_declaration (identifier) @definition)
(method_declaration name: (identifier) @definition)
(constructor_declaration name: (identifier) @definition)
(local_function_statement name: (identifier) @definition)
(parameter name: (identifier) @definition)
(variable_declarator name: (identifier) @definition)
; Task 4a: `property_declaration`/`enum_member_declaration` joined
; tags.scm's `@definition.<kind>` set (auto-properties, enum members) but
; were missing here — the same drift the comment above warns about, this
; time catching a property/enum-member name in the "no @definition
; anywhere" gap, which left it referenced but never indexed as a
; definition (`sym_is_definition` never set) even though `outline()`
; already reported it as one.
(property_declaration name: (identifier) @definition)
(enum_member_declaration name: (identifier) @definition)

(identifier) @reference
