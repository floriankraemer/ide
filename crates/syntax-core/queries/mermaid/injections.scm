; Injection query for Mermaid, ported unchanged from tree-sitter-mermaid
; 0.1.0's `queries/portable/injections.scm` (MIT).
;
; Two real injections: an Event Modeling data block declares its own
; payload language, and an XY chart's Markdown text is Markdown. All three
; injected ids (`json`, `markdown`, `html`) are registered languages in
; this catalog, so each resolves rather than falling back to plain text.

; Portable injections require delimiter-free payload nodes and standard query
; predicates. Families that require editor-specific offsets remain N/A.

; Event Modeling typed data blocks.
((event_data_block
  type: (event_data_type
    kind: (event_data_type_name) @_event_language)
  content: [
    (event_data_fragment)
    (event_nested_data_block)
  ] @injection.content)
  (#eq? @_event_language "json")
  (#set! injection.language "json")
  (#set! injection.combined))

((event_data_block
  type: (event_data_type
    kind: (event_data_type_name) @_event_language)
  content: [
    (event_data_fragment)
    (event_nested_data_block)
  ] @injection.content)
  (#eq? @_event_language "md")
  (#set! injection.language "markdown")
  (#set! injection.combined))

((event_data_block
  type: (event_data_type
    kind: (event_data_type_name) @_event_language)
  content: [
    (event_data_fragment)
    (event_nested_data_block)
  ] @injection.content)
  (#eq? @_event_language "html")
  (#set! injection.language "html")
  (#set! injection.combined))

; XY Chart Markdown text already exposes its delimiters separately.
((xy_chart_markdown_text
  [
    (xy_chart_markdown_content)
    (xy_chart_markdown_backtick_content)
  ] @injection.content)
  (#set! injection.language "markdown")
  (#set! injection.combined))
