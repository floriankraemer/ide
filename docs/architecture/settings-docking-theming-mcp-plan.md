# Implementation plan: Settings, Docking, Theming, MCP Foundation, Line Numbers, Tab Reorder, Syntax Highlighting Foundation, Quick Wins

## Status

Draft — approved, implementation starting. Builds on the shipped MVP
([MVP implementation plan](mvp-implementation-plan.md)) and stays inside
[ADR 0001](decisions/0001-core-tech-stack.md) (Rust core + Qt6 UI via
`cxx-qt`, hybrid plugin system). Does not reopen ADR 0001.

## Progress

Living status table — update the relevant row(s) **in the same commit**
that finishes a task, so status and code never drift apart. A fresh
session should read this table (and `git log`) before picking up work,
per `CLAUDE.md`.

| Task | Status | Commit |
|---|---|---|
| R1 | done | `1099c15` |
| R2 | done | `1099c15` |
| R3 | done | `1099c15` |
| R4 | done | `1099c15` |
| R5 | done | `1099c15` |
| R6 | done | `1099c15` |
| R7 | done | `05fe5ce` (docs sync swept in earlier; plan doc itself fixed here) |
| C1 | done | `ec078c1` |
| C2 | done | `ad49424` (`cargo test -p app-config`; Docker `linux-artifact` build verified; manual open-3-folders pass not run, no display here) |
| L1 | done | `ad49424` (Docker `linux-artifact` build verified; manual close/relaunch-geometry pass not run, no display here) |
| L2 | done | `ad49424` (`cargo test -p app-core`; Docker `linux-artifact` build verified; manual Save-As pass not run, no display here) |
| L3 | done | `751c35f` (`cargo test -p syntax-core`; Docker `linux-artifact` build verified; manual .rs/.json label-switch pass not run, no display here) |
| G1 | done | `38e846b` (Docker `linux-artifact` build verified; no display here for gutter-rendering manual pass) |
| G2 | done | `05df10e` (Docker `linux-artifact` build verified; manual drag-reorder pass not run, no display here) |
| T1 | done | `843d464` (Docker `linux-artifact` build verified; dark-launch visual not manually smoke-tested, no display here) |
| T2 | done | `aceb830` (`cargo test -p app-config`; Docker `linux-artifact` build verified; live-switch trigger arrives with S1, manual restyle pass not run, no display here) |
| S1 | done | `2e0e0c2` (Docker `linux-artifact` build verified; manual change-theme/OK/relaunch and Cancel-discards passes not run, no display here) |
| S2 | done | `2e0e0c2` (`cargo test -p app-config`; Docker `linux-artifact` build verified; manual live-update/persist passes not run, no display here) |
| Y1 | done | `1475e3f` |
| Y2 | done | `93dcc3d` (`cargo test -p app-core -p syntax-core`; Docker `linux-artifact` build verified; manual .rs/.json/.txt visual pass not run, no display here) |
| D1 | done | `c7c3a41` (go: Docker `linux-artifact` **and** `windows-artifact` both build/link clean; two vendor-side gaps found and worked around, see commit; runtime dock rendering not manually verified, no display here) |
| D2 | done | `0b1ebcb` (docs only — ADR-0005) |
| D3 | done | `3984752` (Docker `linux-artifact` **and** `windows-artifact` both build/link clean; manual float/redock and G2 tab-behavior passes not run, no display here) |
| D4 | done | `9420ed8` (Docker `linux-artifact` **and** `windows-artifact` both build/link clean; manual float-sidebar/relaunch-restored pass not run, no display here) |
| M1 | done | `74c0bef` |
| M2 | done | `16302d2` (docs only — ADR-0004) |
| M3 | done | `pending` (`cargo test -p mcp-server -p app-core`; Qt-leakage gate clean on `mcp-server`; Docker `linux-artifact` build verified; MCP-client-against-running-UI round trip not run, no display here) |
| M4 | todo | |
| M5 | todo | |

## Context

The IDE has a working MVP: open-folder, sidebar file tree, tabbed
`QPlainTextEdit` editor with save, filesystem watcher.
It has no theming, no settings, no docking, no line numbers, no tab
reordering, no syntax highlighting, and no AI-control surface.
This plan pushes it from "MVP shell" toward a real open-source,
high-performance, AI-first IDE: a JetBrains-style Settings window, a
JetBrains/VS-style dockable window system, a Darcula + Light theme pair
styled after the JetBrains "Material Theme" look, a foundation for an MCP
server so AI agents can fully read/control the editor, editor line
numbers, reorderable tabs, font/color settings, a tree-sitter-based
multi-language syntax-highlighting foundation, and a short list of
additional quick wins.

Decisions made up front: the MCP foundation supports full read+write
editor control (not read-only); docking uses the Qt Advanced Docking
System (ADS) library rather than a hand-rolled docking engine; syntax
highlighting is built on tree-sitter.

## Current-state facts (verified against source, post-Phase-0)

- Five crates: `editor-core`, `project-model`, **`app-core`** (all
  Qt-free, unit-tested without a Qt runtime), `ui-shell` (only crate
  touching `cxx-qt`/Qt), `app` (thin binary).
- `app-core::AppSession` owns the one open `ProjectSession` plus an
  open-document table keyed by `TabId(u64)` (issued monotonically from 1,
  never reused), and exposes `Result<T, AppError>` command methods
  (`open_project`, `open_file`, `save_tab`, `rename_entry`,
  `delete_entry`, `close_tab`, `reload_tab`, `check_external_change`, …).
  All business rules live here: the binary-open rule, rename path
  construction, delete → tab-invalidation as one call, watcher-echo
  suppression. 21 unit tests.
- `crates/ui-shell/src/bridge.rs` is a thin adapter: `ProjectTreeModel`
  and `DocumentManager` QObjects hold a shared `Rc<RefCell<AppSession>>`
  (`shared_session()`), own no domain state, and each invokable is
  slot → translate → `AppSession` call → emit signal/refresh model.
  Errors cross as typed `FfiResult`/`FfiOpenResult` structs (`code` +
  `message`), never a `QString` sentinel.
- `crates/ui-shell/cpp/main_window.cpp`'s `EditorTabs` is a humble view:
  no business rules, branches only on `FfiResult::code`. Tab identity is
  the session's `TabId`, stored as a Qt dynamic property (`"tabId"`) on
  each page widget and looked up by scanning `tabWidget_` — **there is no
  separate index-bookkeeping list to keep in sync** (the old parallel
  `titles_ QStringList` is gone entirely). This means enabling tab drag
  reorder needs no companion identity fix — the `tabId` property travels
  with the widget regardless of its visual position.
- `QMainWindow` (plain, not subclassed) → `QSplitter(Qt::Horizontal)` →
  `QTreeView` (project tree) + `QTabWidget` (tab strip,
  `setMovable(false)` still set — tab reorder still explicitly disabled,
  this plan's G2 task). One `QPlainTextEdit` per open tab.
- **No class in the codebase uses `Q_OBJECT`/moc today.** `EditorTabs :
  public QObject` only uses lambda-based `connect()`. `build.rs` never
  invokes `moc`. Line numbers, syntax highlighting, and ADS are all
  first-time moc consumers — new build-system surface, not just new app
  code.
- `crates/ui-shell/build.rs`: `cxx_qt_build::CxxQtBuilder` with
  `.qt_module("Widgets")`/`.qt_module("Gui")` only, no `Quick`/`Qml`.
- `editor_core::Document`'s dirty flag is the single source of truth
  (ADR-0003): `QTextDocument::modificationChanged` forwards into
  `AppSession::set_tab_dirty`; the view reads it back, never keeps its
  own authoritative copy.
- `project-model::watcher` now exposes `is_structural_change` (moved out
  of `bridge.rs` — domain logic, not adapter translation) and re-exports
  `notify::EventKind`, so `ui-shell` no longer depends on `notify`
  directly.
- `project-model` persists exactly one thing outside `AppSession`'s
  in-memory state: last-opened project path, as one plain-text line
  (`persist_last_project`/`read_last_project`, explicitly not
  serde/toml/json). `default_config_dir()` = `dirs::config_dir().join("ide")`.
  No `serde`, `toml`, `tree-sitter`, MCP/JSON-RPC, or `tokio` anywhere yet.
- Docker cross-builds Windows via MXE + pinned mingw-w64. Any new vendored
  C++ (ADS) must build under both the Linux and Windows-cross toolchains
  or the Windows artifact silently regresses.

## Phase 0 (prerequisite): `app-core` refactor — complete

[ADR-0002](decisions/0002-application-layer-and-humble-view.md),
[ADR-0003](decisions/0003-ffi-conventions.md), and
[`layering.md`](layering.md) were accepted as binding architecture while
this plan was being written, and the refactor they describe — Qt-free
`app-core` (`AppSession`, `TabId(u64)`, typed `AppError`), a thin
`bridge.rs` adapter, and a humble `main_window.cpp` view — has since
landed and been committed. `layering.md`'s "known debt" section is
cleared. `cargo test --workspace` passes for the three Qt-free crates
(62 tests) and `cargo tree` confirms no Qt leakage into
`editor-core`/`project-model`/`app-core`. See "Current-state facts"
above for the resulting shape.

One thing landed better than originally scoped: `TabId` identity lives
as a widget property looked up dynamically, not a maintained
`TabId → index` map — so tab reordering (G2 below) needs no identity
bookkeeping at all, just enabling the widget's built-in drag support.

All feature tasks below (C/L/G/T/S/Y/D/M) build on this architecture, not
on the old bridge.rs/int-index/`QString` one. In particular:

- **G2 (tab reordering)** is now just `setMovable(true)` — no
  `TabList::move_tab` or index-resync fix needed, per the note above.
- **L1/L2 (Exit, Save As)** and all quick wins route through `AppSession`
  commands and typed `FfiResult`/`FfiOpenResult`s, identifying tabs by
  `TabId` rather than index, from the start.
- **M3–M5 (MCP tool wiring)** dispatch into the same `AppSession`
  commands the UI adapter calls — MCP tool handlers become another thin
  adapter over `AppSession`, exactly mirroring `bridge.rs`'s role, still
  queued onto the Qt thread via `CxxQtThread` for any command that must
  emit a UI-visible signal. Tools identify tabs/buffers by `TabId` or
  path, never by tab-strip index.
- **C1 (`app-config` crate)** is unaffected by Phase 0 — it's a sibling
  Qt-free crate, not part of the tree/document orchestration `app-core`
  owns.

## Architecture decisions

| # | Decision | Why |
|---|---|---|
| A1 | New Qt-free crate `app-config` (serde+toml) for structured settings | Mirrors `project-model`'s Qt-free/unit-testable pattern; the existing single-line persistence doesn't scale to theme/font/colors/recent-projects/window-geometry. `serde`+`toml` is a warranted new dependency, not hand-rolled parsing. |
| A2 | Docking: vendor **Qt Advanced Docking System (ADS)** as a git submodule, compiled via extended `build.rs` (moc + cc), CMake-via-`cmake`-crate as fallback only if that proves unworkable | Hand-rolling a docking engine (drag-to-dock previews, floating/pinned/auto-hide panels, layout persistence) is a multi-month reinvention of a solved, battle-tested library. Try direct moc+cc integration first to keep one build system (Cargo); fall back to a second (CMake) only if proven necessary. |
| A3 | Theming: QSS-based engine, Darcula + Light themes, Material-inspired palette; editor text colors via `QPalette` (separate from chrome QSS); icon theming explicitly out of scope | QSS is what the architecture overview already named; `QPalette`-only can't restyle scrollbars/tabs/menus. JetBrains itself splits "UI Theme" from "Editor Color Scheme" — same split here avoids the per-editor override fighting the global stylesheet cascade. |
| A4 | MCP transport: **local Streamable-HTTP JSON-RPC on 127.0.0.1**, port + short-lived auth token written to a discovery file in `default_config_dir()` | The IDE is already running when an agent attaches — stdio (subprocess-owns-lifecycle) is the wrong shape. A domain socket lacks a clean Windows story. Streamable HTTP is what off-the-shelf MCP clients already speak with zero custom transport code. Loopback + per-launch token avoids an unauthenticated open door. |
| A5 | Syntax highlighting: **tree-sitter**, wrapped by new Qt-free crate `syntax-core`, driven by a `QSyntaxHighlighter` subclass in `ui-shell/cpp` | User-directed. Grammars are added as crates one at a time, matching the existing "add crates when the work starts" discipline. |
| A6 | v1 re-tokenizes the whole buffer on every highlight call — no tree-sitter incremental `InputEdit` reparse yet | Deliberate ceiling. Upgrade path: a stateful `Highlighter` holding a persistent `tree_sitter::Tree` per document, using `.edit()` + incremental `.parse()`, once large-file typing latency is actually measured as a problem. |
| A7 | New `recent_projects` list lives in `app-config`; the existing `last-project.txt` mechanism is left untouched, not migrated | The old mechanism is shipped and working; consolidating it into the new TOML file is churn with a migration-compatibility cost for no user-facing benefit. |

**ADRs owed** (write once the relevant spike lands): A2 (ADS build
integration — once Task D1 confirms which approach works, on both Linux
and Windows-cross), A4 (MCP transport — once Task M1 confirms
feasibility).

## Crate/file structure

```
ide/
  Cargo.toml                    # add app-config, syntax-core, mcp-server, tree-sitter-* to workspace
  crates/
    editor-core/                # unchanged boundary: Qt-free
    project-model/               # unchanged boundary: Qt-free; unchanged persistence format
    app-config/                  # NEW, Qt-free — Settings struct, serde+toml load/save, recent-projects
    syntax-core/                 # NEW, Qt-free — tree-sitter wrapper, language registry, highlight()
    mcp-server/                  # NEW, Qt-free — MCP transport + tool dispatch over a channel interface
    ui-shell/                    # only crate touching cxx-qt/Qt — grows: settings dialog, docking,
                                  # theming, gutter/highlighter C++ classes, wires mcp-server to QObjects
    app/                         # unchanged: thin binary wiring crate
  third_party/
    qt-advanced-docking-system/  # NEW — vendored ADS source (git submodule, pinned release tag)
  docs/architecture/decisions/
    0002-*.md                    # owed: Qt build integration (cxx-qt-build + vendored ADS + moc)
    000N-mcp-transport.md        # owed: MCP transport choice
```

`app-config` and `syntax-core` stay Qt-free like `editor-core`/`project-model`
(unit-testable with plain `cargo test`). `mcp-server` also stays Qt-free
and must not depend on `ui-shell` (would be circular) — it exposes an
`EditorCommands` channel/trait pair that `ui-shell` implements, translating
each command into a closure queued onto `ProjectTreeModel`'s/
`DocumentManager`'s existing `CxxQtThread` handle — **reusing the exact
cross-thread pattern `start_watcher()` in `bridge.rs` already uses for the
filesystem watcher**, not inventing a second mechanism.

## Per-feature design

**Settings window** (`app-config::Settings`: theme, editor_font_size,
editor_font_family, editor_colors, recent_projects, window_geometry,
window_state — all `#[serde(default)]` so old/partial files don't
hard-fail). UI: new `ui-shell/cpp/settings_dialog.{h,cpp}`, `QDialog` with
`QListWidget` category list + `QStackedWidget` detail pane (JetBrains
split-pane pattern). Two categories only for this pass: **Appearance**
(theme combo, live-applies via `qApp->setStyleSheet()`) and **Editor**
(font size/family, color pickers via `QColorDialog`, applied via
`QPalette`/`QFont` to open editors). `File > Settings...` (`Ctrl+,`). No
placeholder categories with nothing behind them.

**Dockable windows**: vendor ADS as a git submodule; extend `build.rs` to
invoke `moc` over ADS headers and add ADS `.cpp`+generated `moc_*.cpp`
into the same `CxxQtBuilder` compilation (primary path); fall back to the
`cmake` crate building ADS's own CMakeLists into a static lib only if that
fails. Spike (Task D1) on Linux first, then verify on the Windows MXE
cross-build stage, **before** any application code depends on ADS.
Migration scope is deliberately limited: sidebar tree becomes one
`CDockWidget`, the tab-strip area becomes a second `CDockWidget` (still
one `QTabWidget` inside it, not one dock widget per open file) —
replacing the current `QSplitter` as central widget. Room left for future
dock widgets (search, run console, MCP activity log). Layout persists via
`CDockManager::saveState()/restoreState()` through the same `window_state`
settings field used for window geometry.

**Theming**: `darcula.qss` / `light.qss` (Material-inspired palette,
explicit named color constants, not scattered hex), applied via
`qApp->setStyleSheet()` at startup and live on switch. Editor text colors
via `QPalette`, separate from chrome QSS. Font size via
`QFont::setPointSize()`, not QSS. Icon theming explicitly deferred
(separate design-asset task).

**Line numbers**: new `ui-shell/cpp/code_editor.{h,cpp}` — `CodeEditor :
public QPlainTextEdit` (`Q_OBJECT`) + sibling `LineNumberArea : public
QWidget`, standard Qt "Code Editor" example pattern (`resizeEvent`
override, gutter repaint on `blockCountChanged`/`updateRequest`/
`cursorPositionChanged`). Every `new QPlainTextEdit(tabWidget_)` in
`EditorTabs::onTabOpened` becomes `new CodeEditor(tabWidget_)` — drop-in,
since existing `qobject_cast<QPlainTextEdit*>` call sites keep working.
**Sequenced before ADS** — smallest possible surface to first prove `moc`
integration in `build.rs` works at all.

**Tab reordering**: `setMovable(true)` on `main_window.cpp`'s
`QTabWidget`. No adapter or `app-core` change needed — per "Phase 0"
above, `TabId` already rides as a dynamic property on each page widget
and is looked up by scanning, not by a maintained index map, so a
drag-reorder can't desynchronize anything. This is now a one-line task.

**Syntax highlighting foundation**: `syntax-core` — `Language` enum
(Rust, Json, PlainText) + extension map (shared later with the status
bar), `HighlightSpan`/`TokenKind`, `highlight(language, text) ->
Vec<HighlightSpan>` backed by `tree-sitter` + `tree-sitter-rust` +
`tree-sitter-json` (two starter grammars only). Bridge: a plain `cxx`
free function `highlight_line` (no QObject state needed — pure per-call
function). `ui-shell/cpp/syntax_highlighter.{h,cpp}`: `SyntaxHighlighter :
public QSyntaxHighlighter` overriding `highlightBlock`, mapping spans to
`QTextCharFormat` colored from the active theme. One instance per
`CodeEditor`, language resolved from file extension. v1 is whole-buffer
re-tokenize per block-changed event, marked in code: `// ponytail:
full-buffer tree-sitter parse per highlightBlock call, O(document) per
edit. Upgrade: persistent tree_sitter::Tree per Document + InputEdit
incremental reparse once large-file typing latency is measured and found
wanting.`

**MCP server foundation**: `mcp-server` crate, Streamable-HTTP loopback
transport (A4). Spike `rmcp` SDK vs. hand-rolled JSON-RPC over
`axum`/`tokio` first (Task M1) — pick whichever is actually viable, don't
commit sight-unseen. This is the one place `tokio` enters the workspace
(justified: Streamable-HTTP is inherently async I/O). `run_app()` spawns
the MCP listener on its own thread after building the main window,
parallel to how the filesystem watcher is spawned. Every tool call
becomes a closure queued via the existing `CxxQtThread` pattern; a
`oneshot` channel carries the response back.

First-slice tools (read + write): `list_project_tree`,
`list_open_buffers`, `read_buffer(path)`, `open_file(path)`,
`edit_buffer(path, content)` (full-content replace for v1, mirroring
`AppSession::save_tab`'s existing contract — no new range-based edit
protocol yet), `save_buffer(path)`, `get_cursor_position(path)` (needs a
small new `AppSession::cursor_position(TabId)` command + adapter
invokable, since cursor state currently lives only in the widget; MCP
resolves `path` to `TabId` the same way `AppSession::find_tab_by_path`
already does internally). Every write path routes through the same
`AppSession` commands the UI adapter calls — no parallel MCP-only
mutation path.

**Quick wins**:
- *Save As* — new `AppSession::save_tab_as(TabId, PathBuf)` command
  (reuses `Document::set_path` + existing `save()`), exposed as a
  `DocumentManager::saveTabAs(tabId, path)` invokable; watcher already
  picks up the new file for free.
- *Exit + window geometry* — new `IdeMainWindow : public QMainWindow`
  subclass with `closeEvent()` reusing `EditorTabs::confirmCloseTab` per
  tab; `Exit` menu calls `window->close()` so both paths share one prompt
  flow; geometry/state saved to `app-config::Settings` on close (state
  field doubles for ADS layout persistence).
- *Recent projects* — `File > Recent Projects` submenu from
  `app-config::Settings::recent_projects`, pushed-to on successful
  `open_folder`.
- *Status bar* — line:col (from `CodeEditor::cursorPositionChanged`),
  static "UTF-8" label (accurate — only UTF-8 is supported today), current
  file's language (reuses the `syntax-core` extension map).
- Explicitly not doing: encoding detection/conversion, a Keymap settings
  category with nothing configurable behind it yet.

## Sequencing

Line numbers (moc-proving) before ADS (moc-heavy). `app-config` early
since theming/geometry/recent-projects all depend on it. MCP last among
major features (depends only on existing `DocumentManager`/
`ProjectTreeModel` invokables, nothing else here). Task groups: **C**
(app-config) → **L** (quick wins depending on C) / **G** (gutter + tab
reorder) → **T** (theming) → **S** (settings dialog, depends on C+T) →
**Y** (syntax highlighting, depends on G's proven moc pattern) → **D**
(docking, gated on D1 spike) → **M** (MCP, gated on M1 spike). Groups can
run in parallel except where noted.

## Task breakdown

| # | Task | Deliverable | Verification |
|---|---|---|---|
| C1 | Scaffold `app-config` crate | `Settings` struct (serde+toml), load/save, default-on-missing-file | `cargo test -p app-config`: round-trip, missing file → defaults, old/partial field doesn't panic |
| C2 | Recent projects | push/dedupe/cap logic in `app-config`; `open_folder` success pushes to it; `File > Recent Projects` submenu | `cargo test -p app-config`; manual: open 3 folders, submenu correct order, click reopens |
| L1 | `IdeMainWindow` + Exit + geometry persistence | subclass with `closeEvent()` reusing `confirmCloseTab`; Exit calls `window->close()`; geometry/state persisted | Manual: dirty tab + Exit prompts correctly; resize/move, relaunch, geometry restored |
| L2 | Save As | `AppSession::save_tab_as` command + `DocumentManager::saveTabAs(tabId, path)` invokable; menu wired | Manual: Save As inside project — appears in tree via watcher, tab retitles, subsequent Ctrl+S targets new path |
| L3 | Status bar | `QStatusBar`: line:col, UTF-8, language (needs Y1) | Manual: switch between `.rs`/`.json` tabs, labels correct |
| G1 | `CodeEditor` gutter + first moc integration | `build.rs` invokes `moc` over `code_editor.h`; `CodeEditor`+`LineNumberArea`; `onTabOpened` uses `CodeEditor` | `cargo build` produces moc'd code; manual: gutter shows correct numbers, updates on scroll/edit; existing `qobject_cast` sites still work |
| G2 | Tab reordering | `setMovable(true)` in `main_window.cpp` — no `app-core`/adapter change needed (see "Phase 0" above) | Manual: drag-reorder tabs; dirty indicators, close, and external-change prompts still target the correct (moved) tab |
| T1 | QSS engine + Darcula | `.qss` loading mechanism; darcula default | Manual: app launches dark-themed, chrome restyled |
| T2 | Light theme + live switch | `light.qss`; `setStyleSheet()` live switch | Manual: switch theme, restyles without restart |
| S1 | Settings dialog shell | category list + stacked detail pane; Appearance page wired to T2 + persisted | Manual: change theme, OK, relaunch persists; Cancel discards |
| S2 | Editor colors + font settings | font size/family, color pickers; applied live + persisted | Manual: change settings, all open editors update live, persists across relaunch |
| Y1 | `syntax-core` crate + Rust/JSON grammars | `Language`/extension map, `highlight()` via tree-sitter | `cargo test -p syntax-core`: known snippets → expected spans |
| Y2 | `SyntaxHighlighter` QObject | moc'd `QSyntaxHighlighter` subclass, `highlight_line` bridge fn, attached per `CodeEditor`, theme-sourced colors | Manual: `.rs`/`.json` visibly highlighted; `.txt` opens unhighlighted, no crash |
| D1 | ADS build-integration spike | submodule pinned; primary moc+cc attempted, fallback only if needed; minimal `CDockWidget` renders | Builds/renders on Linux **and** on Windows MXE stage — go/no-go gate |
| D2 | ADS ADR | write up D1's outcome | Reviewed |
| D3 | Migrate sidebar+editor to ADS dock widgets | `CDockManager` replaces `QSplitter`; tree + tab-strip each one `CDockWidget` | Manual: float/redock each panel; tab behavior (incl. G2) unaffected |
| D4 | Dock layout persistence | `saveState()/restoreState()` through `window_state` | Manual: float sidebar, relaunch, restored |
| M1 | MCP transport spike | `rmcp` vs hand-rolled evaluated; one lands with a no-op tool | A minimal MCP/curl client calls the no-op tool and gets a response |
| M2 | MCP ADR | write up M1's decision | Reviewed |
| M3 | `EditorCommands` channel + thread wiring | `mcp-server` channel/trait; listener thread spawned in `run_app()`; wired via `CxxQtThread::queue()` | MCP `list_open_buffers` reflects UI-opened tabs in the same process |
| M4 | First-slice read tools | `list_project_tree`, `list_open_buffers`, `read_buffer`, `get_cursor_position` (+ new `AppSession::cursor_position(TabId)` command) | MCP client reads match visible UI state |
| M5 | First-slice write tools | `open_file`, `edit_buffer`, `save_buffer` via existing `AppSession` commands (same ones the UI adapter calls) | MCP client writes visibly affect UI; round-trips to disk match manual Ctrl+S |

## Verification approach

Each task verifies independently per the table above (`cargo test -p
<crate>` for Qt-free crates, manual UI pass for `ui-shell` changes — no
headless Qt test runner exists in this repo, consistent with the MVP
plan's approach). Before declaring the whole plan done: full manual smoke
pass covering every US from the MVP plus every new item here (open/edit/
save, tree ops, tab drag-reorder, line numbers on a real file, both
themes, settings persistence across relaunch, dock float/redock, a `.rs`
and `.json` file syntax-highlighted, and an external MCP client driving
open/edit/save on a running instance) — on both Linux and the Windows
cross-build artifact, matching the MVP plan's Task 11 cross-platform smoke
test.

## Open questions carried forward

- Exact pinned ADS release tag, and moc+cc vs. cmake-fallback build path —
  resolved by D1.
- `rmcp` vs. hand-rolled MCP stack — resolved by M1.
- QSS via `.qrc`/`rcc` vs. install-relative directory — resolved during
  T1.
- Whether tree-sitter incremental reparse becomes necessary — revisit only
  once measured.
