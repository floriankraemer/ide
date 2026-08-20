# Clickable code navigation: Go to Declaration, Find Usages from the caret, Go to Implementation, navigation history

## Progress

Living status table — update the relevant row(s) **in the same commit**
that finishes a task, so status and code never drift apart.
A fresh session should read this table (and `git log`) before picking up work, per `CLAUDE.md`.

| Task | Status | Commit |
|---|---|---|
| N1 | done | `8d11fb8` |
| N2 | done | `8d11fb8` |
| N3 | done | `8d11fb8` |
| N4 | open | |
| N5 | open | |
| N6 | open | |
| N7 | open | |
| N8 | open | |
| N9 | open | |

## Context

The IDE could already *find* symbols but not *navigate* to them from the code itself.
Before this plan the only routes to a definition were the Class View dock's double-click and the `Ctrl+Shift+O` Go to Symbol dialog; `CodeEditor` had no mouse handling at all beyond the fold gutter, and nothing mapped "the identifier under the caret" to a definition site.
Reading unfamiliar code was therefore slow — the opposite of this project's stated performance goal.

Two supporting gaps made the feature unreliable if built naively:

1. **The symbol index went stale on the first edit.**
   `index_core::TextIndex::reindex_file`/`remove_file` existed and were tested, but had **zero call sites** outside the crate — the index was built once on project open and never updated.
   ADR-0008 assigned that wiring to task H; it was not done.
   Line numbers drifted after any edit, so a jump landed on the wrong line.
2. **The symbol schema stored a line but no column**, so a jump could only land at the start of a line, never on the identifier.

## Key design decisions

1. **Two-tier resolution, local file first, project index second.**
   Local-file candidates are definition-position occurrences of the same name in the buffer the caret is in, ranked nearest-preceding-first; they win outright over project-wide candidates.
   Shadowing falls out of the ordering (an inner `let x` is nearer the caret than an outer one) instead of needing a scope graph.
   See [ADR-0010](decisions/0010-code-navigation.md).
2. **No scope queries, no binding resolver.**
   Upgrading all five `locals.scm` files to real tree-sitter scope queries (`@local.scope`/`@local.definition`) was rejected: five languages of scope-chain work for a precision delta decision 1 already covers on the cases users actually `Ctrl+Click`.
   Cross-file type/binding resolution stays an explicit ADR-0008 non-goal.
3. **The ranking rule lives in `index-core`**, which already depends on `syntax-core` and already owns the symbol schema — one Qt-free, unit-testable function.
   `app-core` cannot host it: the layering table does not allow `app-core → index-core`, and the index is owned by `ui-shell`'s `SearchModel`.
4. **Hover costs nothing.**
   `Ctrl`-hover underlines any identifier-shaped word under the mouse with no resolution work — no index hit, no parse, no FFI round trip per mouse move.
   Resolution happens on click, exactly as JetBrains does it.
5. **Go to Implementation rides a third document type** (`doc_type = "inherit"`) in the same tantivy index, fed by a new `syntax_core::supertype_edges()` and its per-language `inherits.scm`.
   Reusing the shared `path` term means an existing `delete_term(path)` on reindex still wipes those rows in one call.

## Tasks

### N1 — `index-core`: column in the schema + exact-name definition lookup

`sym_col` field (0-based byte column within the line) added to the symbol schema and populated in `index_symbols`; `col` added to `SymbolMatch`; `line_number_at` replaced by `line_and_col_at`.
New `find_definitions_exact(name)` narrows to the name inside tantivy instead of fetching every definition doc and substring-filtering in Rust, as `find_definitions` does.

### N2 — `index-core`: the resolution rule

`TextIndex::resolve_declaration(current_path, current_content, byte_offset) -> Resolution`, implementing decision 1.
`current_content` is passed in rather than read from disk, so an unsaved buffer resolves against what the user is actually looking at.

### N3 — Supertype edges for Go to Implementation

`syntax_core::supertype_edges()` plus `queries/<lang>/inherits.scm` for all five languages, and the `doc_type = "inherit"` rows, `find_implementations(supertype)` and `find_supertypes(type_name)` in `index-core`.

### N4 — Fix the stale index

`SearchModel::reindexFile`/`removeFile` invokables on a background thread, called after a successful save and from the project watcher path.
Without this the whole feature silently jumps to stale lines; it is a correctness prerequisite, not a nice-to-have.

### N5 — `app-core`: navigation history

`NavigationHistory` with `record`/`back`/`forward`, owned by `AppSession`.
A new `record` truncates the forward tail; positions in the same file within ±1 line of the stack top collapse instead of pushing; the stack is capped.

### N6 — `ui-shell` bridge

`SearchModel::resolveDeclaration`, `findImplementations`, `findSupertypes` — background threads with the streaming signal trio the existing `usagesFound`/`usagesFinished`/`usagesFailed` uses.
`DocumentManager` history invokables returning a typed `FfiLocation { found, path, line, col }` — a typed `found` flag, never a QString sentinel (ADR-0003).

### N7 — `CodeEditor`: Ctrl-hover + Ctrl+Click

Mouse tracking, hover underline as a third contributor to the single `ExtraSelection` list the current-line band and find matches already share, and a `declarationRequested` signal on Ctrl+Click — mirroring `TerminalWidget`'s link handling.

### N8 — Menu actions, chooser popup, history wiring

New `Navigate` menu and `ActionDef`s, the ambiguity chooser, caret-driven Find Usages reusing the existing `FindUsagesPanel`, and history recording in the two functions every jump in the app already funnels through.

### N9 — Docs

ADR-0010, this plan doc, and the `layering.md`/`overview.md` sync.

## Deferred, deliberately

- **Persisted-index reuse across runs.**
  `TextIndex::build()` removes and rebuilds `.ide-index/` on every project open, so a large project pays a full walk and parse at each launch.
  Reusing the on-disk index needs an mtime field per document and a staleness sweep at open.
  Worth doing — but it is a startup-time task, not a navigation one.
- **Real tree-sitter scope queries** (`@local.scope`) replacing the local-tier heuristic — only worth it if the heuristic proves wrong in practice.
- **Cross-file type/binding resolution** — an ADR-0008 non-goal, and in practice a language server.
