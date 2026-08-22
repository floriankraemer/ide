; Haskell highlights.scm — adapted from tree-sitter-haskell 0.23.1's own
; queries/highlights.scm (MIT, https://github.com/tree-sitter/tree-sitter-haskell).
; Upstream has seventeen patterns guarded by a predicate (eighteen predicate
; calls — one pattern carries two `#eq?`s; none uses `#match?`). Those
; predicates ARE evaluated here (see queries/go/highlights.scm), and twelve
; of the seventeen are ported below. Same-range captures resolve
; first-pattern-wins in this crate while upstream assumes last-match-wins,
; so a ported pattern sits ABOVE the broader one it is meant to beat rather
; than where upstream puts it.
;
; The five still absent, and why:
;
;   * the two `#eq? @_name @variable` / `#eq? @_name @function`
;     signature-followed-by-declaration patterns (upstream lines 148 and
;     164): their body carries a stray `match: (_)` at group level, and the
;     first of the two only restates `(decl/bind name: (variable)
;     @variable)`, which this file already has;
;   * the composition-defined function at upstream line 290 — the plain
;     `(decl name: (variable) @function)` above already paints that name;
;   * the two `(#eq? @_name "qq")` quasi-quote patterns, which fire only for
;     a quoter literally named `qq`.
;
; A ported pattern keeps upstream's underscore captures (`@_op`, `@_name`,
; `@_type`) exactly as written. An underscore capture is the tree-sitter
; convention for a name that exists only as a predicate operand: it resolves
; to no scope and paints nothing, and the pattern's *other* capture is the one
; that gets coloured. Renaming `@_op` to `@operator` would paint every `$` and
; `<$>` in the file, which is the opposite of what these patterns do.
;
; Upstream's `(qualified_module (module) @constructor)` pattern stays
; commented out below, carrying upstream's own `; TODO broken, also huh?`
; marker: it is dead upstream too, and nothing here fixed it.
;
; Also absent: upstream's bare `(variable) @type` catch-all. Upstream pairs
; it with `(variable) @variable` and relies on last-match-wins to let `@type`
; claim every variable no earlier pattern took; first-pattern-wins cannot
; reproduce that layering, and painting every leftover variable as a type is
; wrong. Only the `@variable` half is kept, last in the file, where every
; specific pattern above still beats it.

; ----------------------------------------------------------------------------
; Parameters and variables
; NOTE: These are at the top, so that they have low priority,
; and don't override destructured parameters
(pattern/wildcard) @variable

(decl/function
  patterns: (patterns
    (_) @variable.parameter))

(expression/lambda
  (_)+ @variable.parameter
  "->")

(decl/function
  (infix
    (pattern) @variable.parameter))

; ----------------------------------------------------------------------------
; Name tests
; A `(variable)` node and the `(expression/variable)` wrapping it cover the
; same byte range, and equal ranges resolve first-pattern-wins — so these
; sit above the function/variable patterns instead of at the bottom of the
; file, which is where upstream (last-match-wins) puts them.

; True or False
((constructor) @boolean
  (#any-of? @boolean "True" "False"))

; otherwise (= True)
((variable) @boolean
  (#eq? @boolean "otherwise"))

; Exceptions/error handling
((variable) @keyword.exception
  (#any-of? @keyword.exception
    "error" "undefined" "try" "tryJust" "tryAny" "catch" "catches" "catchJust" "handle" "handleJust"
    "throw" "throwIO" "throwTo" "throwError" "ioError" "mask" "mask_" "uninterruptibleMask"
    "uninterruptibleMask_" "bracket" "bracket_" "bracketOnErrorSource" "finally" "fail"
    "onException" "expectationFailure"))

; Debugging
((variable) @keyword.debug
  (#any-of? @keyword.debug
    "trace" "traceId" "traceShow" "traceShowId" "traceWith" "traceShowWith" "traceStack" "traceIO"
    "traceM" "traceShowM" "traceEvent" "traceEventWith" "traceEventIO" "flushEventLog" "traceMarker"
    "traceMarkerIO"))

; ----------------------------------------------------------------------------
; Literals and comments
(integer) @number

(negation) @number

(expression/literal
  (float)) @number.float

(char) @character

(string) @string

(unit) @string.special.symbol ; unit, as in ()

(comment) @comment

((haddock) @comment.documentation)

; ----------------------------------------------------------------------------
; Punctuation
[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket

[
  ","
  ";"
] @punctuation.delimiter

; ----------------------------------------------------------------------------
; Keywords, operators, includes
[
  "forall"
  ; "∀" ; utf-8 is not cross-platform safe
] @keyword.repeat

(pragma) @keyword.directive

[
  "if"
  "then"
  "else"
  "case"
  "of"
] @keyword.conditional

[
  "import"
  "qualified"
  "module"
] @keyword.import

[
  (operator)
  (constructor_operator)
  (all_names)
  (wildcard)
  "."
  ".."
  "="
  "|"
  "::"
  "=>"
  "->"
  "<-"
  "\\"
  "`"
  "@"
] @operator

; TODO broken, also huh?
; ((qualified_module
;   (module) @constructor)
;   .
;   (module))

(module
  (module_id) @module)

[
  "where"
  "let"
  "in"
  "class"
  "instance"
  "pattern"
  "data"
  "newtype"
  "family"
  "type"
  "as"
  "hiding"
  "deriving"
  "via"
  "stock"
  "anyclass"
  "do"
  "mdo"
  "rec"
  "infix"
  "infixl"
  "infixr"
] @keyword

; ----------------------------------------------------------------------------
; Functions and variables
(decl
  [
   name: (variable) @function
   names: (binding_list (variable) @function)
  ])

; main is always a function
; (this prevents `main = undefined` from being highlighted as a variable)
(decl/bind
  name: (variable) @function
  (#eq? @function "main"))

; a bind whose preceding signature has a function type is a function
((decl/signature
  name: (variable) @_name
  type: (quantified_type))
  .
  (decl/bind
    (variable) @function)
  (#eq? @function @_name))

; a type that involves 'IO' makes the signature a function, not a value
(decl/signature
  name: (variable) @function
  type: (type/apply
    constructor: (name) @_type)
  (#eq? @_type "IO"))

(decl/bind
  name: (variable) @variable)

; Consider signatures (and accompanying functions)
; with only one value on the rhs as variables
(decl/signature
  name: (variable) @variable
  type: (type))

; Upstream's "signature followed by a function is a function" pattern
; captured the whole `decl/signature` node as `@function`, painting one
; span across the name, the `::` and the entire type. Only the following
; declaration's name is kept.
((decl/signature)
  .
  (decl/function
    name: (variable) @function))

(decl/bind
  name: (variable) @function
  (match
    expression: (expression/lambda)))

; view patterns
(view_pattern
  [
    (expression/variable) @function.call
    (expression/qualified
      (variable) @function.call)
  ])

; consider infix functions as operators
(infix_id
  [
    (variable) @operator
    (qualified
      (variable) @operator)
  ])

; decl/function calls with an infix operator
; e.g. func <$> a <*> b
(infix
  [
    (variable) @function.call
    (qualified
      ((module) @module
        (variable) @function.call))
  ]
  .
  (operator))

; decl/function calls with infix operators
([
    (expression/variable) @function.call
    (expression/qualified
      (variable) @function.call)
  ]
  .
  (operator) @_op
  (#any-of? @_op "$" "<$>" ">>=" "=<<"))

; right hand side of infix operator
((infix
  [
    (operator)
    (infix_id (variable))
  ] ; infix or `func`
  .
  [
    (variable) @function.call
    (qualified
      (variable) @function.call)
  ])
  .
  (operator) @_op
  (#any-of? @_op "$" "<$>" "=<<"))

; decl/function composition, arrows, monadic composition (lhs)
(
  [
    (expression/variable) @function
    (expression/qualified
      (variable) @function)
  ]
  .
  (operator) @_op
  (#any-of? @_op "." ">>>" "***" ">=>" "<=<"))

; right hand side of infix operator
((infix
  [
    (operator)
    (infix_id (variable))
  ] ; infix or `func`
  .
  [
    (variable) @function
    (qualified
      (variable) @function)
  ])
  .
  (operator) @_op
  (#any-of? @_op "." ">>>" "***" ">=>" "<=<"))

; function composition, arrows, monadic composition (rhs)
((operator) @_op
  .
  [
    (expression/variable) @function
    (expression/qualified
      (variable) @function)
  ]
  (#any-of? @_op "." ">>>" "***" ">=>" "<=<"))

; infix operators applied to variables
((expression/variable) @variable
  .
  (operator))

((operator)
  .
  [
    (expression/variable) @variable
    (expression/qualified
      (variable) @variable)
  ])

(apply
  [
    (expression/variable) @function.call
    (expression/qualified
      (variable) @function.call)
  ])

; function compositions, in parentheses, applied
; lhs
(apply
  .
  (expression/parens
    (infix
      [
        (variable) @function.call
        (qualified
          (variable) @function.call)
      ]
      .
      (operator))))

; rhs
(apply
  .
  (expression/parens
    (infix
      (operator)
      .
      [
        (variable) @function.call
        (qualified
          (variable) @function.call)
      ])))

; variables being passed to a function call
(apply
  (_)
  .
  [
    (expression/variable) @variable
    (expression/qualified
      (variable) @variable)
  ])

; scoped function types (func :: a -> b)
(signature
  pattern: (pattern/variable) @function
  type: (quantified_type))

; signatures that have a function type
; + binds that follow them
(decl/signature
  name: (variable) @function
  type: (quantified_type))

; ----------------------------------------------------------------------------
; Types
(name) @type

(type/star) @type


(constructor) @constructor

; ----------------------------------------------------------------------------
; Quasi-quotes
(quoter) @function.call

; namespaced quasi-quoter
(quasiquote
  (_
    (module) @module
    .
    (variable) @function.call))

; Highlighting of quasiquote_body for other languages is handled by injections.scm
; ----------------------------------------------------------------------------
; Fields

(field_name
  (variable) @variable.member)

(import_name
  (name)
  .
  (children
    (variable) @variable.member))


; ----------------------------------------------------------------------------
; Every variable no pattern above claimed. Last on purpose: equal-range
; captures resolve first-pattern-wins, so this cannot steal a node from the
; function/parameter/member patterns above it.
(variable) @variable

; ----------------------------------------------------------------------------
; Spell checking
(comment) @spell
