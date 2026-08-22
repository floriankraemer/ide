; Supertype-edge query (`supertype_edges()`), JavaScript. Same convention
; as rust/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is the class it extends. `extends` takes an arbitrary
; expression in JavaScript; only the plain-identifier form names a type
; statically, so that is the only form reported.

(class_declaration
  name: (type_identifier) @type
  (class_heritage (extends_clause value: (identifier) @supertype)))

; TypeScript also writes the relation down on interfaces and via
; `implements`.

(class_declaration
  name: (type_identifier) @type
  (class_heritage (implements_clause (type_identifier) @supertype)))

(interface_declaration
  name: (type_identifier) @type
  (extends_type_clause type: (type_identifier) @supertype))
