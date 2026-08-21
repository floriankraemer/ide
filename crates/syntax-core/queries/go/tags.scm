; Outline extraction query for Task D (`outline()`), Go. Same
; `tree-sitter-tags` convention as rust/tags.scm. Go's `type_spec` is one
; node for every named type, so struct and interface are distinguished by
; the type expression on the right-hand side; a type alias to anything
; else has no matching SymbolKind and is deliberately not extracted.

(function_declaration name: (identifier) @name) @definition.function
(method_declaration name: (field_identifier) @name) @definition.method
(type_spec name: (type_identifier) @name type: (struct_type)) @definition.struct
(type_spec name: (type_identifier) @name type: (interface_type)) @definition.interface
(field_declaration name: (field_identifier) @name) @definition.field
