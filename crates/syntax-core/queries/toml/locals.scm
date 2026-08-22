; Identifier-occurrence query for A2 (`identifier_occurrences`), TOML.
;
; TOML has no variables. Keys are the closest analogue and, as in
; json/locals.scm, each occurrence stands alone rather than binding once and
; being referenced later — so every key is a `@reference` and there is no
; `@definition` capture.

(bare_key) @reference
