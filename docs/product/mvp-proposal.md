# MVP Proposal: Minimal Text Editor Shell

## Status

Draft — ready for engineering feasibility review.

## Problem / Goal

We are building a cross-platform IDE (Rust core + Qt6 UI, see
[ADR 0001](../architecture/decisions/0001-core-tech-stack.md)) and need a
first working shell to validate the Rust↔Qt integration and give the team
something real to build on.

The MVP goal is narrow: prove we can open a project folder, browse its
files in a sidebar, and edit/save text in a tabbed editor, with a
PHPStorm-like window layout, on Linux and Windows.
No language intelligence, no plugins, no VCS — just a working text editor
shell.
This validates the `cxx-qt` bridge and the `QAbstractItemModel`-backed
project tree early, before any higher-risk subsystem (LSP, debugger, WASM
plugins) is built on top of it.

## Users

Early internal/contributor audience: developers evaluating or contributing
to the IDE itself.
Not yet a target for end-user daily-driver adoption — this MVP is a
technical and UX foundation, not a release.

## In-scope user stories

### US-1: Open a folder as a project

**As a** developer
**I want** to open a folder as a project
**So that** the IDE knows what file tree to show me

**Acceptance criteria**
- [ ] Given no project is open, when I choose "Open Folder" (menu and/or
      startup dialog) and pick a directory, then that directory becomes the
      active project and its contents appear in the left sidebar.
- [ ] Given a project is already open, when I open a different folder, then
      the previous project's tree and open tabs are replaced (single project
      open at a time).
- [ ] Given the app is relaunched, when it starts, then it reopens the last
      project folder automatically.
- [ ] Given a folder no longer exists or isn't readable, when I try to open
      it, then I see a clear error and the previous state (if any) is
      unchanged.

### US-2: Browse project files in a sidebar tree

**As a** developer
**I want** a left-sidebar file/folder tree for the open project
**So that** I can navigate the project's files, PHPStorm-style

**Acceptance criteria**
- [ ] Given a project is open, when the sidebar renders, then it shows the
      project's folder/file hierarchy rooted at the project folder.
- [ ] Given a folder node, when I click its expand/collapse control, then its
      children show or hide; state persists for the session (re-expanding
      after collapsing a parent restores prior expand state where reasonable).
- [ ] Given a file node, when I click it, then the file opens in the editor
      (new tab, or focuses an existing tab for that file — see US-3).
- [ ] Given the tree, when a folder has no children, then it renders without
      an expand affordance (no empty-expand dead end).
- [ ] Given files change on disk outside the IDE (added/removed/renamed),
      when the change occurs while the project is open, then the tree
      auto-refreshes via a filesystem watcher (no manual action required).

### US-2b: Create, rename, and delete files/folders from the tree

**As a** developer
**I want** to create, rename, and delete files/folders from the sidebar
**So that** I can manage my project's structure without leaving the IDE

**Acceptance criteria**
- [ ] Given a folder node (or the project root), when I use its context menu,
      then I can create a new file or new folder inside it.
- [ ] Given a file or folder node, when I use its context menu, then I can
      rename it in place.
- [ ] Given a file or folder node, when I use its context menu and choose
      delete, then I'm asked to confirm before the filesystem deletion
      happens (folders delete recursively with a clear warning).
- [ ] Given a file open in a tab is deleted or renamed via the tree, then
      the corresponding tab reflects it (renamed tab title, or a clear
      "file deleted" state) rather than silently pointing at a stale path.
- [ ] Given a binary/non-text file, when I click it in the tree, then I see
      a "cannot open" message — no attempt to open it as text.

### US-3: Tabbed text editor

**As a** developer
**I want** to open multiple files in tabs and switch between them
**So that** I can work across several files without losing my place

**Acceptance criteria**
- [ ] Given I click a file in the tree, when it's not already open, then a
      new tab is created showing its contents.
- [ ] Given a file is already open in a tab, when I click it again in the
      tree, then the existing tab is focused, not duplicated.
- [ ] Given multiple tabs are open, when I click a tab header, then that
      tab's content becomes visible and editable.
- [ ] Given a tab, when I click its close control, then the tab closes; if
      it has unsaved changes, I'm prompted to save, discard, or cancel.
- [ ] Given the editor area, standard text editing works: type, delete,
      select, cut/copy/paste, undo/redo.
- [ ] Given a very large file (e.g. tens of MB), opening and typing remain
      responsive (no hard multi-second freeze) — the buffer is backed by a
      rope (`ropey` or equivalent), not a naive `String`/`Vec`, from MVP.
- [ ] Given more tabs are open than fit the window width, then the tab strip
      scrolls horizontally (not an overflow menu or wrapping).
- [ ] Given a file open in a tab is modified externally (another editor,
      `git checkout`, etc.), then I'm prompted to reload or keep my version
      — detected via the same filesystem watcher backing tree live-refresh.

### US-4: Edit and save a file

**As a** developer
**I want** to edit a file's text and save it with Ctrl+S
**So that** my changes persist to disk

**Acceptance criteria**
- [ ] Given unsaved edits in the active tab, when I press Ctrl+S (or use a
      Save menu action), then the file's content on disk is overwritten with
      the editor's content.
- [ ] Given a tab with unsaved changes, then its tab header shows a distinct
      unsaved-changes indicator (e.g. a dot/asterisk).
- [ ] Given a save succeeds, then the unsaved indicator clears immediately.
- [ ] Given a save fails (e.g. disk full, permissions, file removed
      externally), then I see a clear error and the tab keeps its unsaved
      state (no silent data loss).
- [ ] Given I try to close the app or the project with unsaved tabs open,
      then I'm prompted to save, discard, or cancel per the standard
      unsaved-changes flow.

### US-5: Native-looking window and menus

**As a** developer
**I want** the app to look and behave like a native desktop application
**So that** it feels at home on my OS instead of like a web app

**Acceptance criteria**
- [ ] Given the app runs on Linux or Windows, then window chrome, menus, and
      dialogs use native Qt6 widgets (per ADR 0001) — no custom-drawn chrome.
- [ ] Given the app window, then a standard menu bar exists with at minimum:
      File (Open Folder, Save, Save As, Exit) and Edit (Undo, Redo, Cut,
      Copy, Paste).
- [ ] Given the overall layout, then it matches the PHPStorm-like shape:
      sidebar on the left, tabbed editor filling the main area — no other
      panels required for MVP.
- [ ] Given the app runs on Linux and on Windows, then core flows (open
      folder, browse tree, open/edit/save tabs) work equivalently on both.

## Out of scope (explicitly deferred)

| Item | Why deferred |
|---|---|
| LSP / autocompletion / diagnostics | Requires the LSP client and a much larger UI surface (completion popups, inline diagnostics); validate the editor shell first. |
| Debugging / debugger adapters | Depends on a stable editor/project model existing first; DAP integration is its own large subsystem. |
| Plugin system (native or WASM) | Architecture supports it (ADR 0001), but no plugin loading is needed to prove the editor shell; adding it now would pull in the host-API design prematurely. |
| Syntax highlighting | Not required for MVP. May be considered a **stretch goal only** if trivially available — not a commitment, and not something to design around. |
| VCS/git integration | Separate subsystem (Project/VCS model); no MVP user story depends on it. |
| Settings/theming UI | Sane Qt6 defaults only; a configuration UI is unnecessary complexity for a shell whose purpose is validating the core loop. |
| Multi-window, split panes, project-wide search/replace, terminal panel | All expand the UI surface significantly; none are needed to prove open-folder → browse → edit → save. |
| macOS support | ADR 0001 targets three OSes eventually; this proposal scopes macOS out of the *MVP* per the user's ask (Linux + Windows only) — should be revisited immediately after MVP, not dropped from the roadmap. |

## Open questions — resolved

1. **File tree live refresh**: auto-refresh via filesystem watcher. Tree
   stays in sync with external add/remove/rename without manual action.
2. **Large-file / large-project ceiling**: design for large files from day
   one — use `ropey` (or equivalent rope) for the text buffer, not a naive
   `String`/`Vec` buffer, even in MVP.
3. **Binary / non-text files in the tree**: clicking a binary file shows a
   "cannot open" message; no attempt to open as text.
4. **Tab overflow behavior**: scrolling tab strip (standard IDE behavior),
   not an overflow menu or wrapping.
5. **External file changes to an open tab**: detect and prompt (reload /
   keep) — reuses the same filesystem watcher backing the tree live-refresh
   (US-2/open question 1), applied per open file.
6. **Multiple top-level roots**: confirmed single-root for MVP per US-1 —
   do not generalize to multi-root during implementation.
7. **New file / new folder / rename / delete from the tree**: **in scope**
   for MVP — basic create/rename/delete via the sidebar tree (context menu
   + confirmation on delete), backed by real filesystem mutations.

### Scope note

Items 1, 2, 5, and 7 expand MVP beyond the original "minimalistic" framing:
a filesystem watcher (shared by tree refresh and open-tab external-change
detection) and a rope-based text buffer are now day-one dependencies, and
tree-based file management (new/rename/delete) adds a context-menu UI plus
filesystem-mutation code path. Architecture/engineering should size these
in the implementation plan rather than treat them as free additions.

## Next step

Once scope and open questions above are confirmed, this is ready for an
engineering breakdown (task-level backlog against the `cxx-qt`
Shell↔Buffer/Project bridge described in the
[architecture overview](../architecture/overview.md)).
