; Foldable regions (Task C): function/impl/trait/mod bodies, struct/enum
; field lists, match blocks. `block` also covers if/while/loop/unsafe
; bodies, which is fine — folding a plain `{ ... }` block is reasonable
; too, not just definitions.
(block) @fold
(declaration_list) @fold
(field_declaration_list) @fold
(enum_variant_list) @fold
(match_block) @fold
