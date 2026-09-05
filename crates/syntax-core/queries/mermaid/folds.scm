; Foldable regions (Task C), Mermaid. Hand-written: tree-sitter-mermaid
; ships no folds query.
;
; A Mermaid file is one diagram whose body is everything after the header
; line, so folding the body is what collapses a diagram down to the line
; that says what it is — the same "one section, one fold" shape
; markdown/folds.scm has. Every diagram family in the grammar has its own
; body node rather than a shared one, hence the list; the nested regions
; worth folding on their own (a flowchart subgraph, a class body, a C4
; boundary) follow it.
[
  (architecture_body)
  (block_body)
  (c4_body)
  (c4_boundary_body)
  (class_body)
  (class_namespace_body)
  (cynefin_body)
  (entity_relationship_body)
  (event_modeling_body)
  (flow_body)
  (gantt_body)
  (git_graph_body)
  (info_body)
  (ishikawa_body)
  (journey_body)
  (kanban_body)
  (mindmap_body)
  (packet_body)
  (pie_body)
  (quadrant_chart_body)
  (radar_body)
  (railroad_abnf_body)
  (railroad_body)
  (railroad_ebnf_body)
  (railroad_peg_body)
  (requirement_body)
  (sankey_body)
  (sequence_body)
  (state_body)
  (swimlane_body)
  (timeline_body)
  (tree_view_body)
  (treemap_body)
  (venn_body)
  (wardley_body)
  (wardley_pipeline_body)
  (xy_chart_body)
  (zenuml_body)
] @fold

[
  (flow_subgraph)
  (swimlane_subgraph)
  (class_member_block)
] @fold
