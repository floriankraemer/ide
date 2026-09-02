; Outline extraction query for Task D (`outline()`), JavaScript.
; Same `tree-sitter-tags` convention as rust/tags.scm.
(class_declaration name: (identifier) @name) @definition.class
(method_definition name: (property_identifier) @name) @definition.method
(field_definition property: (property_identifier) @name) @definition.property
(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function

(variable_declarator
  name: (identifier) @name
  value: [(function_expression) (arrow_function)]) @definition.function
