; Identifier-occurrence query for A2 (`identifier_occurrences`), separate
; from `highlights.scm`. Convention borrowed from the community
; `tree-sitter-tags`/locals queries: `@definition` for identifiers in a
; declaration position, `@reference` for identifiers as read anywhere.
;
; tree-sitter query patterns can't express "an identifier that is NOT in a
; definition position" directly, so the catch-all `(identifier) @reference`
; / `(type_identifier) @reference` patterns below also match definition-site
; nodes (e.g. a function name matches both the `@definition` pattern and the
; catch-all `@reference` pattern). `identifier_occurrences()` in lib.rs
; folds captures by node byte-range with OR, so a node captured as both
; collapses into one `Occurrence` with `is_definition = true` — the
; catch-all is intentionally broad, and the fold-by-range step is what
; makes it correct.

(function_item name: (identifier) @definition)
(struct_item name: (type_identifier) @definition)
(enum_item name: (type_identifier) @definition)
(parameter pattern: (identifier) @definition)
(let_declaration pattern: (identifier) @definition)
(const_item name: (identifier) @definition)
(static_item name: (identifier) @definition)

(identifier) @reference
(type_identifier) @reference
