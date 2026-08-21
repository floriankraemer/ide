; Outline extraction query for Task D (`outline()`), SQL. A schema script's
; named definitions are the objects it creates.
;
; Tables and views are both relations and both map to `@definition.struct`:
; `SymbolKind` has no separate concept for a view, and inventing one by
; borrowing `interface` would put a wrong icon next to it.
(create_table (object_reference name: (identifier) @name)) @definition.struct
(create_view (object_reference name: (identifier) @name)) @definition.struct
(create_function (object_reference name: (identifier) @name)) @definition.function
(column_definition name: (identifier) @name) @definition.field
