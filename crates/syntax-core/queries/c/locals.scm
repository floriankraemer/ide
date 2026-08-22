; Identifier-occurrence query for A2 (`identifier_occurrences`), C. Same
; convention as rust/locals.scm: `@definition` for declaration positions,
; catch-all `(identifier) @reference` for every identifier.

(function_declarator declarator: (identifier) @definition)
(parameter_declaration declarator: (identifier) @definition)
(init_declarator declarator: (identifier) @definition)
(preproc_def name: (identifier) @definition)
(preproc_function_def name: (identifier) @definition)

(identifier) @reference
