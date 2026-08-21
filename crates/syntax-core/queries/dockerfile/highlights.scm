; Dockerfile highlights.scm — adapted from tree-sitter-containerfile 0.9.2's
; own queries/highlights.scm (MIT, https://github.com/wharflab/tree-sitter-containerfile,
; derived from camdencheek/tree-sitter-dockerfile, MIT (c) 2021 Camden Cheek).
;
; Two systematic changes, the same ones go/highlights.scm documents:
;
;   * the `((variable) @constant (#match? ...))` pattern is dropped, because
;     `spans_from_tree` does not evaluate predicates and the guard would
;     ship unevaluated, painting every expansion as a constant;
;   * the decorative `@spell` capture on `(comment)` is dropped, and
;     `@label` on heredoc markers becomes `@string.special` — the standard
;     name in `syntax_core::SCOPES` for a delimiter that belongs to a
;     string.

[
  "FROM"
  "AS"
  "RUN"
  "CMD"
  "LABEL"
  "EXPOSE"
  "ENV"
  "ADD"
  "COPY"
  "ENTRYPOINT"
  "VOLUME"
  "USER"
  "WORKDIR"
  "ARG"
  "ONBUILD"
  "STOPSIGNAL"
  "HEALTHCHECK"
  "SHELL"
  "MAINTAINER"
  "CROSS_BUILD"
] @keyword

[
  ":"
  "@"
] @operator

(comment) @comment

(image_spec
  (image_tag
    ":" @punctuation.special)
  (image_digest
    "@" @punctuation.special))

[
  (double_quoted_string)
  (single_quoted_string)
  (json_string)
] @string

(heredoc_block) @string

[
  (heredoc_marker)
  (heredoc_end)
] @string.special

(escape_sequence) @string.escape

(expansion
  [
    "$"
    "{"
    "}"
  ] @punctuation.special
)

(expansion_operator) @operator

(arg_pair
  name: (unquoted_string) @property)

(env_pair
  name: (unquoted_string) @property)

(label_pair
  key: (_) @property)

(param
  name: (_) @property)

(mount_param
  name: (_) @property)

(mount_param_param) @property

(expose_port) @number
