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
| N4 | done | `3f598d3` |
| N5 | done | `3f598d3` |
| N6 | done | `3f598d3` |
| N7 | done | `3f598d3` |
| N8 | done | `3f598d3` |
| N9 | done | `470c488` |

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
   See [ADR-0011](decisions/0011-code-navigation.md).
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

A `SearchModel::reindexFile` invokable on a background thread, wired to the project watcher's `filesChangedExternally` — which fires for the app's own saves as well as outside edits, so one hook covers both rather than a second, parallel save-time path.
A deletion needs no separate call: `TextIndex::reindex_file` drops the file's rows first and only re-adds them if the path is still readable, so no `removeFile` invokable was added.
Without this the whole feature silently jumps to stale lines; it is a correctness prerequisite, not a nice-to-have.

### N5 — `app-core`: navigation history

`NavigationHistory` with `record`/`back`/`forward`, owned by `AppSession`.
A new `record` truncates the forward tail; positions in the same file within ±1 line of the stack top collapse instead of pushing; the stack is capped.

### N6 — `ui-shell` bridge

`SearchModel::resolveDeclaration`, `findImplementations`, `findSupertypes` — background threads with the streaming signal trio the existing `usagesFound`/`usagesFinished`/`usagesFailed` uses.
Rows cross the seam as one `FfiSymbolMatch` struct rather than eight positional signal parameters: the same row travels on three signals, and a list that long is easy to mis-order at the call site.
`DocumentManager` history invokables returning a typed `FfiLocation { found, path, line, column }` — a typed `found` flag, never a QString sentinel (ADR-0003).

### N7 — `CodeEditor`: Ctrl-hover + Ctrl+Click

Mouse tracking, hover underline as a third contributor to the single `ExtraSelection` list the current-line band and find matches already share, and a `declarationRequested` signal on Ctrl+Click — mirroring `TerminalWidget`'s link handling.

### N8 — Menu actions, chooser popup, history wiring

New `Navigate` menu and `ActionDef`s, the ambiguity chooser, caret-driven Find Usages reusing the existing `FindUsagesPanel`, and history recording in the two functions every jump in the app already funnels through.

### N9 — Docs

ADR-0011, this plan doc, and the `layering.md`/`overview.md` sync.
`overview.md` had drifted well past this task — it still described the MVP and listed five of the eleven crates — so it was brought back in line with the code rather than only patched where navigation touched it.

## Fixed along the way

Two defects surfaced during the end-to-end pass and are fixed here, because the feature is unusable without them:

- **The watcher-driven re-index fed itself.**
  The index lives at `<project_root>/.ide-index/`, inside the tree the watcher watches, so every commit looked like a project change, re-entered `reindex_file`, and committed again.
  The index mutex was then permanently held and *every* query — Find in Files, Go to Symbol, Go to Declaration — hung on "Searching...".
  `TextIndex::reindex_file`/`remove_file` now ignore paths inside their own index directory, guarded where the directory layout that causes it lives rather than in the caller.
- **`rust/locals.scm` had drifted from `rust/tags.scm`.**
  Traits, `impl` targets and struct fields were definitions to the outline but not to `identifier_occurrences`, so `is_definition` was never set for them: Go to Declaration on a trait name found nothing, Go to Symbol could not list a trait, and a struct field was not indexed even as a *reference* (a field is a `field_identifier`, a node kind neither catch-all covered).
  The two queries are now in parity, with a regression test pinning it.
- **Go to Declaration needed a project index it does not actually use for the local tier.**
  `SearchModel::resolveDeclaration` refused outright with "No project is open yet." whenever the index slot was empty — no project open, a lone file, or a project whose index is still building.
  The local tier reads nothing but the buffer, so a Ctrl+Click on a same-file declaration silently did nothing in all three cases.
  Tier 1 is now `index_core::resolve_declaration_in_buffer`, a free function the adapter can call without an index.

## Polish

Symbol rows carry a column now, so Go to Symbol and the Class View project tier land the caret on the identifier instead of at column 0 — previously only Go to Declaration did, which made jumps feel inconsistent depending on which one you used.

## Deferred, deliberately

- **Persisted-index reuse across runs.**
  `TextIndex::build()` removes and rebuilds `.ide-index/` on every project open, so a large project pays a full walk and parse at each launch.
  Reusing the on-disk index needs an mtime field per document and a staleness sweep at open.
  Worth doing — but it is a startup-time task, not a navigation one.
- **Real tree-sitter scope queries** (`@local.scope`) replacing the local-tier heuristic — only worth it if the heuristic proves wrong in practice.
- **Cross-file type/binding resolution** — an ADR-0008 non-goal, and in practice a language server.
- **`.ide-index/` is visible in the project tree.**
  The index directory sits inside the project root and the sidebar shows it like any other folder.
  Hiding it means teaching `project-model`'s tree walk about a directory name that belongs to `index-core`, which the layering table does not allow it to depend on — so this needs a small decision (a configurable ignore list on the tree, most likely) rather than a quick filter.
