; Outline extraction query for Task D (`outline()`), Markdown.
;
; Headings are the obvious outline of a Markdown document, but `outline()`
; maps captures onto `SymbolKind` (class, struct, enum, interface, method,
; function, field) and a heading is none of those; forcing one on would
; put a lie in the Class View. Intentionally empty until `SymbolKind`
; grows a heading-shaped variant, the same shape yaml/tags.scm uses.
