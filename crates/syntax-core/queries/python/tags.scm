; Outline extraction query for Task D (`outline()`), Python. Adapted from
; tree-sitter-python's queries/tags.scm (MIT, Copyright (c) 2016 Max
; Brunsfeld); the `@reference.call` and module-constant patterns are
; dropped — `outline()` only consumes `@definition.<kind>`.
;
; Python has no syntactic method/function distinction, so both are
; `definition.function`; nesting under the class comes from byte-range
; containment in lib.rs.

(class_definition name: (identifier) @name) @definition.class
(function_definition name: (identifier) @name) @definition.function
