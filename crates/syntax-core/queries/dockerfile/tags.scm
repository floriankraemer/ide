; Outline extraction query for Task D (`outline()`), Dockerfile.
;
; The one named, navigable thing in a Dockerfile is the build stage
; (`FROM ... AS builder`) — a self-contained unit later stages copy from.
; It is reported as `@definition.class` because that is the only container
; kind `symbol_kind_for_capture` knows; `@definition.module` would compile
; and then be dropped silently. Unnamed stages have no name token and are
; deliberately not extracted.
(from_instruction (image_alias) @name) @definition.class
