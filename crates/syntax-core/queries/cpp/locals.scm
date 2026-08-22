; Identifier-occurrence query for A2 (`identifier_occurrences`), C++. Same
; convention as rust/locals.scm; the C declaration positions plus the C++
; ones (`qualified_identifier` definitions, template parameters).

(function_declarator declarator: (identifier) @definition)
(function_declarator declarator: (qualified_identifier name: (identifier) @definition))
(function_declarator declarator: (field_identifier) @definition)
(parameter_declaration declarator: (identifier) @definition)
(optional_parameter_declaration declarator: (identifier) @definition)
(init_declarator declarator: (identifier) @definition)
(preproc_def name: (identifier) @definition)
(preproc_function_def name: (identifier) @definition)

; A class/struct/union/enum name and a type alias are `type_identifier`
; nodes, not `identifier`. The name is only a definition when a body
; follows — `class Foo;` and `Foo *p;` are uses of the same name.
(class_specifier name: (type_identifier) @definition body: (field_declaration_list))
(struct_specifier name: (type_identifier) @definition body: (field_declaration_list))
(union_specifier name: (type_identifier) @definition body: (field_declaration_list))
(enum_specifier name: (type_identifier) @definition body: (enumerator_list))
(alias_declaration name: (type_identifier) @definition)
(type_definition declarator: (type_identifier) @definition)
(field_declaration declarator: (field_identifier) @definition)

(identifier) @reference
(type_identifier) @reference
(field_identifier) @reference
