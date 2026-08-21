; Identifier-occurrence query for A2 (`identifier_occurrences`), CSS.
;
; CSS has no identifier binding at all: a selector names markup that lives
; in another file, and a declaration names a property the browser defines.
; Custom properties (`--brand`) are the one thing that is declared in one
; place and read in another, but the grammar gives them no node of their
; own — telling `--brand: red` from `color: red` needs a `#match?`
; guard. Those are evaluated (see queries/go/highlights.scm), so such a
; query could be written; nobody has, and one property name is thin
; pickings for an occurrence index. So this query is intentionally empty,
; the same shape yaml/locals.scm uses.
