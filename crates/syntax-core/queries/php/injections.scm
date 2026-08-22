; Injected regions (I1), PHP.
;
; The catalog's `php` row uses `LANGUAGE_PHP`, the grammar that parses a
; whole template file: everything outside `<?php … ?>` is a `(text)` node,
; which is HTML and is highlighted as such. Without this, the markup half
; of a template file would be one uncoloured blob — which is exactly why
; the row used the body-only grammar until the markup tranche shipped an
; `html` row to inject.

((text) @injection.content
  (#set! injection.language "html"))
