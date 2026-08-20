; Supertype-edge query (`supertype_edges()`), Rust. Each pattern captures
; the *subtype's* name token as `@type` and one supertype it declares as
; `@supertype`; a declaration listing several supertypes yields one match
; per supertype, so lib.rs emits one edge per pair with no extra work.
;
; Rust's "implements" relation is `impl Trait for Type`. Consistent with
; tags.scm's documented scope boundary, only simple (non-generic) impl
; targets are captured — a generic `impl<T> Trait for Foo<T>` has a
; `generic_type` where this expects a `type_identifier` and is skipped
; rather than reported under a wrong name.

(impl_item
  trait: (type_identifier) @supertype
  type: (type_identifier) @type)

; Supertraits: `trait Shape: Debug`. The bound list is a named child, so
; the pattern deliberately does not name the field — field names are the
; part most likely to drift between grammar releases.
(trait_item
  name: (type_identifier) @type
  (trait_bounds (type_identifier) @supertype))
