; Identifier-occurrence query for A2, typescript. The JavaScript patterns
; are included directly (no `inherits:` support in the loader).
(function_declaration name: (identifier) @definition)
(function_expression name: (identifier) @definition)
(class_declaration name: (type_identifier) @definition)
(method_definition name: (property_identifier) @definition)
(variable_declarator name: (identifier) @definition)

(identifier) @reference
(property_identifier) @reference

(interface_declaration name: (type_identifier) @definition)
(type_alias_declaration name: (type_identifier) @definition)
(enum_declaration name: (identifier) @definition)
(required_parameter pattern: (identifier) @definition)
(optional_parameter pattern: (identifier) @definition)

(type_identifier) @reference
