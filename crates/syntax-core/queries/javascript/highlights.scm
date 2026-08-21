; JavaScript highlights.scm — adapted from tree-sitter-javascript
; 0.25.0's queries/highlights.scm and highlights-jsx.scm (MIT,
; (c) 2017 Max Brunsfeld). Predicate-guarded patterns (@constructor /
; @constant / @variable.builtin by name regex, `require`) are dropped:
; this crate's highlighter does not evaluate query predicates, so an
; unevaluated guard would mislabel every identifier.

; Variables and properties

(identifier) @variable
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

; JSX — adapted from tree-sitter-javascript's queries/highlights-jsx.scm.
; The upstream `#match?`-guarded lowercase-tag rule is replaced by an
; unguarded one: predicates are not evaluated here, so component names
; simply get @tag as well.

(jsx_opening_element (identifier) @tag)
(jsx_closing_element (identifier) @tag)
(jsx_self_closing_element (identifier) @tag)
(jsx_attribute (property_identifier) @attribute)
(jsx_opening_element ["<" ">"] @punctuation.bracket)
(jsx_closing_element ["</" ">"] @punctuation.bracket)
(jsx_self_closing_element ["<" "/>"] @punctuation.bracket)
