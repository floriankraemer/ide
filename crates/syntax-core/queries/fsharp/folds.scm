; Foldable regions (Task C), F#. F# is indentation-delimited and has no
; brace-delimited block node, so the foldable regions are the declaration
; nodes themselves rather than their bodies.
(type_definition) @fold
(function_or_value_defn) @fold
(union_type_cases) @fold
(record_fields) @fold
(type_extension_elements) @fold
