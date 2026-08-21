; Identifier-occurrence query for A2 (`identifier_occurrences`), HTML.
;
; HTML has no identifiers that bind: an `id` attribute names a node for
; CSS and JavaScript to find, but the grammar sees only an
; `(attribute_value)` whose meaning depends on the attribute name, which
; needs a `#eq?` predicate that span extraction does not evaluate.
; Intentionally empty, the same shape yaml/locals.scm uses.
