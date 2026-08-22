; Identifier-occurrence query for A2 (`identifier_occurrences`), XML.
;
; XML has no variables. Element names are the closest analogue and, as in
; json/locals.scm, each occurrence stands alone rather than binding once
; and being referenced later — so every element name is a `@reference` and
; there is no `@definition` capture.

(STag (Name) @reference)
(ETag (Name) @reference)
(EmptyElemTag (Name) @reference)
