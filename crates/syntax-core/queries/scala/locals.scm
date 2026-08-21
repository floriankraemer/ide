; Identifier-occurrence query for A2 (`identifier_occurrences`), Scala.
; Same convention as rust/locals.scm: `@definition` for identifiers in a
; declaration position, catch-all references for every identifier and type
; identifier.

(class_definition name: (identifier) @definition)
(object_definition name: (identifier) @definition)
(trait_definition name: (identifier) @definition)
(enum_definition name: (identifier) @definition)
(type_definition name: (type_identifier) @definition)
(function_definition name: (identifier) @definition)
(function_declaration name: (identifier) @definition)
(val_definition pattern: (identifier) @definition)
(var_definition pattern: (identifier) @definition)
(parameter name: (identifier) @definition)
(class_parameter name: (identifier) @definition)

(identifier) @reference
(type_identifier) @reference
