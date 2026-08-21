; Foldable regions (Task C), HTML: any element with a start and an end
; tag. `(script_element)` and `(style_element)` are separate node types
; and so need their own patterns.
(element) @fold
(script_element) @fold
(style_element) @fold
