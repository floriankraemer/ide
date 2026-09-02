; Outline extraction query for Task D (`outline()`), Rust. Follows the
; `tree-sitter-tags` convention: each pattern captures the whole definition
; node as `@definition.<kind>` and its identifier as `@name`. `outline()`
; in lib.rs nests definitions by AST byte-range containment (a struct's
; fields nest under the struct, a fn nests under the impl block that
; contains it), so no explicit parent-pointer capture is needed here.
;
; Rust has no classes; `impl` blocks are the closest structural container
; for a type's methods, so they map onto `SymbolKind::Class` here (a
; pragmatic label — "the group of things attached to this type" — rather
; than a literal class). Only simple (non-generic, non-trait) impl targets
; are captured (`type: (type_identifier)`); `impl Trait for Type` and
; generic impls are out of scope for this first slice. Functions are
; always `SymbolKind::Function`, whether free-standing or nested inside an
; `impl` block as a method — the query has no way to distinguish the two
; without a predicate, and containment in the tree already tells the UI
; which is which.

(struct_item name: (type_identifier) @name) @definition.struct
(enum_item name: (type_identifier) @name) @definition.enum
(trait_item name: (type_identifier) @name) @definition.interface
(impl_item type: (type_identifier) @name) @definition.class
(function_item name: (identifier) @name) @definition.function
(field_declaration name: (field_identifier) @name) @definition.field
; `const`/`static` items have their own grammar node, unlike a constructor
; (Rust has no dedicated constructor syntax to distinguish from a plain
; associated `fn`), so only these two get a new kind here.
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.constant
