; Identifier-occurrence query for A2 (`identifier_occurrences`), C. Same
; convention as rust/locals.scm: `@definition` for declaration positions,
; catch-all `(identifier) @reference` for every identifier.

(function_declarator declarator: (identifier) @definition)
(parameter_declaration declarator: (identifier) @definition)
(init_declarator declarator: (identifier) @definition)
(preproc_def name: (identifier) @definition)
(preproc_function_def name: (identifier) @definition)

; A struct/union/enum tag and a typedef name are `type_identifier` nodes,
; not `identifier`. The tag is only a definition when a body follows —
; `struct point origin;` names the same tag as a use.
(struct_specifier name: (type_identifier) @definition body: (field_declaration_list))
(union_specifier name: (type_identifier) @definition body: (field_declaration_list))
(enum_specifier name: (type_identifier) @definition body: (enumerator_list))
(type_definition declarator: (type_identifier) @definition)
(field_declaration declarator: (field_identifier) @definition)

(identifier) @reference
(type_identifier) @reference
(field_identifier) @reference
