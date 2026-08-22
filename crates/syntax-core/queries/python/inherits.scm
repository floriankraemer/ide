; Supertype-edge query (`supertype_edges()`), Python. Same convention as
; rust/inherits.scm: `@type` is the declaring class's name token,
; `@supertype` is one base it lists. A base given as a dotted path
; (`abc.ABC`) captures the attribute's last identifier.

(class_definition
  name: (identifier) @type
  superclasses: (argument_list (identifier) @supertype))

(class_definition
  name: (identifier) @type
  superclasses: (argument_list (attribute attribute: (identifier) @supertype)))
