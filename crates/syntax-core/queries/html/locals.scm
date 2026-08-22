; Identifier-occurrence query for A2 (`identifier_occurrences`), HTML.
;
; HTML has no identifiers that bind: an `id` attribute names a node for
; CSS and JavaScript to find, but the grammar sees only an
; `(attribute_value)` whose meaning depends on the attribute name. An
; `#eq?` guard on that name would work — those predicates are evaluated
; (see queries/go/highlights.scm) — but an `id` is referenced from other
; files, which is not an occurrence this crate resolves, so there is
; nothing here worth binding. Intentionally empty, the same shape
; yaml/locals.scm uses.
