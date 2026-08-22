; Identifier-occurrence query for A2 (`identifier_occurrences`),
; JavaScript. Same convention as rust/locals.scm: `@definition` for
; declaration positions, catch-all `@reference` for every identifier.
(function_declaration name: (identifier) @definition)
(function_expression name: (identifier) @definition)
(class_declaration name: (identifier) @definition)
(method_definition name: (property_identifier) @definition)
(variable_declarator name: (identifier) @definition)
(formal_parameters (identifier) @definition)

(identifier) @reference
(property_identifier) @reference
