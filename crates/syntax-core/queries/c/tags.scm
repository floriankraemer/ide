; Outline extraction query for Task D (`outline()`), C. Adapted from
; tree-sitter-c's queries/tags.scm (MIT, Copyright (c) 2014 Max
; Brunsfeld); `@definition.type` there has no `SymbolKind` here, so a
; typedef and an enum map onto the nearest kinds this crate models.

(struct_specifier name: (type_identifier) @name body: (_)) @definition.struct
(union_specifier name: (type_identifier) @name body: (_)) @definition.struct
(enum_specifier name: (type_identifier) @name body: (_)) @definition.enum
(function_declarator declarator: (identifier) @name) @definition.function
(field_declaration declarator: (field_identifier) @name) @definition.field
