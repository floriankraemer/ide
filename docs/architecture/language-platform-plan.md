# Language platform: extensible tree-sitter languages, per-language theming, runtime grammars, LSP

## Progress

Living status table — update the relevant row(s) **in the same commit**
that finishes a task, so status and code never drift apart. A fresh
session should read this table (and `git log`) before picking up work,
per `CLAUDE.md`.

| Task | Status | Commit |
|---|---|---|
| X1 | done | `ea4f8eb` |
| X2 | done | `0554341` |
| P1 | done | `faa1a00` |
| R1 | done | `a8f0ebf` |
| R2 | done | `01f1685` |
| R3 | done | `ea4f8eb` |
| T1 | done | `01f1685` |
| T2 | done | `76352ce` |
| T3 | done | `fb02f2b` |
| T4 | done | `6a971f0` |
| I1 | done | `fb02f2b` |
| R4a | done | `7ed7b79` |
| R4b | done | `7ed7b79` |
| R4c | done | `7ed7b79` |
| R4d | done | `b7c7512` |
| R5 | done | `64a4243` |
| R6 | done | `e2a37e4` |
| G1a | done | `01f1685` |
| G1b | done | `f4f2687` |
| G2 | done | `6a971f0` |
| G3 | done | `6a971f0` |
| L1 | done | `0554341` |
| L2 | done | `c0a83b5` |
| L3 | done | `f4f2687` |
| L4 | done | `f4f2687` |
| L5 | done | `6a971f0` |
| L6 | done | `6a971f0` |

## Context

`syntax-core` was already tree-sitter based but not extensible: every language
cost an `enum Language` variant, extension/name match arms, five hand-written
`LazyLock<QueryLanguage>` functions and five dispatch matches — 25 near-identical
functions for five languages.

This plan turns it into a language *platform*: a merged language registry, the
standard tree-sitter capture vocabulary as the scope taxonomy, syntax colors as
data (base plus per-language overrides) instead of hardcoded C++ literals,
runtime-loadable languages, injections, and a modular LSP client.

Full design reasoning lives in the session plan file; this doc carries the
durable task breakdown and status forward.

## Key design decisions

1. **`tree-sitter` 0.24 → 0.26 first.** The 0.24 runtime accepts grammar ABI
   13..=14 while current grammar crates ship ABI 15, which forced the
   `tree-sitter-c-sharp` 0.21.3 / `tree-sitter-php` 0.23.11 pins and would have
   blocked most of the language batch.
2. **One merged `LanguageRegistry`**, built from a const `BUILTIN_LANGUAGES`
   table plus runtime entries — not a const slice and a runtime table consulted
   separately. Mirrors how `keymap::ACTIONS` + `Settings::keymap()` already
   split data from resolution. `Language` is an opaque `Copy` handle.
3. **Compiled languages live behind `Arc`** so a live registry reload cannot
   invalidate an open editor's `Highlighter`.
4. **Lookup is path-based, not extension-based** — `Dockerfile`/`Makefile` have
   no extension. Extension collisions (`.h`, `.ts`, `.m`) resolve
   first-match-wins in catalog order; that is a documented rule, not a bug.
5. **The scope taxonomy is the standard tree-sitter capture vocabulary**, static
   and closed, with hierarchical fallback (`a.b.c` → `a.b` → `a` → dropped).
   Runtime grammars never intern new scope ids. This is what makes upstream
   `.scm` files reusable unmodified.
6. **Scope ids cross the FFI seam as `u16`.** ADR-0003 governs error shapes and
   entity identity; a scope id is neither. A cxx enum would mean every new scope
   touches `bridge.rs`. The C++ side range-guards the index.
7. **No `theme-core` crate.** Theme rules and built-in tables go in
   `syntax-core::theme` (reusing the dotted-scope walk); persistence goes in
   `app-config` as plain string maps, gaining no new dependency.
8. **`LspManager` lives in `lsp-core`, not `bridge.rs`** — lifecycle, restart
   backoff and document-version tracking are rules, and the adapter is allowed
   none. `bridge.rs` keeps only a listener thread and `qt_thread.queue()`.
9. **`lsp-core` uses blocking threads, not tokio.** LSP is request/response over
   one stdio pipe per server; a runtime buys nothing without socket fan-out and
   taxes testing against a stub binary.
10. **Runtime extensibility ships in two stages**: a zero-`unsafe` manifest +
    `.scm` overlay first, then hardened foreign dylibs (canonical symbol only,
    ABI check, full query compile, crash-quarantine marker, never unloaded).

## Tasks

| # | Task | Deliverable | Verification |
|---|---|---|---|
| X1 | Registry test harness | Table-driven test walking the registry: every query compiles, each `queries/<id>/sample.txt` yields ≥1 keyword/string/comment span | Harness fails when pointed at a broken query |
| X2 | LSP stub server | Minimal stub language server: framing, `initialize`, canned diagnostics, a die-mid-session mode | `cargo test -p lsp-core` runs offline against it |
| P1 | tree-sitter runtime bump | 0.26, unpin C#/PHP, drop the C#-specific `language()` special case, fix node-name drift in shipped queries | All shipped queries compile; `cargo test --workspace` green |
| R1 | Language registry | `LanguageDef` + `BUILTIN_LANGUAGES` + `Arc`-held `LanguageRegistry` + opaque `Language(u16)`; 25 `LazyLock` fns and 5 matches deleted; path-based lookup threaded through the seam | Existing tests pass unmodified; guards for unique ids, deterministic `.h`, every query compiles |
| R2 | Single-parse extraction | One entry point returning outline + occurrences + supertype edges; `index-core`'s five parses collapsed to one | `cargo test -p index-core` green; index a fixture repo and compare timing |
| R3 | Scope taxonomy + seam + hot path | `Scope`/`SCOPES` replaces `TokenKind`; `FfiHighlightSpan{start,end,scope:u16}`; C++ format-table indexing with a range guard and `lower_bound` instead of the full-document span scan | Fallback cases; `make test`; typing latency no worse |
| T1 | Theme rules + defaults | `syntax_core::theme`: `ScopeStyle`, three built-in tables, `palette()` with the full precedence chain | Precedence, parent fallback, per-language override |
| T2 | Color persistence | `syntax_colors` base + per-language maps in `app-config`, strings only | Round-trip, partial file, unknown scope ignored |
| T3 | Colors out of C++ | `syntax_palette()` bridge call; `colorForKind`/`vscodeDarkColorForKind` deleted; palette invalidated on theme change and language reload | No hex literals left in `syntax_highlighter.cpp`; live theme switch recolors |
| T4 | Settings > Syntax Colors page | `syntax_colors_page.{h,cpp}` + `SyntaxColorsEditor` draft QObject | Per-language color persists on OK, reverts on Cancel |
| I1 | Injections | `injections.scm` support, multi-tree `Highlighter`, spans merged in document order | JS-in-HTML, code-fence-in-Markdown, HTML-in-PHP fixtures |
| R4a | Batch: Python, C, C++ | catalog rows + queries + sample | X1 harness |
| R4b | Batch: Go, TypeScript, JavaScript | as above | X1 harness |
| R4c | Batch: Bash, YAML, TOML | as above | X1 harness |
| R4d | Batch: Markdown, HTML, CSS, XML | needs I1; `tree-sitter-md` is block + inline | X1 harness + injected-region assertions |
| R5 | Batch: SQL, Ruby, Lua, Make, Dockerfile | filename-matched languages exercise path lookup | X1 harness |
| R6 | Batch: F#, Kotlin, Swift, Scala, Zig, Haskell | riskier ABI/maturity set; drop with a documented reason if a grammar will not load | X1 harness |
| G1a | Runtime data overlay | `language.toml` + external `.scm` layered over builtins by id; zero `unsafe` | Override precedence, malformed manifest, broken query |
| G1b | Foreign grammar dylibs | `libloading`, canonical `tree_sitter_<id>` symbol, ABI check, query compile, crash-quarantine marker, mingw-only on Windows | Valid / bad ABI / missing symbol / broken query fixtures |
| G2 | Live reload | Registry rebuild with `Arc`-held compiled languages; open editors keep their grammar | Reload while a `Highlighter` is alive |
| G3 | Settings > Languages page | Language list with source and load errors, also surfaced in the status bar | A broken grammar shows its error, editor stays up |
| L1 | `lsp-core` foundation | Framing, child-process transport, lifecycle, `LspManager` with restart/backoff and version tracking, server catalog, `[[language_server]]` settings | Framing round-trip, lifecycle, respawn against the stub; no qt/tokio in the dep tree |
| L2 | Diagnostics | `didOpen`/`didChange`/`didSave`, squiggles, Problems dock | Error in a Python file shows a squiggle and a Problems row |
| L3 | Hover | Hover on mouse-dwell, tooltip | Hover a symbol, see server docs |
| L4 | Go-to-definition via LSP | LSP preferred, ADR-0011 index as fallback | Ctrl+Click with and without a server |
| L5 | Completion | Completion request + popup | Trigger completion in a Python file |
| L6 | Settings > Language Servers page | Per-language command/args/enabled plus live status | Bad command surfaces, no crash |

## Out of scope

Per-language settings beyond colors (tab width, comment tokens), indentation and
textobject queries, a chrome-theme editor, and WASM grammars (tree-sitter 0.26's
`wasm` feature drags `wasmtime-c-api`, which would have to cross-compile under
MXE — revisit only if `libloading` proves crashy). A per-file "Set language"
override is deferred but expected once `.h`/`.ts`/`.m` ambiguity is live.

## Known gaps, found while building

Each of these was discovered by the task that hit it, and deliberately
left rather than fixed in place.
They are recorded here so the next session can pick them up with the
reasoning intact.

- **Query predicates are not evaluated.** ([#16](https://github.com/floriankraemer/ide/issues/16))
  `spans_from_tree` runs `QueryCursor::matches` without checking `#match?`,
  `#eq?`, `#any-of?` or `#is-not?`.
  Every tranche therefore had to *drop* upstream patterns guarded by them —
  Ruby's `#is-not? local` rule alone would have painted every identifier as a
  method call.
  The cost is real: CamelCase-is-a-type and SCREAMING_CASE-is-a-constant
  conventions are simply absent.
  Implementing predicate evaluation would let most of those patterns be
  restored, and is the single largest available improvement to highlighting
  quality.
- **`SCOPES` has no `markup` family.** ([#18](https://github.com/floriankraemer/ide/issues/18))
  Markdown headings, links, emphasis and code spans are captured upstream as
  `@markup.heading`, `@markup.link` and so on, which resolve to nothing here,
  so a Markdown file shows only its fenced code and inline HTML coloured.
  Adding the family touches `SCOPES`, every theme table and the view's format
  table, so it is its own change rather than a markup-tranche edit.
- **The X1 harness can be satisfied by a descendant scope.** ([#17](https://github.com/floriankraemer/ide/issues/17))
  `produces_scope` counts a `string.escape` span as satisfying `string`,
  which is right for fallback but means a language that has *lost* its
  `string` rule can still pass — this actually happened to Zig during R6 and
  was caught by eye, not by the harness.
  Requiring at least one exact match, with descendants as a fallback only
  when the exact scope is genuinely unused, would close it.
- **~~`filenames` is exact-match only.~~** (fixed, [#19](https://github.com/floriankraemer/ide/issues/19))
  `language_for_path` now resolves in three ordered steps: exact file name,
  then extension, then the file name with its final `.suffix` removed matched
  against `filenames` again.
  `Dockerfile.dev` and `Makefile.local` resolve through the third step, while
  `Dockerfile.md` stays Markdown because a real extension is consulted first.
  No pattern field was needed: the rule holds for every row in the catalog and
  for runtime languages, so no language opts into it.
- ~~**`lsp-core`'s extension-to-language-id table will drift.**~~ ([#20](https://github.com/floriankraemer/ide/issues/20), fixed)
  It had drifted: no server started for the markup and batch-two languages.
  The duplicate table is gone rather than extended — `lsp-core` now derives
  the language from `syntax-core`'s registry and keeps only what the protocol
  actually owns, the server command per language id and the one id that
  diverges (`tsx` -> `typescriptreact`).
  See [ADR-0018](decisions/0018-single-source-language-detection.md).
- **Runtime language definitions leak one generation per reload.** (not filed: bounded and deliberate)
  Bounded and tiny — a few KB per reload of a human pressing a button — and
  the alternative costs a lifetime parameter on every `LanguageDef` reader.
  The reasoning, and the one condition that flips the answer (reloading from
  a file watcher rather than a button), is commented at the leak site.
- **No user-facing enable/disable for a healthy language.** (not filed: unbuilt feature, not a defect)
  The Languages page can only re-enable a crash-quarantined grammar, because
  no settings field or registry filtering exists for a plain disable.
  A button was not added rather than shipping one that lies.
- **Everything visual is unverified.** (not filed: a review task, not a defect)
  There is no display in the build environment, so the three settings pages,
  the Problems dock, squiggles, hover tooltips and the completion popup have
  been compiled and unit-tested but never seen.
  `docs/design/language-platform-ui.md` and the final task reports list what
  a human should click through.

Two further defects found during the work are filed but not listed above,
because they were fixed or accepted rather than left open as gaps:
the go-to-definition regression the single-parse refactor introduced
([#21](https://github.com/floriankraemer/ide/issues/21), since fixed with a
caller-driven parse handle: the miss path runs the `locals` walk and stops),
and the TOML nested-table gotcha that silently discarded a hand-edited dotted
scope colour ([#22](https://github.com/floriankraemer/ide/issues/22), fixed:
both spellings load, and an unknown scope name is now said out loud on the
Syntax Colors page).
