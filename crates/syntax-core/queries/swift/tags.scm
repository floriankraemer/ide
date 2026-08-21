; Outline extraction query for Task D (`outline()`), Swift. Same
; `tree-sitter-tags` convention as rust/tags.scm.
;
; `class`, `struct`, `enum`, `actor` and `extension` share one
; `class_declaration` node in this grammar, distinguished by its
; `declaration_kind` token — matched literally here, since this crate does
; not evaluate `#eq?` predicates. `extension` and `actor` have no
; `SymbolKind` and are deliberately not extracted.

(class_declaration declaration_kind: "class" name: (type_identifier) @name) @definition.class
(class_declaration declaration_kind: "struct" name: (type_identifier) @name) @definition.struct
(class_declaration declaration_kind: "enum" name: (type_identifier) @name) @definition.enum
(protocol_declaration name: (type_identifier) @name) @definition.interface
(function_declaration name: (simple_identifier) @name) @definition.method
(protocol_function_declaration name: (simple_identifier) @name) @definition.method
(property_declaration name: (pattern bound_identifier: (simple_identifier) @name)) @definition.field
(enum_entry name: (simple_identifier) @name) @definition.field
