; Identifier-occurrence query for A2 (`identifier_occurrences`).
;
; JSON has no identifiers in the traditional sense (no variables, no
; function/type names) — object keys are the closest analog, and unlike a
; Rust `let` binding a key is not "defined" once and "referenced"
; elsewhere; each occurrence of a key stands alone. So this query captures
; every object key as `@reference` and defines no `@definition` capture at
; all: `identifier_occurrences(Language::Json, ...)` always returns
; occurrences with `is_definition = false`. Documented here rather than
; silently omitted so the choice is legible to whoever consumes this next
; (E1).

(pair key: (string) @reference)
