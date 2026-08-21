; Identifier-occurrence query for A2 (`identifier_occurrences`), CSS.
;
; CSS has no identifier binding at all: a selector names markup that lives
; in another file, and a declaration names a property the browser defines.
; Custom properties (`--brand`) are the one thing that is declared in one
; place and read in another, but the grammar gives them no node of their
; own — telling `--brand: red` from `color: red` needs a `#match?`
; predicate, which span extraction does not evaluate. So this query is
; intentionally empty, the same shape yaml/locals.scm uses.
