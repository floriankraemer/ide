; Supertype-edge query (`supertype_edges()`), PHP. Same convention as
; rust/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is one type it extends or implements.
;
; `base_clause` is `extends`, `class_interface_clause` is `implements`.
; PHP allows several names in either, and each yields its own match.

(class_declaration
  name: (name) @type
  (base_clause (name) @supertype))

(class_declaration
  name: (name) @type
  (class_interface_clause (name) @supertype))

(interface_declaration
  name: (name) @type
  (base_clause (name) @supertype))
