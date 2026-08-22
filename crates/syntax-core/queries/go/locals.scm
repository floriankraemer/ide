; Identifier-occurrence query for A2 (`identifier_occurrences`), Go. Same
; convention as rust/locals.scm: `@definition` for declaration positions,
; catch-all `@reference` for every identifier.

(function_declaration name: (identifier) @definition)
(method_declaration name: (field_identifier) @definition)
(type_spec name: (type_identifier) @definition)
(var_spec name: (identifier) @definition)
(const_spec name: (identifier) @definition)
(parameter_declaration name: (identifier) @definition)
(short_var_declaration left: (expression_list (identifier) @definition))

(identifier) @reference
(field_identifier) @reference
(type_identifier) @reference
