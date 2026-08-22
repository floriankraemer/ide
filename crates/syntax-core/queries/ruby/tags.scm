; Outline extraction query for Task D (`outline()`), Ruby. Same
; `tree-sitter-tags` convention as rust/tags.scm.
;
; A `singleton_method` (`def self.foo`) is a method on the class object, so
; it is reported as a method like any other; Ruby draws no syntactic line
; the outline could show differently. A `module` is reported as a class:
; `SymbolKind` has no module/namespace kind, and `@definition.module` would
; compile and then be silently dropped by `symbol_kind_for_capture`.
(class name: (constant) @name) @definition.class
(module name: (constant) @name) @definition.class
(method name: [(identifier) (constant)] @name) @definition.method
(singleton_method name: [(identifier) (constant)] @name) @definition.method
