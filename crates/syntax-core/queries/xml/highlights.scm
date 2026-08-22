; XML highlights.scm — adapted from tree-sitter-xml 0.7.0's own
; `queries/xml/highlights.scm` (MIT,
; https://github.com/tree-sitter-grammars/tree-sitter-xml).
;
; Upstream's `#any-of?`-guarded `(EntityRef) @constant.builtin` pattern is
; ported back below. `#any-of?` is evaluated (see
; queries/go/highlights.scm) and same-node captures resolve
; first-pattern-wins, so it is placed *above* the general
; `(EntityRef) @constant` rule — upstream puts it after, relying on the
; opposite convention — and the five predefined entities get their own
; colour.
;
; Capture names are otherwise left exactly as upstream writes them, on
; purpose — keeping the file close to upstream keeps it updatable. They all
; resolve: `@markup`, `@markup.link`, `@markup.raw` and `@markup.heading`
; are in `syntax_core::SCOPES` outright, and `@string.special.symbol`
; resolves by `Scope::resolve` walking the dotted name up to
; `string.special`. (An earlier revision of this note also listed `@error`
; as an unknown name in use here; it never appears in this file, and any
; capture on it would produce no span at all rather than a wrong colour.)
;
; XML needs no `no-scopes.txt`: `<?xml …?>` gives a real `@keyword`,
; attribute values give strings and `<!-- … -->` gives comments.


;; XML declaration

"xml" @keyword

[ "version" "encoding" "standalone" ] @property

(EncName) @string.special

(VersionNum) @number

[ "yes" "no" ] @boolean

;; Processing instructions

(PI) @embedded

(PI (PITarget) @keyword)

;; Element declaration

(elementdecl
  "ELEMENT" @keyword
  (Name) @tag)

(contentspec
  (_ (Name) @property))

"#PCDATA" @type.builtin

[ "EMPTY" "ANY" ] @string.special.symbol

[ "*" "?" "+" ] @operator

;; Entity declaration

(GEDecl
  "ENTITY" @keyword
  (Name) @constant)

(GEDecl (EntityValue) @string)

(NDataDecl
  "NDATA" @keyword
  (Name) @label)

;; Parsed entity declaration

(PEDecl
  "ENTITY" @keyword
  "%" @operator
  (Name) @constant)

(PEDecl (EntityValue) @string)

;; Notation declaration

(NotationDecl
  "NOTATION" @keyword
  (Name) @constant)

(NotationDecl
  (ExternalID
    (SystemLiteral (URI) @string.special)))

;; Attlist declaration

(AttlistDecl
  "ATTLIST" @keyword
  (Name) @tag)

(AttDef (Name) @property)

(AttDef (Enumeration (Nmtoken) @string))

(DefaultDecl (AttValue) @string)

[
  (StringType)
  (TokenizedType)
] @type.builtin

(NotationType "NOTATION" @type.builtin)

[
  "#REQUIRED"
  "#IMPLIED"
  "#FIXED"
] @attribute

;; Entities

((EntityRef) @constant.builtin
 (#any-of? @constant.builtin
   "&amp;" "&lt;" "&gt;" "&quot;" "&apos;"))

(EntityRef) @constant

(CharRef) @constant

(PEReference) @constant

;; External references

[ "PUBLIC" "SYSTEM" ] @keyword

(PubidLiteral) @string.special

(SystemLiteral (URI) @markup.link)

;; Processing instructions

(XmlModelPI "xml-model" @keyword)

(StyleSheetPI "xml-stylesheet" @keyword)

(PseudoAtt (Name) @property)

(PseudoAtt (PseudoAttValue) @string)

;; Doctype declaration

(doctypedecl "DOCTYPE" @keyword)

(doctypedecl (Name) @type)

;; Tags

(STag (Name) @tag)

(ETag (Name) @tag)

(EmptyElemTag (Name) @tag)

;; Attributes

(Attribute (Name) @property)

(Attribute (AttValue) @string)

;; Delimiters & punctuation

[
 "<?" "?>"
 "<!" "]]>"
 "<" ">"
 "</" "/>"
] @punctuation.delimiter

[ "(" ")" "[" "]" ] @punctuation.bracket

[ "\"" "'" ] @punctuation.delimiter

[ "," "|" "=" ] @operator

;; Text

(CharData) @markup

(CDSect
  (CDStart) @markup.heading
  (CData) @markup.raw
  "]]>" @markup.heading)

;; Misc

(Comment) @comment
