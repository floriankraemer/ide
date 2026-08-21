; Outline extraction query for Task D (`outline()`), Haskell. Same
; `tree-sitter-tags` convention as rust/tags.scm.
;
; A `data`/`newtype` declaration is reported as a struct and a type class
; as an interface — the closest `SymbolKind` each has. A type signature is
; deliberately not extracted: it is a second mention of the equation that
; follows it, and reporting both would double every function in the
; outline.

(function name: (variable) @name) @definition.function
(bind name: (variable) @name) @definition.function
(data_type name: (name) @name) @definition.struct
(newtype name: (name) @name) @definition.struct
(class name: (name) @name) @definition.interface
(data_constructor constructor: (prefix name: (constructor) @name)) @definition.field
