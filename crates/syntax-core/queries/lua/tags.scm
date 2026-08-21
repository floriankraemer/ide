; Outline extraction query for Task D (`outline()`), Lua. Same
; `tree-sitter-tags` convention as rust/tags.scm.
;
; Lua has no class syntax — `function M.foo()` and `function obj:foo()` are
; the whole vocabulary — so the outline is functions and methods only.
(function_declaration name: (identifier) @name) @definition.function
(function_declaration
  name: (dot_index_expression field: (identifier) @name)) @definition.function
(function_declaration
  name: (method_index_expression method: (identifier) @name)) @definition.method
