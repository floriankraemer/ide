; Supertype-edge query (`supertype_edges()`), JavaScript. Same convention
; as rust/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is the class it extends. `extends` takes an arbitrary
; expression in JavaScript; only the plain-identifier form names a type
; statically, so that is the only form reported.

(class_declaration
  name: (identifier) @type
  (class_heritage (identifier) @supertype))
