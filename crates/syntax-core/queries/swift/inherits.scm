; Supertype-edge query (`supertype_edges()`), Swift. Same convention as
; java/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is one type it inherits from or conforms to.
;
; Swift writes a superclass and a protocol conformance identically, as one
; `inheritance_specifier` list after the type name, so both come out of the
; same pattern.

(class_declaration
  name: (type_identifier) @type
  (inheritance_specifier
    inherits_from: (user_type (type_identifier) @supertype)))

(protocol_declaration
  name: (type_identifier) @type
  (inheritance_specifier
    inherits_from: (user_type (type_identifier) @supertype)))
