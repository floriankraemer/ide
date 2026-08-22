; Injected regions (I1), Markdown block grammar — adapted from
; tree-sitter-md 0.5.3's own
; `tree-sitter-markdown/queries/injections.scm` (MIT).
;
; `tree-sitter-md` is two grammars: this one sees blocks and leaves every
; run of prose as one opaque `(inline)` node, which the last pattern here
; hands to the `markdown_inline` catalog row. That row is the reason
; `markdown_inline` exists and the only way it is ever reached — it claims
; no extension.
;
; A fenced block names its own language, so the language comes from an
; `@injection.language` capture rather than a `#set!` directive. The
; capture is the `(language)` node inside `(info_string)`, so ```rust,
; ```rs and ```RUST all resolve — see `injection_language_alias` in
; lib.rs, which normalises the common fence aliases onto registry ids.
; The content capture is `(code_fence_content)`, never the whole
; `(fenced_code_block)`: capturing the block would feed the ``` fences to
; the injected parser.
;
; The upstream `latex` pattern is dropped — there is no LaTeX row in the
; catalog, so it could only ever resolve to nothing.

(fenced_code_block
  (info_string
    (language) @injection.language)
  (code_fence_content) @injection.content)

((html_block) @injection.content
  (#set! injection.language "html"))

((minus_metadata) @injection.content
  (#set! injection.language "yaml"))

((plus_metadata) @injection.content
  (#set! injection.language "toml"))

((inline) @injection.content
  (#set! injection.language "markdown_inline"))
