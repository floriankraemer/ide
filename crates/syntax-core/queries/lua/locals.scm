; Identifier-occurrence query for A2 (`identifier_occurrences`), Lua. Same
; convention as rust/locals.scm: `@definition` for declaration positions,
; catch-all `@reference` for every identifier.

(variable_declaration
  (assignment_statement (variable_list name: (identifier) @definition)))
(function_declaration name: (identifier) @definition)
(parameters (identifier) @definition)
(for_generic_clause (variable_list name: (identifier) @definition))
(for_numeric_clause name: (identifier) @definition)

(identifier) @reference
