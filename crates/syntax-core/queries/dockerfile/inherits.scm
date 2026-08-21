; Supertype-edge query (`supertype_edges()`), Dockerfile — intentionally
; empty, the same shape json/inherits.scm uses. A Dockerfile declares no
; types. `FROM base AS derived` is image layering, not a type hierarchy,
; and reporting it as a supertype edge would put a lie in the index.
