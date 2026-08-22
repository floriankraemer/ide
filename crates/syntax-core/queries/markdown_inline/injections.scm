; Injected regions (I1), Markdown inline grammar — from tree-sitter-md
; 0.5.3's own `tree-sitter-markdown-inline/queries/injections.scm` (MIT).
;
; Inline HTML (`<kbd>Esc</kbd>` in the middle of a sentence) is real HTML
; and highlights as such. The upstream `latex_block` pattern is dropped:
; there is no LaTeX row in the catalog for it to resolve to.

((html_tag) @injection.content
  (#set! injection.language "html"))
