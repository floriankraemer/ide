; Outline extraction query for Task D (`outline()`), Java. Same
; `tree-sitter-tags` convention as rust/tags.scm: whole definition node as
; `@definition.<kind>`, its identifier as `@name`, nesting by byte-range
; containment in lib.rs.

(class_declaration name: (identifier) @name) @definition.class
(interface_declaration name: (identifier) @name) @definition.interface
(enum_declaration name: (identifier) @name) @definition.enum
(method_declaration name: (identifier) @name) @definition.method
(constructor_declaration name: (identifier) @name) @definition.constructor
(field_declaration
  declarator: (variable_declarator name: (identifier) @name)) @definition.field
(enum_constant name: (identifier) @name) @definition.enum_member
