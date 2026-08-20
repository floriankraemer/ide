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

; The definition patterns below mirror rust/tags.scm's, so that every
; construct the outline calls a definition is also a definition here. They
; drifted apart once: traits, `impl` targets and struct fields were only in
; tags.scm, which left them out of the index's `is_definition` flag
; entirely — Go to Declaration on a trait name found nothing, and a field
; name was not even indexed as a reference, because a field is a
; `field_identifier`, a node kind neither catch-all below used to cover.
(function_item name: (identifier) @definition)
(struct_item name: (type_identifier) @definition)
(enum_item name: (type_identifier) @definition)
(trait_item name: (type_identifier) @definition)
(impl_item type: (type_identifier) @definition)
(field_declaration name: (field_identifier) @definition)
(parameter pattern: (identifier) @definition)
(let_declaration pattern: (identifier) @definition)
(const_item name: (identifier) @definition)
(static_item name: (identifier) @definition)

(identifier) @reference
(type_identifier) @reference
(field_identifier) @reference
