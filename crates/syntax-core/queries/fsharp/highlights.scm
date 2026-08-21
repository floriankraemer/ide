; F# highlights.scm — adapted from tree-sitter-fsharp 0.3.11's own
; queries/highlights.scm (MIT, https://github.com/ionide/tree-sitter-fsharp).
; Eleven upstream patterns guarded by `#match?`/`#eq?` predicates are
; still absent: not because this crate leaves predicates unevaluated (it
; does not — see queries/go/highlights.scm) but because they have not been
; ported back. The naming-conventions block at the end of this file is the
; part that has.
;
; The queries are written against the implementation grammar
; (`LANGUAGE_FSHARP`); the crate's second grammar, for `.fsi` signature
; files, is not registered — see the catalog row for why.

;; ----------------------------------------------------------------------------
;; Literals and comments

[
  (line_comment)
  (xml_doc)
  (block_comment)
] @comment @spell

(xml_doc) @comment.documentation @spell

; Upstream captures `(const [(_) @constant (unit) @constant.builtin])`.
; The `(_)` alternative is a wildcard over every literal, so it stacked a
; `constant` span underneath every string and number — the same problem a
; catch-all `(identifier) @variable` causes. Only the `unit` alternative
; is kept.
(const (unit) @constant.builtin)

(primary_constr_args (_) @variable.parameter)

(class_as_reference
  (_) @variable.parameter.builtin)

;; ----------------------------------------------------------------------------
;; Punctuation

(type_name type_name: (_) @type.definition)
(exception_definition exception_name: (_) @type.definition)

[
 (_type)
 (atomic_type)
] @type

(member_signature
  .
  (identifier) @function.member
  (curried_spec
    (arguments_spec
      "*"* @operator
      (argument_spec
        (argument_name_spec
          "?"? @character.special
          name: (_) @variable.parameter)))))

(union_type_case (identifier) @constant)

(rules
  (rule
    pattern: (_) @constant
    block: (_)))

(wildcard_pattern) @character.special

(identifier_pattern
  .
  (_) @constant
  .
  (_) @variable)

(optional_pattern
  "?" @character.special)

(fsi_directive_decl . (string) @module)

(import_decl . (_) @module)
(named_module
  name: (_) @module)
(namespace
  name: (_) @module)
(module_defn
  .
  (_) @module)

(ce_expression
  .
  (_) @constant.macro)

(field_initializer
  field: (_) @property)

(record_fields
  (record_field
    .
    (identifier) @property))

(value_declaration_left . (_) @variable)

(function_declaration_left
  . (_) @function)

; Upstream's `(argument_patterns) @variable.parameter` is dropped: the node
; is the whole parenthesized parameter *group*, so it painted one span
; across the parameter names, their type annotations and the parentheses.
; The `(typed_pattern ...)` pattern below captures the individual
; parameters instead.
(typed_pattern
  (_pattern) @variable.parameter
  (_type) @type)

;; A member name has two mutually-exclusive shapes, matched separately so the
;; highlight is deterministic regardless of tree-sitter's alternation-match order
;; (0.26.11 changed which branch of an overlapping `[...]` wins for a node that
;; matched both). Bare `member M(x)` -> M is the method; instance `member this.M`
;; -> `this` is the self parameter, M the method.
(member_defn
  (method_or_prop_defn
    (property_or_ident . (identifier) @function .)
    args: (_)* @variable.parameter))

(member_defn
  (method_or_prop_defn
    (property_or_ident
      instance: (identifier) @variable.parameter.builtin
      method: (identifier) @function.method)
    args: (_)* @variable.parameter))


(dot_expression
  base: (_) @variable.member
  field: (long_identifier_or_op
    (identifier) @property))

(application_expression
  .
  (long_identifier_or_op
    (identifier) @function.call)
  .
  (_))

(application_expression
  .
  (dot_expression
    field: (long_identifier_or_op
      (identifier) @function.call))
  .
  (_))

(application_expression
  .
  (typed_expression
    (long_identifier_or_op
      (identifier) @function.call)
    (_))
  .
  (_))

(application_expression
  .
  (typed_expression
    (dot_expression
      field: (long_identifier_or_op
        (identifier) @function.call))
    (_))
  .
  (_))

[
  (xint)
  (int)
  (int16)
  (uint16)
  (int32)
  (uint32)
  (int64)
  (uint64)
  (nativeint)
  (unativeint)
] @number

[
  (ieee32)
  (ieee64)
  (float)
  (decimal)
] @number.float

(bool) @boolean

([
  (string)
  (triple_quoted_string)
  (verbatim_string)
  (char)
  (format_string)
  (format_triple_quoted_string)
] @spell @string)

(compiler_directive_decl) @keyword.directive

(preproc_line
  "#line" @keyword.directive)

(attribute
  target: (identifier)? @keyword
  (_type) @attribute)

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
  "[|"
  "|]"
  "{|"
  "|}"
] @punctuation.bracket

[
  "[<"
  ">]"
] @punctuation.special

(format_string_eval
  [
    "{"
    "}"
  ] @punctuation.special)

[
  ","
  ";"
  ":"
  "."
] @punctuation.delimiter

[
  "|"
  "="
  ">"
  "<"
  "-"
  "~"
  "->"
  "<-"
  "&"
  "&&"
  "|"
  "||"
  ":>"
  ":?>"
  ".."
  (infix_op)
  (prefix_op)
  (op_identifier)
] @operator

(generic_type
  [
   "<"
   ">"
  ] @punctuation.bracket)

(typed_expression
  ">" @punctuation.bracket)

[
  "if"
  "then"
  "else"
  "elif"
  "when"
  "match"
  "match!"
] @keyword.conditional

[
  "and"
  "or"
  "not"
  "upcast"
  "downcast"
] @keyword.operator

[
  "return"
  "return!"
  "yield"
  "yield!"
] @keyword.return

[
  "for"
  "while"
  "downto"
  "to"
] @keyword.repeat


[
  "open"
  "#r"
  "#load"
] @keyword.import

[
  "abstract"
  "delegate"
  "extern"
  "static"
  "inline"
  "mutable"
  "override"
  "rec"
  "global"
  (access_modifier)
] @keyword.modifier

[
  "let"
  "let!"
  "use"
  "use!"
  "and!"
  "member"
] @keyword.function

[
  "enum"
  "type"
  "exception"
  "inherit"
  "interface"
  "and"
  "class"
  "struct"
] @keyword.type

;; `query { ... }` custom operations whose names are unambiguous (they are not
;; ordinary F# functions/values). Common-named operations (zip, head, count,
;; where, select, sortBy, groupBy, ...) are intentionally omitted: they cannot
;; be scoped to a `query` builder with the current query language without
;; highlighting the same names everywhere.

;; Operations that take an argument: matched only as an application head, so
;; module-qualified names (List.sortByDescending) and member access on an
;; expression ((expr).sortByDescending) are left untouched.

;; Zero-argument terminal operations: matched only as a bare statement inside a
;; computation-expression body, which likewise excludes member access.

[
  "as"
  "assert"
  "begin"
  "end"
  "done"
  "default"
  "in"
  "do"
  "do!"
  "fun"
  "function"
  "get"
  "set"
  "lazy"
  "new"
  "of"
  "struct"
  "val"
  "module"
  "namespace"
  "with"
] @keyword

[
  "null"
] @constant.builtin

(match_expression "with" @keyword.conditional)

(try_expression
  [
    "try"
    "with"
    "finally"
  ] @keyword.exception)

(preproc_if
  [
    "#if" @keyword.directive
    "#endif" @keyword.directive
  ]
  condition: (_)? @keyword.directive)

(preproc_else
  "#else" @keyword.directive)

; Inactive branch of a directive the grammar could not place structurally;
; render like a comment, the way C/C++ editors gray out inactive regions.
(preproc_inactive) @comment

((long_identifier
  (identifier)+ @variable.member
  .
  (identifier)))

; --- Naming conventions -----------------------------------------------
;
; Guarded by `#match?` text predicates, which `QueryCursor::matches` does
; evaluate (see `spans_from_tree`). They sit last on purpose: captures on
; the same node resolve first-pattern-wins, so every specific pattern
; above still beats these catch-alls.

; SCREAMING_CASE is a constant. Two characters minimum, so a bare
; `T` stays a type rather than becoming a constant.
((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))

; CamelCase is a type.
((identifier) @type
  (#match? @type "^[A-Z]"))
