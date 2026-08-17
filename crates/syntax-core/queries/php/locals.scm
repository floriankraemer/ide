; Identifier-occurrence query for A2 (`identifier_occurrences`), PHP.
;
; PHP nests a `(name)` node inside every `(variable_name)` node (`$foo` is
; `(variable_name (name))`), unlike Rust/C#/Java where identifier and
; type-identifier are sibling-level node kinds. A naive catch-all
; `(name) @reference` would therefore double-count every variable
; occurrence (once for the outer `variable_name` span "$foo", once for the
; inner `name` span "foo"). To avoid that, this query only captures bare
; `(name)` at specific known reference sites (call/instantiation targets)
; rather than with an unrestricted catch-all, and uses `(variable_name)`
; itself — not its inner `name` — as the unit of occurrence for `$foo`
; sites. Same `@definition`/`@reference` fold-by-byte-range convention as
; the other languages' locals.scm otherwise.

(class_declaration name: (name) @definition)
(interface_declaration name: (name) @definition)
(trait_declaration name: (name) @definition)
(function_definition name: (name) @definition)
(method_declaration name: (name) @definition)
(simple_parameter name: (variable_name) @definition)
(property_element name: (variable_name) @definition)

(variable_name) @reference
(function_call_expression function: (name) @reference)
(member_call_expression name: (name) @reference)
(scoped_call_expression name: (name) @reference)
(object_creation_expression (name) @reference)
