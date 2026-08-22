; Outline extraction query for Task D (`outline()`), C++. Adapted from
; tree-sitter-cpp's queries/tags.scm (MIT, Copyright (c) 2014 Max
; Brunsfeld). Upstream `; inherits: c` is resolved by spelling the C
; patterns out here — see cpp/highlights.scm for why.

(struct_specifier name: (type_identifier) @name body: (_)) @definition.struct
(union_specifier name: (type_identifier) @name body: (_)) @definition.struct
(enum_specifier name: (type_identifier) @name body: (_)) @definition.enum
(class_specifier name: (type_identifier) @name body: (_)) @definition.class
(function_declarator declarator: (identifier) @name) @definition.function
(function_declarator declarator: (field_identifier) @name) @definition.method
(function_declarator
  declarator: (qualified_identifier name: (identifier) @name)) @definition.method
(field_declaration declarator: (field_identifier) @name) @definition.field
