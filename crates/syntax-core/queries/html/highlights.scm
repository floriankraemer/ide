; HTML highlights.scm — tree-sitter-html 0.23.2's own
; `queries/highlights.scm` (MIT,
; https://github.com/tree-sitter/tree-sitter-html), unchanged.
;
; `@tag.error` has no entry in `syntax_core::SCOPES` and none of its
; ancestors resolve to one either, so it simply yields no spans — the
; documented fate of a capture name the taxonomy does not know.
;
; There is no `@keyword` pattern: HTML has no keywords. The harness still
; finds one in html/sample.txt because the `<script>` element injects
; JavaScript, which does — that is the point of this tranche.

(tag_name) @tag
(erroneous_end_tag_name) @tag.error
(doctype) @constant
(attribute_name) @attribute
(attribute_value) @string
(comment) @comment

[
  "<"
  ">"
  "</"
  "/>"
] @punctuation.bracket
