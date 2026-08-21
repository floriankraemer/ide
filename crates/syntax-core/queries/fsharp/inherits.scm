; Supertype-edge query (`supertype_edges()`), F#. Same convention as
; java/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is one type it inherits from or implements.
;
; F# separates the two: `inherit` for the single base class, `interface
; ... with` for each implemented interface, so both shapes are matched.

(anon_type_defn
  (type_name type_name: (identifier) @type)
  (class_inherits_decl (simple_type (long_identifier (identifier) @supertype))))

(anon_type_defn
  (type_name type_name: (identifier) @type)
  (type_extension_elements
    (interface_implementation (simple_type (long_identifier (identifier) @supertype)))))
