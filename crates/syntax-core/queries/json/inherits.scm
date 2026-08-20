; JSON has no type hierarchy, so this query is intentionally empty — the
; same shape json/tags.scm uses. `supertype_edges()` returns an empty vec
; for JSON without needing a `None` special case in the dispatcher.
