# 0011. Code navigation: local-file-first declaration resolution, hover-free of resolution, supertype edges as a third index schema

## Status

Proposed.
Implemented as tasks N1–N9 of [the code navigation plan](../code-navigation-plan.md).

## Context

The IDE could find symbols but not navigate to them from the code.
Reaching a definition meant the Class View dock or the `Ctrl+Shift+O` dialog; the editor widget itself had no mouse handling beyond the fold gutter, and no code anywhere mapped "the identifier under the caret" to a definition site.

ADR-0008 built the symbol/reference index deliberately **name-based**: `find_definitions` matches a name substring, `find_usages` matches an exact name, and cross-file type/binding resolution is an explicit non-goal.
Go to Declaration is the first feature where that boundary is felt directly by the user: `Ctrl+Click` on a local variable that shares its name with a field in another file must not jump to the other file.

Three sub-decisions had to be made: how precise resolution should be and where the rule lives; how much work a `Ctrl`-hover may cost; and how the type hierarchy needed by Go to Implementation is stored.

## Decision

### Two-tier resolution: local file first, project index second

`TextIndex::resolve_declaration(current_path, current_content, byte_offset)` resolves in two tiers:

1. **Local file.** Definition-position occurrences (`syntax_core::identifier_occurrences`, `is_definition = true`) of the same name in the passed-in buffer, ranked nearest-preceding-first, then nearest-following.
   Local candidates win outright — if the name is declared in this file, a same-named symbol elsewhere is not what the caret meant.
2. **Project.** Otherwise, `find_definitions_exact(name)` over the index, excluding the current file.

Shadowing, parameters, and local bindings resolve correctly out of the ordering rule alone: an inner `let x` sits nearer the caret than an outer one.
The occurrence source matters here — `outline()`/`tags.scm` never captures a `let` binding or a parameter, while `locals.scm` does, so the local tier reads occurrences and merges kind/container from the outline only for display.

Ambiguity is surfaced, never guessed: tier 2 legitimately returns several candidates for two unrelated `run()` methods, and the view shows a chooser rather than picking one.

Rejected alternatives:

| Option | Why rejected |
|--------|--------------|
| Name-only lookup (no local tier) | Jumps to the wrong file on any shadowed local or same-named member — the single most common `Ctrl+Click` target. |
| Real tree-sitter scope queries (`@local.scope`/`@local.definition`) replacing the heuristic | Five languages of scope-chain work for a precision delta the nearest-preceding rule already covers in practice. Kept as the documented upgrade path if the heuristic proves wrong. |
| Full cross-file type/binding resolution | A language server in all but name, and an explicit ADR-0008 non-goal. Realistically this becomes "adopt LSP and delegate to rust-analyzer/omnisharp/jdtls/intelephense" — a product decision, not a refactor. |

### The rule lives in `index-core`

The ranking rule is a business rule, so `CLAUDE.md`'s humble-view rule and ADR-0002 keep it out of `bridge.rs` and out of `cpp/`.
It goes in `index-core`, which already depends on `syntax-core` and already owns the symbol schema — one Qt-free, unit-testable function.

`app-core` cannot host it: the layering table does not permit `app-core → index-core`, and the index is owned by `ui-shell`'s `SearchModel`.
Navigation *history*, by contrast, needs no index at all and does live in `app-core`.

`resolve_declaration` takes the buffer as a parameter rather than reading the file from disk, so an unsaved edit resolves against what the user is actually looking at — the same shape `saveTab(id, content)` and the F1–F8 find invokables already use (ADR-0009).

### Hover does no resolution

`Ctrl`-hover underlines any identifier-shaped word under the mouse: first character a letter or `_`, span taken from `QTextCursor::WordUnderCursor`.
No index query, no parse, no FFI round trip per mouse move.
Resolution runs on click.

This is deliberately *less* correct than underlining only resolvable identifiers — a `Ctrl`-hover over an unresolvable word still underlines — and it is what JetBrains does.
The alternative costs a background query per mouse-move event to remove an underline the user is about to click through anyway.

### Supertype edges are a third document type in the same index

Go to Implementation needs the "declares as base/interface/trait" relation, which `tags.scm` does not capture.
A new `syntax_core::supertype_edges()` reads a per-language `inherits.scm` (`@type` = the declaring type's name token, `@supertype` = one declared supertype), and `index-core` stores each edge as a `doc_type = "inherit"` document alongside the existing `text` and `symbol` shapes.

The edge rows carry the shared `path` term, so the existing `delete_term(path)` in `reindex_file`/`remove_file` drops them with the rest of a file's documents — no new invalidation path.

Consistent with `tags.scm`'s documented scope, only simple (non-generic) type names are captured: a generic base is a `generic_name`/`generic_type` node, and is skipped rather than reported under a wrong name.

### The symbol schema gains a column

`sym_col` (0-based byte offset within the line) is added so a jump lands on the identifier rather than at column 0.
`SymbolMatch` gains a matching `col`.

## Consequences

- Positive: the common case — a local, a parameter, a method in the file you are reading — resolves without touching the index at all, and the project tier is a single tantivy term query rather than `find_definitions`' fetch-everything-then-filter-in-Rust pass.
- Positive: every existing jump entry point (Find in Files, Go to Symbol, Class View, Go to Line) gains back/forward history for free, because history is recorded in the two functions all of them already funnel through.
- Negative / accepted: resolution is still name-based. Two unrelated same-named symbols in different files produce a chooser popup where a real resolver would produce one answer. Documented boundary, inherited from ADR-0008.
- Negative / accepted: `inherits.scm` skips generic supertypes, so `impl<T> Shape for Foo<T>` produces no edge. Same boundary `tags.scm` already documents for generic `impl` targets.
- The index staleness gap ADR-0008 left open (task H never wired `reindex_file`) is closed by this work, because a navigation feature is unusable on a stale index. Reusing a *persisted* index across runs — `build()` still wipes `.ide-index/` on every project open — stays open, and is recorded as deferred in the plan doc.

## Related

- [ADR-0008: Project index](0008-project-index.md) — the name-based schema this extends, and the deferred reindex wiring this closes.
- [ADR-0003: FFI seam conventions](0003-ffi-conventions.md) — the typed `FfiLocation { found, ... }` result and the streaming failure signals used here.
- [ADR-0002: Application layer and humble view](0002-application-layer-and-humble-view.md) — why the ranking rule cannot live in `bridge.rs` or `cpp/`.
- [ADR-0009: Find & Replace](0009-find-and-replace.md) — the "pass the live buffer as a parameter" convention `resolve_declaration` follows.
