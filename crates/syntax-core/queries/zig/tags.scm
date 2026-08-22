; Outline extraction query for Task D (`outline()`), Zig. Same
; `tree-sitter-tags` convention as rust/tags.scm.
;
; Zig has no named type declarations: a container is an anonymous
; `struct`/`enum`/`union` expression bound by a `const`, so the name comes
; from the enclosing `variable_declaration` and the kind from the
; expression it binds. A `union` is reported as a struct because
; `SymbolKind` has no union variant; a plain `const` that binds anything
; else is a value, not a definition, and is deliberately not extracted.

(function_declaration name: (identifier) @name) @definition.function
(variable_declaration (identifier) @name (struct_declaration)) @definition.struct
(variable_declaration (identifier) @name (union_declaration)) @definition.struct
(variable_declaration (identifier) @name (enum_declaration)) @definition.enum
(container_field name: (identifier) @name) @definition.field
