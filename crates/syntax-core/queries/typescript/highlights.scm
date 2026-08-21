; TypeScript highlights.scm. The JavaScript half is included directly
; because upstream's `; inherits: javascript` directive is not
; implemented by this crate's query loader. See javascript/highlights.scm
; for the source and licence of the JavaScript patterns.

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

; TypeScript additions — adapted from tree-sitter-typescript 0.23.2's
; queries/highlights.scm (MIT, (c) 2017 GitHub). Upstream's
; `; inherits: javascript` directive is not implemented by this crate's
; query loader, so the JavaScript half above is included verbatim instead.
; The upstream `#match? "^[A-Z]"` rule that types capitalized identifiers
; is dropped: predicates are not evaluated here and it would type every
; identifier.

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
