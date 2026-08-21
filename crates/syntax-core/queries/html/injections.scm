; Injected regions (I1), HTML — tree-sitter-html 0.23.2's own
; `queries/injections.scm` (MIT), unchanged.
;
; `(raw_text)` is the element's *content*, not the element: capturing
; `(script_element)` instead would feed `<script>` and `</script>` to the
; JavaScript parser and produce garbage spans.

((script_element
  (raw_text) @injection.content)
 (#set! injection.language "javascript"))

((style_element
  (raw_text) @injection.content)
 (#set! injection.language "css"))
