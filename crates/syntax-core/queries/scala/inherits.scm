; Supertype-edge query (`supertype_edges()`), Scala. Same convention as
; java/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is one type it extends.
;
; `extends A with B` puts every parent in one `extends_clause`, one `type:`
; child each, so a declaration with several parents yields one match — and
; therefore one edge — per parent.

(class_definition
  name: (identifier) @type
  (extends_clause type: (type_identifier) @supertype))

(object_definition
  name: (identifier) @type
  (extends_clause type: (type_identifier) @supertype))

(trait_definition
  name: (identifier) @type
  (extends_clause type: (type_identifier) @supertype))

(enum_definition
  name: (identifier) @type
  (extends_clause type: (type_identifier) @supertype))
