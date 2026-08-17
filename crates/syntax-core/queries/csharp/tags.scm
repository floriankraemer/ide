; Outline extraction query for Task D (`outline()`), C#. Same
; `tree-sitter-tags` convention as rust/tags.scm: whole definition node as
; `@definition.<kind>`, its identifier as `@name`, nesting done by
; byte-range containment in lib.rs (methods/fields nest under their
; class/struct automatically since their AST nodes sit inside it).

(class_declaration name: (identifier) @name) @definition.class
(interface_declaration name: (identifier) @name) @definition.interface
(struct_declaration name: (identifier) @name) @definition.struct
(enum_declaration name: (identifier) @name) @definition.enum
(method_declaration name: (identifier) @name) @definition.method
(constructor_declaration name: (identifier) @name) @definition.method
(field_declaration
  (variable_declaration
    (variable_declarator name: (identifier) @name))) @definition.field
(property_declaration name: (identifier) @name) @definition.field
