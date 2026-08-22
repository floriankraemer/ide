; Outline extraction query for Task D (`outline()`), Scala. Same
; `tree-sitter-tags` convention as rust/tags.scm.
;
; An `object` is reported as a class: it is a singleton class, and
; `SymbolKind` has no module variant. `given` and `extension` definitions
; have no `SymbolKind` either and are deliberately not extracted.

(class_definition name: (identifier) @name) @definition.class
(object_definition name: (identifier) @name) @definition.class
(trait_definition name: (identifier) @name) @definition.interface
(enum_definition name: (identifier) @name) @definition.enum
(function_definition name: (identifier) @name) @definition.method
(function_declaration name: (identifier) @name) @definition.method
(val_definition pattern: (identifier) @name) @definition.field
(var_definition pattern: (identifier) @name) @definition.field
(class_parameter name: (identifier) @name) @definition.field
