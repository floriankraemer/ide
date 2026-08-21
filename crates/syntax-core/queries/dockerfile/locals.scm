; Identifier-occurrence query for A2 (`identifier_occurrences`), Dockerfile.
;
; A Dockerfile's only bindings are build arguments and environment
; variables: `ARG`/`ENV` introduce a name, `${NAME}` expands it — the same
; definition/reference split bash/locals.scm uses.

(arg_pair name: (unquoted_string) @definition)
(env_pair name: (unquoted_string) @definition)

(variable) @reference
