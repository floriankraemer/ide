; Supertype-edge query (`supertype_edges()`), Haskell. Same convention as
; java/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is one type it declares itself a member of.
;
; Haskell's analogue of "implements an interface" is an instance
; declaration, and it is written the other way round from every other
; language here — the class comes first and the instantiating type second
; — so `@supertype` is the head and `@type` the argument. Only instances
; whose head argument is a plain type name produce an edge; an instance on
; a structural type (`instance Container []`) has no name token to anchor
; one and is skipped.

(instance
  name: (name) @supertype
  patterns: (type_patterns (name) @type))
