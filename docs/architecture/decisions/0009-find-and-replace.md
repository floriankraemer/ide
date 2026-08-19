# 0009. Find & Replace: matching in `editor-core`, project-wide replace through `index-core`

## Status

Accepted.
Implemented as tasks F1–F8 of [the Find & Replace plan](../find-replace-plan.md).
Verified under Xvfb end to end: in-editor find/replace including a regex capture replace undone by a single `Ctrl+Z`, and a case-insensitive project-wide replace that rewrote one file on disk and raised the existing external-change prompt in the open tab.

## Context

The IDE had project-wide *search* (ADR-0008's `index-core`, wired to the Find in Files dock) but no in-editor find at all and no replace anywhere.
`Ctrl+F` and `Ctrl+R` were unbound.
Two decisions had to be made: where the matching logic lives, and how a project-wide replace reaches files that are open in the editor.

Qt offers `QPlainTextEdit::find` and `QRegularExpression`, which would have made an in-editor find a pure C++ feature with no Rust involvement at all.

## Decision

**Matching lives in Rust, in `editor-core::search`.**
A new module exposes `find_matches` and `replacements` over `regex`, taking a `SearchOptions { regex, case_sensitive }`.
`CLAUDE.md`'s humble-view rule and ADR-0002 put business rules in the Qt-free crates; "what counts as a match" and "what text replaces it" are rules, not rendering.
Using `QRegularExpression` in `find_bar.cpp` would have been the first matching decision encoded in C++, and would have given the editor and Find in Files two different regex dialects (Qt's PCRE-ish one vs the `regex` crate's, which `grep-regex` already implements for project search).

Consequences of that choice:

- `editor-core` gains a direct `regex` dependency. It was already in `Cargo.lock` transitively via `grep-regex`/`ignore`/`tantivy`, so this costs no extra compilation. `editor-core` stays Qt-free.
- Match offsets cross the FFI seam as **UTF-16 code units**, not the UTF-8 byte offsets `FfiHighlightSpan` uses. The only consumer positions a `QTextCursor`, which indexes in UTF-16, so converting once inside `editor-core` beats making the view carry an offset table.
- The invokables take the widget's **current text** as a parameter rather than reading `Document`'s rope. Live keystrokes never reach the rope (ADR-0003: the widget owns the live buffer, the session owns dirty state), so searching the rope would search pre-edit text. This is the same shape `saveTab(id, content)` already has.
- An uncompilable pattern cannot be reported in the return value, because the invokables return `Vec<T>` and ADR-0003 bans sentinel values. It travels as its own `findPatternInvalid` signal, mirroring `SearchModel::searchFailed`.

**Project-wide replace writes to disk and leans on the existing external-change flow.**
`TextIndex::replace_in_files` applies the spans it is given, re-indexes the touched files, and skips (never half-writes) any file that changed since the search.
Open tabs are not patched in memory: the write lands on disk, the filesystem watcher fires, and the existing `checkExternalChange` → `externalChangeDetected` prompt asks the affected tab to reload.
Adding a second, buffer-level path for the same edit would mean two mechanisms that can disagree about which copy is authoritative.

Because a project-wide rewrite has no undo, the Find in Files panel lists every match with a checkbox (checked by default) and `Replace All` acts only on the checked ones, behind a confirmation naming the match and file counts.
The in-editor Replace All needs no such gate: it goes through `QTextCursor` in one edit block, so `Ctrl+Z` reverses it.

**Case sensitivity is a search option everywhere**, added to `TextIndex::search` alongside the existing `is_regex`.
The `content` field's ngram tokenizer is case-sensitive, so a case-insensitive query skips the ngram narrowing entirely and scans every indexed file — narrowing is an optimisation, not a correctness requirement (ADR-0008), and a lowercased companion field is the upgrade path if this ever gets slow.

`TextIndex::search` also now reports **every** occurrence on a line instead of only the first.
That was harmless for a search-only feature and wrong for replace: a second match on a line would have been silently left behind.

## Consequences

- One regex dialect (the `regex` crate's) across in-editor find, Find in Files, and both replaces.
- `find_bar.{h,cpp}` is a new moc'd C++ class and the crate's third hand-written QObject; it paints, scrolls, splices and counts, and asks Rust for everything else.
- The find bar floats over the `CodeEditor` rather than sitting in a container above it, so the editor remains the tab page widget and the `tabId` dynamic property plus every `qobject_cast<QPlainTextEdit *>` lookup keep working unchanged.
- Project-wide replace is not atomic across files: a crash mid-run leaves earlier files rewritten. Per-file writes are whole-file, so no file is left half-written.
