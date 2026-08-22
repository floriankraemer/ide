; Identifier-occurrence query for A2 (`identifier_occurrences`), C++. Same
; convention as rust/locals.scm; the C declaration positions plus the C++
; ones (`qualified_identifier` definitions, template parameters).

(function_declarator declarator: (identifier) @definition)
(function_declarator declarator: (qualified_identifier name: (identifier) @definition))
(parameter_declaration declarator: (identifier) @definition)
(optional_parameter_declaration declarator: (identifier) @definition)
(init_declarator declarator: (identifier) @definition)
(preproc_def name: (identifier) @definition)
(preproc_function_def name: (identifier) @definition)

(identifier) @reference
