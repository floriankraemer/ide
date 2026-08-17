# Language expansion, folding, Class View, embedded terminal, project index+search

## Progress

Living status table — update the relevant row(s) **in the same commit**
that finishes a task, so status and code never drift apart. A fresh
session should read this table (and `git log`) before picking up work,
per `CLAUDE.md`.

| Task | Status | Commit |
|---|---|---|
| A | done | 176116e |
| A1 | done | ea45195 |
| A2 | done | 073bbf4 |
| B | open | |
| C | open | |
| D | open | |
| E1 | open | |
| F1 | done | 178ac09 |
| F2 | open | |
| F3 | open | |
| G1 | open | |
| H | open | |
| I | open | |
| J | open | |

## Context

User request: add C#, Java, and PHP syntax highlighting; collapsible code
folding for class/method blocks; a JetBrains/VS-style "Class View" (tree of
classes/methods/members, dockable); an embedded, dockable cross-platform
shell (PowerShell, WSL2, regular shell); and project-wide indexing with a
search foundation covering both text ("Find in Files") and symbols/
references — explicitly optimized for editing/indexing/startup speed
("fastest IDE on earth").

Designed by the `software-architect` subagent against the existing
codebase (verified via exploration: no folding/outline/terminal/index code
exists anywhere yet; `syntax-core` only supports Rust/JSON via hand-rolled
node-kind matching; ADS docking has two dock widgets today — Editor,
Project tree — with an explicit extension point already earmarked for
"search, run console"). Full plan reasoning lives in this session's plan
file; this doc carries the durable task breakdown forward.

## Key design decisions

1. **`syntax-core` migrates to tree-sitter query-based extraction**
   (`highlights.scm`/`folds.scm`/`tags.scm` per language) before adding
   C#/Java/PHP — avoids 15 hand-written matchers (5 languages × 3
   concerns) and matches the `tree-sitter-tags`/Helix/nvim-treesitter
   convention, so grammar-shipped query files are often directly reusable.
2. **Incremental reparse lands as part of this work, not deferred.**
   `syntax-core` moves to a persistent `tree_sitter::Tree` per `Document`
   with `.edit()` + incremental `.parse()`, replacing the documented
   full-buffer-reparse-per-revision ceiling (prior ADR decision A6) —
   folding, outline, and reference extraction all read from one
   incrementally-maintained tree instead of tripling that ceiling's cost.
3. **Embedded terminal**: new Qt-free crates `pty-core` (`portable-pty`,
   cross-platform PTY transport) and `terminal-core` (`alacritty_terminal`,
   VT100/grid state, unit-testable) — not `QTermWidget`, which would put
   untestable VT logic behind Qt. `cpp/` only paints the cell grid and
   forwards input (humble view, same shape as `SyntaxHighlighter`).
4. **Project index**: new Qt-free crate `index-core`, two schemas —
   text (tantivy ngram(3) candidate-narrowing + `grep-searcher`/
   `grep-regex` exact verification, for Find in Files) and symbols/
   references (tantivy fields fed by `syntax-core::outline()` +
   new `identifier_occurrences()`, name-based — no cross-file type/
   binding resolution, a deliberate scope boundary short of a language
   server). Both feed Class View's project-wide tier and a new Symbol
   Search UI (go-to-symbol, find-usages). Incremental updates ride the
   existing `ProjectWatcher`.

All new background work (PTY reads, indexing) uses a plain `std::thread` +
`CxxQtThread::queue()` to marshal results to Qt — the exact pattern
`start_mcp_server` already uses (`ui-shell/src/bridge.rs:1160-1182`).
`tokio` stays justified only for `mcp-server`.

## New crates

| Crate | Layer | New deps | Purpose |
|---|---|---|---|
| `syntax-core` (extended) | Qt-free | `tree-sitter-c-sharp`, `tree-sitter-java`, `tree-sitter-php` | +3 languages, query-based extraction, incremental reparse |
| `pty-core` | Qt-free, new | `portable-pty` | Cross-platform PTY spawn/IO/resize |
| `terminal-core` | Qt-free, new | `alacritty_terminal` | VT100/grid state, consumes `pty-core`'s byte stream |
| `index-core` | Qt-free, new | `tantivy`, `grep-searcher`, `grep-regex`, `grep-matcher`, `ignore` | Text + symbol/reference index, Find in Files, Symbol Search |

`docs/architecture/layering.md`'s allowed-imports table needs rows for the
3 new crates and `syntax-core`'s new deps, updated in the same commit as
the structural change (per `CLAUDE.md`). Add all four crates to the
Qt-leakage verification list: `cargo tree -p <crate> -e normal | grep -i qt`
must stay empty.

## Task breakdown

| # | Task | Deliverable | Verification |
|---|---|---|---|
| A | Query-engine migration (Rust+JSON) | `tree_sitter::Query`/`QueryCursor` against `highlights.scm` replaces `classify_rust`/`classify_json`; existing `HighlightSpan` output/tests unchanged | `cargo test -p syntax-core`, existing tests green unmodified |
| A1 | Incremental reparse | Persistent `tree_sitter::Tree` per `Document`, `.edit()` + incremental `.parse()`, replaces full-buffer reparse-per-revision in `SyntaxHighlighter` | `cargo test -p syntax-core`; manual typing-latency pass on a large real file |
| A2 | `identifier_occurrences()` | Generic per-language identifier-node query, every occurrence of a name (not just definitions) | `cargo test -p syntax-core`: known snippets → expected occurrence list |
| B | C#/Java/PHP grammars | `Language` variants + extension map + `highlights.scm`/`folds.scm`/`tags.scm` trio per language (`php_only` grammar, not embedded-HTML) | `cargo test -p syntax-core` per language fixture |
| C | Folding UI | `syntax-core::fold_ranges()` off the incremental tree; gutter fold/unfold markers in `cpp/`, collapsed-state kept in the C++ editor widget (view state, not persisted) | Manual: fold/unfold a class and a method |
| D | Class View — per-file tier | New dock widget at `buildCentralWidget()`'s extension point; tree model from `syntax-core::outline()` on current tab, refreshed on save | Manual: multi-class file shows correct tree, updates on save |
| E1 | Symbol+reference schema | `index-core` tantivy fields (name/kind/file/line/container/`is_definition`) fed by `outline()` + `identifier_occurrences()` | `cargo test -p index-core`: fixture project → expected symbol/reference rows |
| F1 | `pty-core` | Cross-platform PTY spawn/IO/resize via `portable-pty`; per-platform shell resolution (Windows: `pwsh.exe`/`powershell.exe`/`wsl.exe`; Linux: `$SHELL`) | `cargo test -p pty-core`; Docker `linux-artifact` **and** `windows-artifact` build clean |
| F2 | `terminal-core` | `alacritty_terminal`-backed grid/cursor/selection state from PTY byte stream | `cargo test -p terminal-core`: known escape-sequence fixtures |
| F3 | Terminal dock widget | `QPainter`-based grid widget in `cpp/`, forwards input via `CxxQtThread::queue()`; new dock widget at extension point | Manual: real shell opens, keystrokes/output round-trip on Linux and Windows |
| G1 | `index-core` text-index skeleton | tantivy ngram(3) content field + `grep-searcher`/`grep-regex` verification; initial walk via `ignore` crate; incremental updates via `ProjectWatcher` | `cargo test -p index-core`: build+query round trip on a fixture repo |
| H | Find in Files UI | Search UI wired to `index-core` text schema, match spans highlighted | Manual: multi-language repo, correct results and spans |
| I | Class View — project-wide tier | Data source swap to `index-core` symbol query (same widget/model as D) | Manual: project-wide tree matches indexed symbols |
| J | Symbol Search UI | Go-to-symbol (`is_definition=true` query) + find-usages (all occurrences by name, grouped by file) | Manual: multi-file fixture, find a symbol and its usages |

## Sequencing

Track 1 (sequential): A → A1 → B · A1 → C · A1 → D.
Track 2 (parallel, independent from day one): F1 → F2 → F3.
Track 3: G1 (independent) → H; A1 → A2 → E1 (needs A2 + G1's schema
groundwork) → J; E1 + D → I.

**Gate before starting F1/G1 implementation**: `devops-expert` must verify
`portable-pty`/`alacritty_terminal` (Windows ConPTY) and tantivy's
`zstd-sys` C dependency build clean under the existing MXE cross-build
stage — same class of risk D1 hit for ADS ("two vendor-side gaps found and
worked around" per the prior plan doc). Do not mark ADR-0007/0008 Accepted
until this comes back clean.

## ADRs owed (numbered from 0006)

| # | Decision | Why |
|---|---|---|
| ADR-0006 | Tree-sitter query-based extraction (`.scm` files) replaces hand-rolled node-kind matching; `syntax-core` moves to persistent-`Tree` incremental reparse | Scales to 5 languages × 3 concerns without a matcher explosion; closes the documented A6 reparse ceiling instead of inheriting it into 3 new features |
| ADR-0007 | Embedded terminal: `portable-pty` + `alacritty_terminal` + custom `QPainter` grid widget | `QTermWidget` rejected (untestable VT logic behind Qt); hand-rolled VT100 rejected (correctness minefield). Pending devops-expert MXE check |
| ADR-0008 | Project index: hybrid tantivy(ngram)+ripgrep-crates for text, name-based (not type-resolved) symbol/reference schema | Pure full-text index rejected (wrong tool for exact substring/regex search); pure on-demand grep rejected (no persistent symbol foundation). Records "find usages" as name-based by deliberate scope boundary, not oversight. Pending devops-expert MXE check (zstd-sys) |

## Verification approach

Each task verifies independently per the table above (`cargo test -p
<crate>` for Qt-free crates, manual UI pass for `ui-shell` changes,
consistent with the prior plan doc's approach — no headless Qt test
runner exists in this repo). Docker `linux-artifact`/`windows-artifact`
builds required for every task touching the 3 new native-dependency
crates. Before declaring this initiative done: full manual smoke pass
covering every new US here on both Linux and the Windows cross-build
artifact (new-language highlighting, fold/unfold, Class View both tiers,
terminal on both platforms, Find in Files, Symbol Search go-to-symbol and
find-usages, incremental re-index after a save).
