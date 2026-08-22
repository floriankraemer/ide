; Identifier-occurrence query for A2 (`identifier_occurrences`), Python.
; Same convention as rust/locals.scm: `@definition` for declaration
; positions, catch-all `(identifier) @reference` for every identifier.

(function_definition name: (identifier) @definition)
(class_definition name: (identifier) @definition)
(parameters (identifier) @definition)
(default_parameter name: (identifier) @definition)
(typed_parameter (identifier) @definition)
(assignment left: (identifier) @definition)
(for_statement left: (identifier) @definition)
(global_statement (identifier) @definition)

(identifier) @reference
