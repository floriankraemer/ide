; Behavior-preserving port of the old hand-rolled `classify_json` matcher
; to a tree-sitter query. Capture names map onto `TokenKind` in lib.rs.
; Standard JSON has no comments, so no @comment capture appears here,
; matching the old matcher.

(string) @string
(number) @number

(true) @keyword
(false) @keyword
(null) @keyword
