; XML highlights.scm — adapted from tree-sitter-xml 0.7.0's own
; `queries/xml/highlights.scm` (MIT,
; https://github.com/tree-sitter-grammars/tree-sitter-xml).
;
; One upstream pattern is still absent:
;
;   ((EntityRef) @constant.builtin
;    (#any-of? @constant.builtin "&amp;" "&lt;" ...))
;
; `#any-of?` is evaluated (see queries/go/highlights.scm), so it would
; match only the entities it lists, and — since same-node captures resolve
; first-pattern-wins — placing it before the `(EntityRef) @constant`
; pattern below would give the builtins their own colour. It has simply
; not been ported back, so the general `@constant` capture carries every
; entity reference and the distinction is lost, which costs a shade of
; colour.
;
; Capture names upstream uses that `syntax_core::SCOPES` does not know
; (`@markup`, `@markup.link`, `@markup.raw`, `@error`,
; `@string.special.symbol` — which does resolve, up to `string.special`)
; are left as written: an unknown name yields no spans rather than a wrong
; colour, and keeping the file close to upstream keeps it updatable.
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
