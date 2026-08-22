; Identifier-occurrence query for A2 (`identifier_occurrences`), Ruby. Same
; convention as rust/locals.scm: `@definition` for declaration positions,
; catch-all `@reference` for every identifier.

(method name: (identifier) @definition)
(singleton_method name: (identifier) @definition)
(class name: (constant) @definition)
(module name: (constant) @definition)
(assignment left: (identifier) @definition)
(method_parameters (identifier) @definition)
(block_parameters (identifier) @definition)
(optional_parameter name: (identifier) @definition)
(keyword_parameter name: (identifier) @definition)

(identifier) @reference
(constant) @reference
(instance_variable) @reference
