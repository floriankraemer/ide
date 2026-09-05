; Identifier-occurrence query for Mermaid (A2, `identifier_occurrences`),
; ported from tree-sitter-mermaid 0.1.0's `queries/portable/locals.scm`
; (MIT).
;
; Renamed to this engine's capture names: upstream's `@local.definition`
; and `@local.reference` are `@definition` and `@reference` here (see the
; header of rust/locals.scm). Upstream's single `@local.scope` pattern is
; dropped rather than renamed — this engine has no scope concept in
; `locals.scm`; it folds captures by node range and asks only "is this
; node a definition site", so a scope capture would describe nothing.


; Architecture.
(architecture_group_statement
  id: (architecture_identifier) @definition)

[
  (architecture_service_statement
    id: (architecture_identifier) @definition)
  (architecture_junction_statement
    id: (architecture_identifier) @definition)
]

(architecture_parent_clause
  parent: (architecture_identifier) @reference)

(architecture_edge_endpoint
  id: (architecture_identifier) @reference)

(architecture_alignment_statement
  member: (architecture_identifier) @reference)

; Block.
(block_node
  id: (block_identifier) @definition
  shape: (_))

(block_edge_statement
  source: (block_node
    id: (block_identifier) @reference
    !shape))

(block_edge_statement
  target: (block_node
    id: (block_identifier) @reference
    !shape))

; C4.
(c4_entity_declaration
  id: (c4_reference
    value: (c4_identifier) @definition))

(c4_boundary_statement
  id: (c4_reference
    value: (c4_identifier) @definition))

(c4_relationship_statement
  source: (c4_reference
    value: (c4_identifier) @reference))

(c4_relationship_statement
  target: (c4_reference
    value: (c4_identifier) @reference))

(c4_style_update_statement
  source: (c4_reference
    value: (c4_identifier) @reference))

(c4_style_update_statement
  target: (c4_reference
    value: (c4_identifier) @reference))

; Class.
(class_namespace_declaration
  name: (class_namespace_name
    (identifier) @definition))

(class_declaration
  name: (class_name
    (identifier) @definition))

(class_reference
  (identifier) @reference)

; Entity Relationship.
(er_entity_declaration
  name: (er_entity_name) @definition)

(er_relationship
  source: (er_entity_reference) @reference)

(er_relationship
  target: (er_entity_reference) @reference)

; Event Modeling.
(event_entity_statement
  name: (event_qualified_name) @definition)

(event_data_statement
  name: (event_data_name) @definition)

(event_frame_statement
  entity: (event_qualified_name) @reference)

(event_frame_statement
  data_reference: (event_data_reference
    name: (event_data_name) @reference))

; Flowchart.
(flow_vertex
  id: (flow_node_id) @definition
  shape: (_))

(flow_vertex
  id: (flow_node_id) @reference
  !shape)

(flow_class_assignment_statement
  targets: (flow_identifier_list
    item: (flow_reference) @reference))

(flow_style_statement
  target: (flow_node_id) @reference)

(flow_click_statement
  target: (flow_node_id) @reference)

; Gantt.
(gantt_task_statement
  metadata: (gantt_task_metadata
    (gantt_task_item
      value: (gantt_task_atom) @definition)))

(gantt_reference) @reference

; GitGraph.
(git_graph_branch_statement
  name: (git_graph_reference) @definition)

(git_graph_checkout_statement
  branch: (git_graph_reference) @reference)

(git_graph_merge_statement
  branch: (git_graph_reference) @reference)

; Radar.
(radar_axis
  name: (radar_identifier) @definition)

(radar_curve
  name: (radar_identifier) @definition)

(radar_detailed_entry
  axis: (radar_identifier) @reference)

; Railroad constructor dialect.
(railroad_rule
  name: (railroad_identifier) @definition)

(railroad_reference
  name: (railroad_string) @reference)

; Railroad ABNF.
(railroad_abnf_rule
  name: (railroad_abnf_rule_name) @definition)

(railroad_abnf_reference
  name: (railroad_abnf_rule_name) @reference)

; Railroad EBNF.
(railroad_ebnf_rule
  name: (railroad_ebnf_identifier) @definition)

(railroad_ebnf_reference
  name: (railroad_ebnf_identifier) @reference)

; Railroad PEG.
(railroad_peg_rule
  name: (railroad_peg_identifier) @definition)

(railroad_peg_reference
  name: (railroad_peg_identifier) @reference)

; Requirement.
[
  (requirement_declaration
    name: (requirement_name) @definition)
  (requirement_element_declaration
    name: (requirement_name) @definition)
]

(requirement_relationship_statement
  source: (requirement_reference) @reference)

(requirement_relationship_statement
  target: (requirement_reference) @reference)

; Sequence.
(sequence_participant_declaration
  name: (sequence_participant_name) @definition)

(sequence_actor_reference) @reference

; State.
(state_alias_clause
  name: (state_name) @definition)

[
  (state_named_declaration
    name: (state_name) @definition)
  (state_pseudostate_declaration
    name: (state_name) @definition)
  (state_composite_declaration
    name: (state_name) @definition)
]

(state_reference) @reference

; Swimlane.
(swimlane_vertex
  id: (swimlane_node_id) @definition
  shape: (_))

(swimlane_vertex
  id: (swimlane_node_id) @reference
  !shape)

(swimlane_class_assignment_statement
  targets: (swimlane_identifier_list
    item: (swimlane_reference) @reference))

(swimlane_style_statement
  target: (swimlane_node_id) @reference)

(swimlane_click_statement
  target: (swimlane_node_id) @reference)

; Venn.
(venn_set_statement
  expression: (venn_set_expression
    set: (venn_identifier) @definition))

(venn_intersection_expression
  set: (venn_identifier) @reference)

; Wardley.
[
  (wardley_component_statement
    name: (wardley_name) @definition)
  (wardley_anchor_statement
    name: (wardley_name) @definition)
]

(wardley_link_statement
  source: (wardley_name) @reference)

(wardley_link_statement
  target: (wardley_name) @reference)

(wardley_evolve_statement
  component: (wardley_name) @reference)

; ZenUML.
[
  (zenuml_starter_declaration
    participant: (zenuml_name) @definition)
  (zenuml_participant_declaration
    name: (zenuml_name) @definition)
]

(zenuml_assignment
  assignee: (zenuml_assignee
    item: (zenuml_identifier) @definition))

(zenuml_endpoint
  name: (zenuml_name) @reference)
