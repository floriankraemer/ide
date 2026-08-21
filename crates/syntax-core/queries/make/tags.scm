; Outline extraction query for Task D (`outline()`), Make. A makefile's
; navigable things are its targets, and a target is the closest analogue of
; a callable, so it maps to `@definition.function`. Variables are
; deliberately not extracted: `SymbolKind` has no constant/variable kind,
; so a `@definition.constant` capture would compile and then be silently
; dropped by `symbol_kind_for_capture` — a query that looks like it works
; and does not.
(targets (word) @name) @definition.function
