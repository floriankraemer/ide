; Outline extraction query for Mermaid (Task D, `outline()`), ported from
; tree-sitter-mermaid 0.1.0's `queries/portable/tags.scm` (MIT).
;
; Upstream tags four kinds; this engine maps a `definition.<kind>` capture
; onto `syntax_core::SymbolKind`, which has no Module and no Variable
; variant. Rather than paint a flowchart subgraph as a `Class` — a lie the
; Class View would then show — every module- and variable-kinded pattern is
; dropped here, and the kinds that map exactly are kept: entities,
; participants, states, requirements and ZenUML actors as `Class`,
; railroad/EBNF rules and journey/gantt tasks as `Function`.
;
; The dropped patterns come back the day `SymbolKind` grows a container or
; value variant; that is one revert of this filter, not a re-port. Markdown
; made the same call for headings (queries/markdown/tags.scm).

[
  (architecture_service_statement
    id: (architecture_identifier) @name)
  (architecture_junction_statement
    id: (architecture_identifier) @name)
] @definition.class

(c4_entity_declaration
  id: (c4_reference
    value: (c4_identifier) @name)) @definition.class

(class_declaration
  name: (class_name
    (identifier) @name)) @definition.class

; Entity Relationship.
(er_entity_declaration
  name: (er_entity_name) @name) @definition.class

; Event Modeling.
(event_entity_statement
  name: (event_qualified_name) @name) @definition.class

(gantt_task_statement
  metadata: (gantt_task_metadata
    (gantt_task_item
      value: (gantt_task_atom) @name))) @definition.function

(journey_task_statement
  task: (journey_task_name) @name) @definition.function

; Railroad constructor dialect.
(railroad_rule
  name: (railroad_identifier) @name) @definition.function

; Railroad ABNF.
(railroad_abnf_rule
  name: (railroad_abnf_rule_name) @name) @definition.function

; Railroad EBNF.
(railroad_ebnf_rule
  name: (railroad_ebnf_identifier) @name) @definition.function

; Railroad PEG.
(railroad_peg_rule
  name: (railroad_peg_identifier) @name) @definition.function

; Requirement.
[
  (requirement_declaration
    name: (requirement_name) @name)
  (requirement_element_declaration
    name: (requirement_name) @name)
] @definition.class

; Sequence.
(sequence_participant_declaration
  name: (sequence_participant_name) @name) @definition.class

; State.
(state_alias_clause
  name: (state_name) @name) @definition.class

[
  (state_named_declaration
    name: (state_name) @name)
  (state_pseudostate_declaration
    name: (state_name) @name)
] @definition.class

; ZenUML.
[
  (zenuml_starter_declaration
    participant: (zenuml_name) @name)
  (zenuml_participant_declaration
    name: (zenuml_name) @name)
] @definition.class
