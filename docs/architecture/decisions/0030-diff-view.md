# 0030. `DiffView`: one diff component, Git-free

## Status

Accepted

## Context

`docs/architecture/next-five-features-plan.md`'s F3 ("Git v1") lane needs one diff-rendering widget for four call sites: the refactor/rename/AI-apply preview, the project-wide Replace-in-Files preview, a future Git gutter, and a future diff tab.
`editor_core::diff` (line hunks over `imara-diff`, intra-line spans, ceilings) already landed as F3-1, deliberately Git-free — see that module's own doc comment.
Nothing consumed it yet.

Two of the four call sites already existed and were faking a diff:

- `RefactorController::onRefactorReady` built `RefactorPreviewDialog` rows whose `detail` column was `previewText(edit.new_text)` — the first line of one edit's replacement text, truncated to 80 characters, not a diff against what the file used to say.
- `SearchResultsPanel::replaceAll` had no preview at all: a `QMessageBox::question` stating a match/file count, then a direct write to disk with no way to see what would change and no undo.

The other two call sites — a Git gutter and a diff tab — need the `vcs-core` Git backend, which does not exist yet (F3-2 through F3-12d in the plan).
Building `TabKind::Diff` and its `diff_labels(TabId)` accessor now would add a tested code path with no real caller: nothing produces a diff tab until a `vcs.showDiff` action exists, and that action needs a repository to diff against.

## Decision

### 1. Diff computation stays in `editor_core::diff`; nothing here duplicates it

`DiffView` and every crate that feeds it call `editor_core::diff::{diff_lines, diff_inline}` and read the `Hunk`/`InlineSpan` types it already defines.
`lsp-core` gained a normal dependency on `editor-core` for this (`docs/architecture/layering.md`'s `lsp-core` row) — `lsp_core::diff_preview::file_diff` turns a pending refactoring's before text and `DocumentEdits` into the after text and its hunks, reusing `apply_to_text` and `diff_lines` rather than a second implementation of either.
`index-core` already depended on `editor-core`, so `index_core::TextIndex::preview_replacements` needed no new edge.

### 2. `DiffView` is a generic, reusable `QWidget` — two texts, hunks, spans, a language id

Constructor: `(leftText, rightText, hunks, spans, languageId)`.
Two read-only `QPlainTextEdit` panes, one shared vertical scroll (by fraction of each pane's own scrollbar range, since an insertion or deletion routinely gives the two sides different line counts), a thin painted ribbon per pane marking hunk ranges by colour, `QTextEdit::ExtraSelection`s for the intra-line spans, and F7/Shift+F7 to select the next/previous hunk on both panes and scroll it into view.
It knows nothing about where its two texts came from — no Git, no LSP, no search index.

`hunks` and `spans` cross the FFI seam as new plain-data structs in `ui-shell/src/bridge/ffi.rs` (`FfiHunk`/`FfiHunkKind`, `FfiInlineSpan`/`FfiDiffSide`), following the file's existing struct conventions.
A `Vec<T>` field is not a shape cxx supports on a shared struct, so `FfiFileDiff` (`path`, `old_text`, `new_text`) crosses on its own and hunks/spans are fetched by their own accessor calls (`pendingFileHunks`/`pendingFileSpans` and the `replacePreview*` equivalents) — the same "getter, not a payload struct" shape `pendingEdits()`/`pendingOps()` already established for the refactor preview.

`languageId` is threaded through but unused this slice: plain monospace text (`QFontDatabase::systemFont(QFontDatabase::FixedFont)`) is what both retrofitted call sites need, and wiring `SyntaxHighlighter` onto two read-only panes is a real lift a later pass can do without touching `DiffView`'s public shape.

`DiffView` is C++ and untested by design (`CLAUDE.md`: "C++ stays thin and is untested by design — if you feel you need a C++ test, the logic is in the wrong layer").
Every decision it paints — which lines changed, what changed within a line — was made in `editor_core::diff` or in the Rust code that fetches a file's diff; the widget only lays the answer out.

### 3. The refactor preview and the Replace-in-Files preview retrofit onto `DiffView` in this same slice

**Refactor/rename/AI-apply.** `LanguageService` gained `pendingFileDiff(path)`/`pendingFileHunks(path)`/`pendingFileSpans(path)`, computed lazily from the pending `lsp_core::EditPlan` when a file's row is selected — not eagerly for every file, since a refactoring can touch many.
`RefactorPreviewDialog` grew an optional `DiffProvider` callback and, when one is supplied, a `QSplitter` showing the selected row's `DiffView` beside the file tree.
`RefactorController::onRefactorReady` is the only caller that supplies one.
The AI chat panel's per-block Apply already routes through this same dialog (`ai_chat_panel.cpp`'s Apply button, tooltip: "Preview and apply this block through the refactoring preview") — retrofitting the dialog covers it with no separate AI-panel-specific preview.

**Replace in Files.** `index_core::TextIndex` gained `preview_replacements`, a non-mutating sibling of `replace_in_files` sharing its grouping/splice logic (`spliced_content`) but returning `FileDiffPreview { path, old_text, new_text, hunks }` instead of writing.
`SearchModel` gained `previewReplacements(...)` (async, mirroring `replaceInFiles`'s worker-thread shape) and per-file `replacePreviewDiff`/`Hunks`/`Spans` getters, computed eagerly for every file in one pass since `preview_replacements` already builds each file's new content in memory regardless.
`SearchResultsPanel::replaceAll` now shows the same `RefactorPreviewDialog` (with a `DiffProvider` backed by the eager preview) instead of a count-only `QMessageBox`, before writing.
Persistence is unchanged: confirming still writes straight to disk with no undo for a file that is not open — a real, separate, larger gap this slice does not close.

Both retrofits reuse the same dialog rather than inventing a second "list of files, per-file diff, confirm" idiom.

### 4. `TabKind::Diff` and a Git-triggered diff tab are deliberately deferred

A diff opened as its own tab (`vcs.showDiff` against a repository) needs the `vcs-core` Git backend, which is separate future work (F3-2 through F3-19 in the plan).
Building `TabKind::Diff`/`diff_labels(TabId)` now would ship a tested path with no real caller — exactly the "no half-finished implementations" pattern this repo's history has already been burned by once (see the git log around `b839b53`/`6c787c8`).
It is picked back up once the Git backend lands and an action can actually produce a diff to show.

## Consequences

- The refactor preview, the AI chat's per-block Apply, and Replace in Files all show a real before/after diff — hunks, intra-line highlighting, F7 navigation — instead of a truncated snippet or a bare count.
- `lsp-core` now depends on `editor-core` (a new edge; `docs/architecture/layering.md` updated).
  No cycle: `editor-core` is domain-layer and does not depend back on `lsp-core`.
- `index-core/src/lib.rs`, already at its ratcheted file-size ceiling, gained a sibling module (`replace_preview.rs`) rather than growing further — `replace_in_files`, `preview_replacements` and the splice helper they share moved there together, shrinking the ceiling rather than raising it.
- `previewText` (the truncated-first-line renderer) stays: it is still the fallback label for a resource-operation row (create/rename/delete has no text diff to show) and the AI panel's non-diff contexts, not dead code.
- The Git gutter, the Changes dock, and diff-as-a-tab all consume the same `DiffView`/`editor_core::diff` once F3's Git backend exists — no second diff component to build or keep in sync.

## Alternatives rejected

**Diff computation in `vcs-core`.** Rejected already by F3-1's own doc comment, restated here because this ADR is what makes it load-bearing for two more call sites: a rename preview or a Replace-in-Files preview would need a Git repository to show a diff, and a project with no repository would get none.

**A separate preview dialog for Replace in Files.** `RefactorPreviewDialog` already understands "list of files, checkable, confirm" — a second dialog class with the same shape would be one more idiom to learn for no behavioural difference; the `DiffProvider` callback is enough to let each call site supply its own source of hunks.

**`FfiFileDiff` carrying `Vec<FfiHunk>`/`Vec<FfiInlineSpan>` as struct fields.** Not a shape cxx supports on a shared struct crossing the FFI seam; every other list in this bridge already crosses as a method's own return value (`pendingEdits()`, `pendingOps()`), so hunks/spans follow that convention instead of inventing a new one.

**Eager per-file diffing for the refactor preview, to match Replace in Files' eager approach.** A refactoring's `EditPlan` can span many files with large edits; computing every file's diff before the dialog is even shown would cost more than most refactorings ever need displayed.
  Replace in Files already builds every file's new content in memory as part of resolving its spans, so eager costs nothing extra there — the two call sites made different calls because their underlying costs differ, not because of a rule about which is "correct."

**Building `TabKind::Diff` now, alongside `DiffView`.** Discussed in Decision §4: it would have no real caller until `vcs-core` exists, and shipping tested-but-unreachable code is the anti-pattern this repo's `CLAUDE.md` and prior history both warn against.
