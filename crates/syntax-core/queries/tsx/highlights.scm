; TSX highlights.scm — TypeScript plus JSX. TSX is a separate grammar
; in tree-sitter-typescript, so it needs its own compiled query; the
; JavaScript and TypeScript halves are included directly for the same
; reason typescript/highlights.scm includes them (no `inherits:
; javascript` support). See javascript/highlights.scm for licence.

; Variables and properties

(property_identifier) @property
(this) @variable.builtin
(super) @variable.builtin

; Function and method definitions

(function_expression name: (identifier) @function)
(function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @function.method)

(pair
  key: (property_identifier) @function.method
  value: [(function_expression) (arrow_function)])

(variable_declarator
  name: (identifier) @function
  value: [(function_expression) (arrow_function)])

; Function and method calls

(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (member_expression
    property: (property_identifier) @function.method))

; Literals

[
  (true)
  (false)
  (null)
  (undefined)
] @constant.builtin

(comment) @comment

[
  (string)
  (template_string)
] @string

(regex) @string.regexp
(number) @number
(escape_sequence) @escape

; Tokens

[
  ";"
  (optional_chain)
  "."
  ","
] @punctuation.delimiter

[
  "-" "--" "-=" "+" "++" "+=" "*" "*=" "**" "**=" "/" "/=" "%" "%="
  "<" "<=" "<<" "<<=" "=" "==" "===" "!" "!=" "!==" "=>" ">" ">=" ">>"
  ">>=" ">>>" ">>>=" "~" "^" "&" "|" "^=" "&=" "|=" "&&" "||" "??"
  "&&=" "||=" "??="
] @operator

[ "(" ")" "[" "]" "{" "}" ] @punctuation.bracket

(template_substitution
  "${" @punctuation.special
  "}" @punctuation.special)

[
  "as"
  "async"
  "await"
  "break"
  "case"
  "catch"
  "class"
  "const"
  "continue"
  "debugger"
  "default"
  "delete"
  "do"
  "else"
  "export"
  "extends"
  "finally"
  "for"
  "from"
  "function"
  "get"
  "if"
  "import"
  "in"
  "instanceof"
  "let"
  "new"
  "of"
  "return"
  "set"
  "static"
  "switch"
  "target"
  "throw"
  "try"
  "typeof"
  "var"
  "void"
  "while"
  "with"
  "yield"
] @keyword

; TypeScript additions — adapted from tree-sitter-typescript 0.23.2's
; queries/highlights.scm (MIT, (c) 2017 GitHub). Upstream's
; `; inherits: javascript` directive is not implemented by this crate's
; query loader, so the JavaScript half above is included verbatim instead.
; Upstream's `((identifier) @type (#match? @type "^[A-Z]"))` is
; deliberately not ported. It captures exactly the same nodes as the
; @constructor rule in the naming-conventions block at the end of this
; file, so with same-node captures resolving first-pattern-wins the two
; cannot coexist: one of them is simply dead. @constructor is the better
; of the two here, because real types already have their own node
; ((type_identifier) @type, above) and what is left for the (identifier)
; fallback is capitalized names in value position — classes being
; constructed or called.

(type_identifier) @type
(predefined_type) @type.builtin

(type_arguments
  "<" @punctuation.bracket
  ">" @punctuation.bracket)

(required_parameter (identifier) @variable.parameter)
(optional_parameter (identifier) @variable.parameter)

[
  "abstract"
  "declare"
  "enum"
  "implements"
  "interface"
  "keyof"
  "namespace"
  "private"
  "protected"
  "public"
  "type"
  "readonly"
  "override"
  "satisfies"
] @keyword

; JSX — adapted from tree-sitter-javascript's queries/highlights-jsx.scm.
; The upstream `#match?`-guarded lowercase-tag rule is replaced by an
; unguarded one, so component names get @tag as well. The guard would
; work — `#match?` is evaluated (see queries/go/highlights.scm) — it has
; just not been ported back.

(jsx_opening_element (identifier) @tag)
(jsx_closing_element (identifier) @tag)
(jsx_self_closing_element (identifier) @tag)
(jsx_attribute (property_identifier) @attribute)
(jsx_opening_element ["<" ">"] @punctuation.bracket)
(jsx_closing_element ["</" ">"] @punctuation.bracket)
(jsx_self_closing_element ["<" "/>"] @punctuation.bracket)

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

; CamelCase is a constructor.
((identifier) @constructor
  (#match? @constructor "^[A-Z]"))

; The catch-all, last of all: with same-node captures resolving
; first-pattern-wins, every pattern above — including the two
; conventions — beats it, and it only reaches the identifiers nothing
; else claimed.
(identifier) @variable
