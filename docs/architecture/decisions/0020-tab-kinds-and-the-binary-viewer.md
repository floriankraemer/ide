# 0020. Tab kinds and the read-only binary viewer

## Status

Accepted.
Implemented as tasks B1–B5 of [the large-files and binary-viewer plan](../large-files-and-binary-viewer-plan.md).

## Context

Clicking a binary file in the project tree produced a dialog and nothing else.
`AppSession::open_file` sniffed the first 8 KB, and anything that looked binary was rejected with `AppError::BinaryFile` before a tab could be created.
That was the right call while every tab was a text document — opening a PNG in a `QPlainTextEdit` shows mojibake and invites the user to save it back, corrupting the file — but "you cannot look at this" is a poor answer for a tool whose job is to show you what is in your project.

Three things had to be decided, and none of them is display logic.

**A tab had one kind, implicitly.**
`TabEntry` was exactly `{ id: TabId, doc: Document }`, and the view assumed every page in a tab group was a `CodeEditor`.
Nothing in the codebase had ever needed a second kind of tab: every non-text surface in the product — the terminal, Problems, Search Results, Class View — is an ADS dock beside the editor area, and Settings is a modal dialog. A binary viewer is the first page in the editor area that is not a text document.

**The sniff decided two different things at once.**
`looks_binary_file` folded "this file is binary" and "this file could not be read at all" into the same `Ok(true)`, and `open_file` folded them again with `.unwrap_or(true)`.
While both answers produced the same dialog that was merely misleading. Once binary files open, it becomes wrong: a deleted or unreadable file would open an empty hex view instead of saying what actually went wrong.

**A binary can be far larger than a text file.**
Whatever showed the bytes could not assume it may read the whole file, the way `Document::open` reads a text file into a rope.

## Decision

### A tab has an explicit kind, decided in Rust

`TabEntry` now holds a `TabContent` — `Text(Document)` or `Binary(BinaryFile)` — and `TabKind` crosses the FFI seam as a stable numeric code (0 text, 1 binary) alongside the existing `AppError` codes.

Everything a tab needs regardless of kind is answered by `TabContent`: path, title, rename retargeting, delete flagging.
That is what keeps the change small — the tab strip, session restore, the external-change watcher, "focus, don't duplicate", and the tree-driven rename and delete all work on a binary tab without knowing it exists.
Only the genuinely text-only commands — edit, save, save-as, reload, dirty state — have to care, and they say so with a new typed error, `AppError::NotATextTab` (code 9), rather than pretending the tab is missing.

The view asks `tabKind(tabId)` and builds a `CodeEditor` or a `HexViewer`.
It does not decide the kind from the path or the bytes: that is a rule, and rules do not live in the view (ADR-0002).

The `HexViewer` is not a `QPlainTextEdit`, so every `qobject_cast<QPlainTextEdit *>` and `qobject_cast<CodeEditor *>` in `main_window.cpp` already skips it, and every `currentEditor()` call site already null-guards — Find, Find Usages, Save, Save As, the LSP wiring and diagnostics are inert on a hex tab without a single new branch.
The two loops that must reach every page regardless of kind — the editor font and the editor colours — got an explicit `forEachHexViewer` counterpart, because silently not applying the user's font to one kind of tab is exactly the sort of drift that goes unnoticed.

`AppError::BinaryFile` and its code 3 are retained but never returned.
The numeric codes are an append-only FFI contract (ADR-0003) pinned by a test; removing a variant would renumber nothing today but invites it later.

### The hex formatting lives in `editor-core`

`editor_core::hex` decides the offset format, the byte grouping, which bytes count as printable, what replaces the ones that don't, and how a short final row is padded so the ASCII column stays aligned.
`FfiHexRow` carries three ready-to-paint strings, and the widget lays them out in three columns.

It lives in `editor-core` rather than a new crate because that crate already owns the other half of this question — `binary_detect`, which decides what counts as binary in the first place. Splitting the two across crates would put one rule in each.

### The viewer is read-only, and reads only what it shows

`BinaryFile` keeps the file open and seeks to the window it is asked for; `hexRows(tabId, firstRow, count)` is pulled per repaint for the visible rows only.
Nothing loads the file into memory, so a multi-gigabyte binary costs the same to open and scroll as a small one — which is the whole reason not to reuse the text path.

Read-only is a deliberate stopping point, not an oversight.
Hex *editing* needs a byte buffer with its own dirty state, undo, and save semantics, parallel to `Document` and sharing none of its code; and overwrite-vs-insert semantics are a real design question for a file format-unaware editor.
None of that is needed to answer "what is in this file", which is what the dialog was refusing to do.

## Consequences

- Binary files open instead of erroring. `CODE_BINARY_FILE` is dead weight kept for contract stability.
- An unreadable file now reports the real `io::Error` (`CODE_OPEN_FILE`), not "is a binary file".
- The editor area has two kinds of page. A third (an image preview, a diff view) now has a place to plug in: add a `TabKind` variant, a `TabContent` variant, and a widget.
- Session restore needs no schema change — it persists bare paths, and a binary path re-opens as a hex tab.
- The viewer's scrollbar counts rows in an `int`, so it tops out around 34 GB. Noted in the code; a proportional scrollbar is the fix if that ever matters.
- No selection or copy in the hex view yet. Deliberate; add when someone asks.
