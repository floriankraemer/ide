; Supertype-edge query (`supertype_edges()`), Kotlin. Same convention as
; java/inherits.scm: `@type` is the declaring type's name token,
; `@supertype` is one type it extends or implements.
;
; Kotlin does not distinguish the two syntactically — a superclass is
; written as a constructor call and an interface as a bare type, both in
; the same `delegation_specifiers` list — so both shapes are matched.

(class_declaration
  name: (identifier) @type
  (delegation_specifiers
    (delegation_specifier
      (constructor_invocation
        (user_type (identifier) @supertype)))))

(class_declaration
  name: (identifier) @type
  (delegation_specifiers
    (delegation_specifier
      (user_type (identifier) @supertype))))

(object_declaration
  name: (identifier) @type
  (delegation_specifiers
    (delegation_specifier
      (user_type (identifier) @supertype))))
