; Outline extraction query for Task D (`outline()`), JSON.
;
; JSON has no classes, methods, or fields in the source-code sense a Class
; View would show — a JSON document is just nested values. There is
; nothing meaningful to extract, so this file is intentionally empty (zero
; patterns, which is a valid, trivially-compiling tree-sitter query).
; `outline(Language::Json, ...)` returns an empty Vec, mirroring how
; `identifier_occurrences()` handles languages with nothing to report.
