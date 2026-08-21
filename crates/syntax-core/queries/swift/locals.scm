; Identifier-occurrence query for A2 (`identifier_occurrences`), Swift.
; Same convention as rust/locals.scm: `@definition` for identifiers in a
; declaration position, catch-all references for every identifier and type
; identifier.

(class_declaration name: (type_identifier) @definition)
(protocol_declaration name: (type_identifier) @definition)
(typealias_declaration name: (type_identifier) @definition)
(function_declaration name: (simple_identifier) @definition)
(parameter name: (simple_identifier) @definition)
(property_declaration name: (pattern bound_identifier: (simple_identifier) @definition))

(simple_identifier) @reference
(type_identifier) @reference
