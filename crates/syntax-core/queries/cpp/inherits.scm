; Supertype-edge query (`supertype_edges()`), C++. Same convention as
; rust/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is one base it derives from. Each base in a base-class
; clause yields its own match.

(class_specifier
  name: (type_identifier) @type
  (base_class_clause (type_identifier) @supertype))

(struct_specifier
  name: (type_identifier) @type
  (base_class_clause (type_identifier) @supertype))
