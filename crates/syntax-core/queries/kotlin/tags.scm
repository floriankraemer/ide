; Outline extraction query for Task D (`outline()`), Kotlin. Same
; `tree-sitter-tags` convention as rust/tags.scm.
;
; `class`, `interface` and `enum class` share one `class_declaration` node
; in this grammar, so the leading keyword token is matched to tell them
; apart. An `enum class` carries both `enum` and `class` tokens and is
; therefore reported as a class — which it is. Splitting it out would need
; a `#eq?` predicate; those are evaluated (see queries/go/highlights.scm),
; so it could be written — it just has not been.
;
; Kotlin has one `function_declaration` node at every nesting level, so
; every function is reported as `definition.function`; `outline()` nests it
; under its class by byte-range containment anyway.

(class_declaration "class" name: (identifier) @name) @definition.class
(class_declaration "interface" name: (identifier) @name) @definition.interface
(object_declaration name: (identifier) @name) @definition.class
(function_declaration name: (identifier) @name) @definition.function
; Kotlin properties (`val`/`var`) are a first-class language concept, not
; merely fields, so this gets the dedicated kind; enum entries likewise
; have their own grammar node.
(property_declaration (variable_declaration (identifier) @name)) @definition.property
(enum_entry (identifier) @name) @definition.enum_member
