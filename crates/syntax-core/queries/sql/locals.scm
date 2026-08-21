; Identifier-occurrence query for A2 (`identifier_occurrences`), SQL.
;
; SQL's only binding construct in a single statement is the alias: a CTE
; name, a table alias or a column alias is introduced once and referenced
; by later clauses. Everything else — table names, column names — is a
; reference to a schema object this crate cannot see, so it is captured as
; `@reference` only.

(relation alias: (identifier) @definition)
(term alias: (identifier) @definition)
(cte (identifier) @definition)

(identifier) @reference
