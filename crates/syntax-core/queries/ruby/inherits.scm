; Supertype-edge query (`supertype_edges()`), Ruby. `class Foo < Bar`
; states the superclass syntactically, which is the one edge Ruby writes
; down. Mixins (`include`/`extend`/`prepend`) are ordinary method calls,
; indistinguishable from any other call without resolving what the receiver
; is, so they are deliberately not reported.
(class
  name: (constant) @type
  superclass: (superclass (constant) @supertype))
