; Outline extraction query for Task D (`outline()`), F#. Same
; `tree-sitter-tags` convention as rust/tags.scm.
;
; F# type definitions are one `type_definition` per shape, so the shape
; node picks the `SymbolKind`: a discriminated union is an enum, a record
; is a struct, a class/interface definition is a class. `let`-bound values
; are not extracted — only `let`-bound functions, which are the ones an
; outline is for.

(function_or_value_defn (function_declaration_left (identifier) @name)) @definition.function
(union_type_defn (type_name type_name: (identifier) @name)) @definition.enum
(record_type_defn (type_name type_name: (identifier) @name)) @definition.struct
(anon_type_defn (type_name type_name: (identifier) @name)) @definition.class
(enum_type_defn (type_name type_name: (identifier) @name)) @definition.enum
(member_defn (method_or_prop_defn name: (property_or_ident method: (identifier) @name))) @definition.method
(record_field (identifier) @name) @definition.field
(union_type_case (identifier) @name) @definition.field
