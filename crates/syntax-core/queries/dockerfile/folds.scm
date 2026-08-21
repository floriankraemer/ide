; Foldable regions (Task C). A Dockerfile has no nesting: the only
; multi-line constructs are a `RUN` continued with backslashes, a heredoc,
; and a JSON-array form of `CMD`/`ENTRYPOINT`. There is no node for a build
; stage — the grammar emits a flat list of instructions — so a stage cannot
; be folded as a unit.
(run_instruction) @fold
(heredoc_block) @fold
(json_string_array) @fold
