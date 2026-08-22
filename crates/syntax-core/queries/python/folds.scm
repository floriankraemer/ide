; Foldable regions (Task C), Python. There are no braces: every suite —
; function body, class body, if/for/while/try body — is a `block`, and
; folding it collapses the indented region under its header. The
; bracketed literals fold too, since a long list or dict spans lines the
; same way a body does.
(block) @fold
(list) @fold
(dictionary) @fold
(set) @fold
(tuple) @fold
(argument_list) @fold
