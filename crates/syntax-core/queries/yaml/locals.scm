; Identifier-occurrence query for A2 (`identifier_occurrences`), YAML.
;
; YAML has no variables or declarations; mapping keys are the closest
; analogue, and — exactly as in json/locals.scm — each occurrence of a key
; stands alone rather than binding once and being referenced later. So
; every key is a `@reference` and there is no `@definition` capture:
; `identifier_occurrences(yaml, ...)` always reports `is_definition = false`.
;
; Anchors and aliases *are* a real define/use pair, so they get the one
; genuine definition/reference relationship YAML has.

(anchor_name) @definition
(alias_name) @reference

(block_mapping_pair key: (flow_node) @reference)
