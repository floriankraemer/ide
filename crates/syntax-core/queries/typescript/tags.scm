; Outline extraction query for Task D (`outline()`), typescript. The
; JavaScript patterns are included directly (no `inherits:` support).
(class_declaration name: (type_identifier) @name) @definition.class
(method_definition name: (property_identifier) @name) @definition.method
(public_field_definition name: (property_identifier) @name) @definition.property
(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function

(variable_declarator
  name: (identifier) @name
  value: [(function_expression) (arrow_function)]) @definition.function

(abstract_class_declaration name: (type_identifier) @name) @definition.class
(interface_declaration name: (type_identifier) @name) @definition.interface
(enum_declaration name: (identifier) @name) @definition.enum
(function_signature name: (identifier) @name) @definition.function
(method_signature name: (property_identifier) @name) @definition.method
(abstract_method_signature name: (property_identifier) @name) @definition.method
; Restricted to interface bodies on purpose: a bare `property_signature`
; also matches members of an inline object type (a destructured
; parameter's annotation), which is not an outline entry.
(interface_body (property_signature name: (property_identifier) @name) @definition.property)
; Enum members: `enum_body`'s own `name:` field covers a bare `Foo` entry,
; `enum_assignment` covers `Foo = 1`. The bare-entry definition is the name
; token itself, not the whole `enum_body` -- else every bare member's
; "whole definition" range would span (and byte-range-contain) every other
; member's, corrupting `outline()`'s containment-based nesting.
(enum_body name: (property_identifier) @definition.enum_member @name)
(enum_assignment name: (property_identifier) @name) @definition.enum_member
