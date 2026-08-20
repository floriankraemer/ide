; Supertype-edge query (`supertype_edges()`), Java. Same convention as
; rust/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is one type it extends or implements.

(class_declaration
  name: (identifier) @type
  (superclass (type_identifier) @supertype))

(class_declaration
  name: (identifier) @type
  (super_interfaces (type_list (type_identifier) @supertype)))

(interface_declaration
  name: (identifier) @type
  (extends_interfaces (type_list (type_identifier) @supertype)))

(enum_declaration
  name: (identifier) @type
  (super_interfaces (type_list (type_identifier) @supertype)))
