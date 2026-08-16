# MVP implementation plan: Minimal Text Editor Shell

## Status

Draft — implementation plan derived from the approved
[MVP proposal](../product/mvp-proposal.md), constrained by
[ADR 0001: core tech stack](decisions/0001-core-tech-stack.md) and the
[architecture overview](overview.md).
This document does not reopen ADR 0001.
It scopes the Rust workspace, the `cxx-qt` bridge, key crate choices, and
the Dockerized cross-compilation build for the MVP only.

Two decisions in this plan carry real reversal cost and should be promoted
to their own ADRs once accepted: the Qt build integration approach
(`cxx-qt-build` over CMake+Corrosion) and the Windows cross-compilation
strategy (aqtinstall over MXE or from-source Qt).
Recommend `tech-writer` split those into ADR-0002 and ADR-0003 after
`senior-software-engineer` confirms feasibility in the scaffold task
(Task 1) and the Docker Windows task (Task 8).

## 1. Cargo workspace layout

The overview.md module list includes LSP, debugger, and plugin-host
modules.
None of those are needed to prove open-folder → browse → edit → save, and
the MVP proposal explicitly excludes them.
This plan scopes the workspace down to four crates and does not create
placeholder crates for out-of-scope modules — an empty `lsp-client` crate
with no code is not a milestone, it is noise that `senior-software-engineer`
would have to maintain and re-justify later.
Add those crates when the work that needs them starts, not before.

```
ide/
  Cargo.toml                 # workspace root
  crates/
    editor-core/              # rope buffer, document/tab state, save logic
    project-model/            # project root, file tree state, fs watcher
    ui-shell/                 # cxx-qt bridge types (QAbstractItemModel impls, QObjects)
    app/                      # binary crate: wires the above into a QApplication
  docker/
    Dockerfile
  dist/                       # build output, gitignored
  docs/
```

Crate responsibilities:

- **`editor-core`** — `ropey`-backed document buffer, open/dirty/save state
  per tab, tab-list ordering.
  No Qt dependency.
  Unit-testable in isolation (open large file, edit, save, dirty-flag
  transitions) without a display or Qt runtime.
- **`project-model`** — single project root, directory tree snapshot,
  `notify`-based filesystem watcher, last-opened-project persistence.
  No Qt dependency either — the tree is a plain Rust data structure; only
  `ui-shell` knows it needs to look like a `QAbstractItemModel`.
- **`ui-shell`** — the `cxx-qt` bridge layer and Qt Widgets glue: the
  `QAbstractItemModel` implementation wrapping `project-model`'s tree, the
  `DocumentManager` QObject wrapping `editor-core`'s tab state, the main
  window/menu/tab-strip/tree-view widget wiring.
  This is the only crate that depends on `cxx-qt` and Qt6.
- **`app`** — thin binary crate: constructs the `QApplication`, constructs
  `project-model`/`editor-core` state, hands ownership to `ui-shell`, runs
  the Qt event loop.
  Kept separate from `ui-shell` so `ui-shell`'s bridge types stay a library
  target `cxx-qt-build` can codegen against cleanly.

Rationale for `editor-core` and `project-model` having no Qt dependency:
both are the parts of the system likely to be reused once LSP/indexing
work starts, and keeping them Qt-free means they can be unit-tested with
plain `cargo test`, without a Qt runtime or display server in CI.

## 2. `cxx-qt` bridge design

### Project tree → `QAbstractItemModel`

`project-model` owns the tree as a plain Rust arena (nodes indexed by ID:
path, name, is-dir, parent, children).
`ui-shell` defines a `#[cxx_qt::bridge]` type, `ProjectTreeModel`, that
implements the `QAbstractItemModel` trait `cxx-qt` provides (`rowCount`,
`columnCount`, `data`, `index`, `parent`) by reading that arena.

Filesystem watcher events arrive on `project-model`'s watcher thread and
update the arena.
For MVP, a changed-directory event triggers a scoped `beginResetModel` /
`endResetModel` around just the affected node's children, not a full-tree
reset and not per-row insert/remove diffing.
This is a deliberate simplification: per-row diffing (matching old vs.
new children to emit precise `beginInsertRows`/`beginRemoveRows` and
preserve exact scroll position and multi-select state) is real work for a
benefit the MVP doesn't need — a directory rarely has more than a few
hundred entries, so a scoped reset repaints fast enough that the user
won't perceive it as different from a diffed update.
Revisit if a real project directory (thousands of entries in one folder)
makes the scoped reset visibly janky.

### Tab / buffer state → Qt widgets

`ui-shell` exposes a `DocumentManager` `QObject` wrapping `editor-core`'s
tab list, with invokable slots (`openFile`, `closeTab`, `saveTab`,
`setActiveTab`) and signals (`tabOpened`, `tabClosed`,
`tabModifiedChanged`, `tabTitleChanged`, `externalChangeDetected`) that the
tab-strip widget and window title connect to.

The rope's role needs to be stated precisely, because "rope-backed buffer
from day one" and "Qt widget renders and edits the text" pull in different
directions:

- **Load**: file bytes are read into a `ropey::Rope` in `editor-core`.
  This is the part that matters for the "tens-of-MB file opens without a
  multi-second freeze" acceptance criterion — reading and slicing a rope
  for initial display is fast regardless of file size.
- **Live editing**: the actual `QPlainTextEdit` widget owns keystroke-level
  editing during a session, using its own internal `QTextDocument`.
  Marshalling every keystroke across the Rust/Qt FFI boundary into the
  rope and back would add latency to the one path where latency is most
  noticeable, for no benefit at MVP scope (no incremental syntax
  highlighting or LSP consuming the rope yet).
  Dirty-state tracking rides on `QTextDocument::modificationChanged`,
  bridged to `editor-core` via a `cxx-qt` signal so tab titles and the
  unsaved-changes indicator update without editor-core needing every
  keystroke.
- **Save**: on Ctrl+S, `ui-shell` pulls the full current text out of the
  widget, `editor-core` writes it to a fresh `Rope` and to disk, and that
  rope becomes the new "on-disk-equivalent" snapshot used for
  external-change comparison (below).

This is a scoped simplification, not an oversight: the rope backs
load/save and external-change diffing, not live keystrokes, because
`QPlainTextEdit` already owns fast native editing and re-deriving that in
Rust would be pure duplication right now.
The ceiling this accepts: if a future profiling pass shows `QPlainTextEdit`
itself struggling on very large single files during *typing* (not just
load), the fix is a custom rope-backed text-widget/document subclass —
real work, correctly deferred until there's evidence it's needed.

### Filesystem watcher → Qt signals (tree refresh and external-change prompts)

One `notify` watcher instance, owned by `project-model`, watches the
project root recursively on a background thread.
Both consumers — tree refresh and open-tab external-change detection — are
fed from that single watcher, per the MVP proposal's resolved open
question 5; there is no second watcher.

Because `notify`'s callback fires on a non-Qt thread, events cannot call
into Qt objects directly — Qt object methods must run on the Qt/GUI
thread.
`cxx-qt` provides exactly this bridge (a `CxxQtThread` handle obtainable
from a `QObject`, safe to hand to another thread, whose `queue()` method
marshals a closure onto the Qt event loop).
`project-model`'s watcher thread holds one such handle each for
`ProjectTreeModel` and `DocumentManager`, and on a filesystem event:

1. Updates its own tree arena.
2. Queues a scoped reset onto `ProjectTreeModel` via its `CxxQtThread`
   handle.
3. If the changed path matches an open tab's backing file, queues
   `externalChangeDetected(path)` onto `DocumentManager` via its handle;
   `ui-shell` connects that to the reload/keep prompt.

This avoids a shared-mutex model between the watcher thread and the Qt
thread — the only cross-thread communication is through `cxx-qt`'s
supported queuing mechanism, which is also what keeps this design free of
hand-rolled synchronization primitives.

## 3. Key crate choices

| Crate | Purpose | Why |
|---|---|---|
| `ropey` | Rope-backed text buffer | Mature, `unicode`-aware, line/char-indexed rope; used in production by other Rust editors (e.g. Helix); exactly what the MVP proposal names as the reference implementation. No custom rope — building one from scratch is exactly the kind of speculative infrastructure this plan avoids. |
| `notify` | Filesystem watcher | De facto standard cross-platform watcher for Rust (inotify on Linux, `ReadDirectoryChangesW` on Windows); actively maintained; single dependency serves both tree refresh and external-change detection, matching the MVP's single-watcher requirement. |
| `cxx-qt`, `cxx-qt-build`, `cxx` | Rust↔Qt bridge | Mandated by ADR 0001 — not a new decision here. |
| `dirs` | Locate platform config dir for "last opened project" persistence | Small, well-known crate for a genuinely platform-specific lookup (`%APPDATA%` vs. `~/.config`) that the standard library doesn't provide. Not worth hand-rolling per-OS path logic for one config value. |

Deliberately not adding: a serialization crate (`serde`/`toml`/`json`) for
the "last opened project" record.
It's a single path string — write and read it as one line of plain text.
Add a real config format when there is a second setting to store; there
isn't one in this MVP.

## 4. Dockerized cross-compilation strategy

**This is the highest-risk part of the plan.** Everything else here is a
fairly ordinary Rust+Qt desktop build; producing a Windows `.exe` via
mingw-w64 cross-compilation from a Linux Docker container, with Qt6 in the
loop, is the one place where the plan could stall on tooling rather than
on IDE logic.

### Windows Qt6-for-mingw approach

Three options were evaluated:

| Option | Why rejected / accepted |
|---|---|
| Build Qt6 from source for mingw-w64 inside the Docker image | Rejected as primary. Correct in principle — full control, exact version match with the Linux Qt6 build — but a from-source Qt6 build is multi-hour, disk-heavy, and requires maintaining cross-compiling CMake toolchain files for Qt itself. That cost is disproportionate to an MVP whose goal is validating the `cxx-qt` bridge, not owning a Qt build pipeline. Kept as the deepest fallback. |
| MXE (M Cross Environment), `qt6` package | Rejected as primary, kept as first fallback. MXE is built exactly for this (cross-building Windows libs from Linux) and its `qt6` package produces a matched mingw-w64+Qt6 pair, but Qt6 support in MXE has a weaker track record than its long-mature Qt5 support, and MXE tends to statically link, which compiles more from source and lengthens Docker image builds. Self-consistent and known-workable if the primary approach breaks. |
| `aqtinstall` fetching official prebuilt Qt6 `mingw_64` binaries | **Recommended.** `aqtinstall` (the community-standard headless installer for Qt's official binaries, widely used in CI) pulls the same prebuilt mingw Qt kit Qt Creator itself ships for Windows/MinGW builds. No multi-hour compile, official upstream binaries (not a third-party rebuild), and the rest of the Docker stage — mingw-w64 GCC, Rust's `x86_64-pc-windows-gnu` target — is still installed explicitly by us, so the Dockerfile stays fully auditable. |

**Decision: `aqtinstall` + official Qt6 `mingw_64` prebuilt binaries**, for
the reasons in the table above.

The single most likely failure mode: Qt's official `mingw_64` kit is built
against a specific MinGW-w64 GCC version (e.g. Qt 6.7's `mingw_64` kit
pairs with a GCC 13-based mingw-w64 toolchain).
The Docker image must pin a matching mingw-w64 GCC version rather than
whatever `apt` resolves by default, or linking will fail with ABI-mismatch
errors that look unrelated to Qt.
Pin both versions explicitly in the Dockerfile and bump them together.

**Fallback ladder**, in order:

1. Primary: `aqtinstall` + pinned official Qt6 `mingw_64` + matching
   pinned mingw-w64 GCC.
2. Fallback A — if the official kit's expected mingw-w64 GCC version
   becomes unavailable or incompatible with the Debian base image: switch
   to MXE's `qt6` package, which builds its own matched pair from source.
   Slower image builds, but the slowdown is a one-time cost per Qt version
   bump (cached as a Docker layer) and it is self-consistent by
   construction.
3. Fallback B — last resort: drop Windows from this MVP delivery
   temporarily, ship Linux-only, and track Windows as an immediate
   follow-up spike.
   This does not abandon Windows support — ADR 0001 still targets it — it
   only accepts that the Docker cross-build may need a dedicated spike
   outside the main MVP task sequence if both prior options prove
   unworkable in the time box.

### Dockerfile structure: one multi-stage file

**Decision: a single multi-stage `docker/Dockerfile`**, built with
`docker build --target linux-builder` and `--target windows-builder`
separately, rather than two Dockerfiles.

Reasoning: the two targets share almost everything up to the point where
they diverge — the same source `COPY`, the same pinned Rust toolchain
version, largely the same apt base layer.
Two separate Dockerfiles would duplicate that shared setup, and duplicated
version pins are exactly the kind of thing that drifts silently (Linux
build picks up Rust 1.8x, Windows build still pinned to 1.7x, and nobody
notices until a build breaks for one target only).
One file with named stages keeps the two builds provably using the same
base, and a single version bump touches one place.

Stage outline (illustrative — package lists and exact commands are
`senior-software-engineer`'s to fill in during Task 7/8):

```
FROM debian:bookworm-slim AS base
# shared: apt update, common build tools, pinned Rust toolchain via rustup

FROM base AS linux-builder
# + Qt6 dev packages (apt: qt6-base-dev, qt6-tools-dev, etc.)
# cargo build --release --target x86_64-unknown-linux-gnu -p app
# -> /out/ide-linux-x86_64

FROM base AS windows-builder
# + mingw-w64 (pinned GCC version) + rustup target add x86_64-pc-windows-gnu
# + aqtinstall: fetch pinned Qt6 mingw_64 kit into /opt/qt6-mingw
# cargo build --release --target x86_64-pc-windows-gnu -p app
# -> /out/ide-windows-x86_64.exe
```

Extract artifacts with `docker buildx build --target <stage> --output
type=local,dest=dist/` rather than building an image and `docker cp`-ing
out of a container — it's the supported BuildKit path for "I just want the
files," and keeps CI/local invocation identical.

## 5. Output artifacts

- `ide/dist/ide-linux-x86_64` — from the `linux-builder` stage.
- `ide/dist/ide-windows-x86_64.exe` — from the `windows-builder` stage.

`dist/` is build output, not a source directory — add it to
`.gitignore` when the workspace is scaffolded (Task 1).

## 6. Task-level breakdown

Sized for `senior-software-engineer` to implement as small, independently
verifiable increments.
Each task lists its deliverable and how to verify it before moving on.

| # | Task | Deliverable | Verification |
|---|---|---|---|
| 1 | Scaffold Cargo workspace | `Cargo.toml` workspace + four empty crates per §1, `.gitignore` including `dist/` and `target/` | `cargo build` succeeds across the workspace with stub `fn main() {}` / empty libs |
| 2 | Basic Qt window via `cxx-qt` | `app` binary opens a native, empty Qt6 Widgets main window with the File/Edit menu bar from US-5 (no functionality behind the menu items yet) | Window opens on Linux, native look, menu bar present |
| 3 | `editor-core`: rope buffer + tab state | `Document` type (rope-backed, load/save/dirty state), `TabList` type; no UI | `cargo test -p editor-core`: open small + large (tens of MB) file, edit, dirty-flag transitions, save round-trip |
| 4 | `project-model`: tree + open-folder | Project root type, directory tree snapshot (no watcher yet), "Open Folder" logic incl. last-opened-project persistence (US-1) | `cargo test -p project-model`: tree reflects a fixture directory; reopening restores last path |
| 5 | Sidebar tree model (`ProjectTreeModel`) | `cxx-qt` `QAbstractItemModel` wrapping `project-model`'s tree, wired into a `QTreeView` in the main window, click-to-open wired to a stub (US-2, minus live refresh) | Manual: open a real folder, tree renders, expand/collapse works, binary-file click shows "cannot open" (US-2b last item) |
| 6 | Tabbed editor (`DocumentManager` + tab strip) | `DocumentManager` QObject wrapping `editor-core`, tab strip widget (scrolling on overflow), click-tree-file opens/focuses tab, Ctrl+S save, unsaved indicator (US-3, US-4 minus external-change) | Manual: open multiple files, edit, switch tabs, save, verify dirty indicator and scrolling strip with enough tabs to overflow |
| 7 | File ops from the tree (US-2b) | Context menu: new file/folder, rename, delete (with confirmation, recursive for folders); open tabs reflect rename/delete of their backing file | Manual: create/rename/delete via context menu; delete/rename a file with an open tab and confirm the tab updates, not silently stale |
| 8 | Filesystem watcher integration | `notify` watcher in `project-model`, `CxxQtThread`-queued updates to both `ProjectTreeModel` (scoped reset) and `DocumentManager` (external-change signal + reload/keep prompt) per §2 | Manual: edit a file externally (another editor / `git checkout`) while open in a tab — prompt appears; add/remove a file on disk — tree updates without manual refresh |
| 9 | Docker: Linux build stage | `docker/Dockerfile` `linux-builder` stage per §4, produces `dist/ide-linux-x86_64` | `docker buildx build --target linux-builder --output type=local,dest=dist/` from a clean checkout produces a running binary |
| 10 | Docker: Windows cross-build stage | `windows-builder` stage per §4 (aqtinstall + pinned mingw-w64), produces `dist/ide-windows-x86_64.exe` | Same `buildx` invocation targeting `windows-builder`; binary runs under Wine or a real Windows machine/VM |
| 11 | Cross-platform smoke test | Manual pass of US-1 through US-5's acceptance criteria on both the Linux binary and the Windows `.exe` | All MVP acceptance criteria checked off on both binaries; any Windows-only defect logged, not silently patched around |

Tasks 1–8 have no dependency on Docker and should be developed and
verified with a plain local Rust+Qt6 dev setup; Docker only enters at
Task 9.
This keeps the highest-risk piece (§4) isolated to the end of the
sequence, so a Docker/Windows-cross-compile problem doesn't block routine
feature work on Tasks 1–8 in the meantime.

## Open questions carried forward

- Exact pinned Qt6 and mingw-w64 GCC versions — pick during Task 10, record
  in the Dockerfile comments and in ADR-0003 once written.
- Whether `QPlainTextEdit`'s native large-file handling actually meets the
  "no hard multi-second freeze" bar at the top end of "tens of MB" without
  further tuning (e.g. disabling line-wrap recalculation) — verify during
  Task 3/6, not assumed.
