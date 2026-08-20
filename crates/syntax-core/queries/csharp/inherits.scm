; Supertype-edge query (`supertype_edges()`), C#. Same convention as
; rust/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is one entry of its base list.
;
; C# does not distinguish a base class from an implemented interface
; syntactically — both live in the same `base_list` — so neither does this
; query. Only simple (non-generic) base names are captured; a generic base
; is a `generic_name`, not an `identifier`, and is skipped rather than
; reported under a wrong name.

(class_declaration
  name: (identifier) @type
  (base_list (identifier) @supertype))

(interface_declaration
  name: (identifier) @type
  (base_list (identifier) @supertype))

(struct_declaration
  name: (identifier) @type
  (base_list (identifier) @supertype))

(record_declaration
  name: (identifier) @type
  (base_list (identifier) @supertype))
