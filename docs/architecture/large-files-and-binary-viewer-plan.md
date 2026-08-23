# Large files and the binary viewer — plan

Two problems that share a cause: the editor assumes every file it is handed is a reasonably sized text file.

**Large files.**
Opening a 4.8 MB generated HTML report (a Plotly page with the minified `plotly.js` bundle inlined — 3887 lines, the longest 609,368 characters) froze the editor for roughly fifteen seconds and left scrolling and typing sluggish afterwards.
Three separate causes, none of them "the file has many lines":

- `QPlainTextEdit` wraps by default, and `QPlainTextDocumentLayout` lays a block out atomically, so a 609k-character line became thousands of visual lines inside one text block and every layout touch of that block cost the whole line.
- Highlighting is whole-document and had no ceiling, so opening the file meant a full tree-sitter parse of 4.8 MB of HTML plus a full non-incremental JavaScript parse of every injected `<script>`, synchronously, before the first paint.
- The line-number gutter asked "does a fold start on this line?" with a linear scan of every fold range in the document, once per painted line, on every scroll step.

**Binary files.**
`AppSession::open_file` sniffed the first 8 KB and refused anything binary with `AppError::BinaryFile`, so clicking a `.png` or an executable in the project tree produced a dialog and nothing else.
A read-only hex view turns that dead end into something useful, and is the first tab in the editor area that is not a text document.

Authority for the layering decisions below: `docs/architecture/layering.md` and [ADR-0020](decisions/0020-tab-kinds-and-the-binary-viewer.md).

## Measurements

Headless runs in `linux-builder` under Xvfb, driving the real app with `xdotool`: click the file in the project tree, then traverse it.
Same machine, same file, debug build.

| Action | Before | After |
| --- | --- | --- |
| Open the 4.8 MB file | 15.2 s | 2.6 s |
| Ctrl+End (whole-document traversal) | 11.9 s | 1.9 s |
| Ctrl+Home | 2.8 s | 1.5 s |

Page-down timings are deliberately not compared: `PageDown` steps by *visual* lines, so the wrapped build covered far less of the document per press and the two numbers measure different amounts of work.

## Progress

| # | Task | Status | Commit |
| --- | --- | --- | --- |
| A1 | Word wrap off by default in `CodeEditor` | done | |
| A2 | Highlighting size ceilings (`MAX_HIGHLIGHT_BYTES`, `MAX_HIGHLIGHT_LINE_BYTES`) in `syntax-core` | done | |
| A3 | O(1) fold-start lookup in the gutter paint | done | |
| B1 | Tab kinds in `app-core`; `open_file` opens binary files | open | |
| B2 | `BinaryFile` + hex row formatting in `editor-core` | open | |
| B3 | `HexViewer` widget and its FFI seam | open | |
| B4 | Wire the viewer into the tab area; audit the `CodeEditor` casts | open | |
| B5 | Stop `looks_binary_file` reporting unreadable files as binary | open | |

## Part A — large files

### A1. Word wrap off by default

`CodeEditor` now sets `QPlainTextEdit::NoWrap`.
This is the default VS Code and IntelliJ ship, and it is what keeps a machine-generated file usable at all: with wrapping on, one 609k-character line becomes thousands of visual lines in a single block, and `QPlainTextDocumentLayout` has to lay that block out in full every time it is touched.
A horizontal scrollbar appears instead.

No word-wrap toggle in the View menu — add one if the missing wrap is actually felt.

### A2. Highlighting has a size ceiling

Both ceilings live in `syntax-core`, next to `MAX_INJECTION_DEPTH`, because "this file is too big to highlight" is a rule and rules do not live in the view (ADR-0002).
They are enforced inside `Highlighter::reparse`, the single path both `set_text` and `edit` take, which means `syntax_highlighter.cpp` needed no change at all: it keeps calling the same functions and simply receives no spans.

- `MAX_HIGHLIGHT_BYTES` (2 MiB): the document is kept but not parsed, and the tree is dropped so `fold_ranges` stops costing anything too.
- `MAX_HIGHLIGHT_LINE_BYTES` (20,000, matching VS Code's `editor.maxTokenizationLineLength`): spans that *begin* inside a longer line are dropped. By start rather than by overlap, because a span opening on an ordinary line and running into a long one is a single character format and costs nothing; what this removes is the thousands of formats a minified line generates within itself.

Ordinary files are unaffected, and pay nothing for the check: no line can exceed the line cap if the whole document does not, so the common case never scans.

### A3. O(1) fold lookup

`CodeEditor` keeps a `QHash<int, FoldRange>` keyed by start block, rebuilt in `setFoldRanges` whenever the tree updates.
The gutter repaints on every scroll step and asks once per painted line, so the previous linear scan made scrolling cost O(visible lines × fold ranges).
Only the first range starting on a given line is kept, which is what the previous scan returned for nested constructs opening on the same line.

## Part B — the binary viewer

See [ADR-0020](decisions/0020-tab-kinds-and-the-binary-viewer.md) for why a tab gained a kind, why the hex formatting lives in `editor-core`, and why the viewer is read-only.

## Verification

```sh
make lint
make test
cargo tree -p editor-core -e normal | grep -i qt   # must stay empty
cargo tree -p app-core    -e normal | grep -i qt   # must stay empty
cargo tree -p syntax-core -e normal | grep -i qt   # must stay empty
```

End to end, against the real app: open a multi-megabyte generated file and traverse it; confirm an ordinary source file still highlights, folds and populates the Class View; open a binary file and confirm the hex view scrolls to the end of a large file and that Save, Find and Find Usages are inert on it.
