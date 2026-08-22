; Identifier-occurrence query for A2 (`identifier_occurrences`), Java.
; Same convention as rust/locals.scm: `@definition` for identifiers in a
; declaration position, catch-all `(identifier) @reference` for every
; identifier (folded by byte-range with OR in lib.rs, so a definition site
; that also matches the catch-all collapses to `is_definition = true`).

(class_declaration name: (identifier) @definition)
(interface_declaration name: (identifier) @definition)
(enum_declaration name: (identifier) @definition)
(constructor_declaration name: (identifier) @definition)
(method_declaration name: (identifier) @definition)
(formal_parameter name: (identifier) @definition)
(variable_declarator name: (identifier) @definition)

(identifier) @reference
; Type usages (`extends Foo`, a field's type, `new Foo()`) are
; `type_identifier` nodes, not `identifier` — without this catch-all they
; are not occurrences at all, and Go to Declaration / Rename on a type use
; reports "no symbol under the caret".
(type_identifier) @reference
