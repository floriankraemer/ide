# Implementation plan: Find & Replace (in-editor and project-wide)

## Status

Done.
Builds on the shipped Find in Files dock (ADR-0008 / `index-core`) and stays inside ADR-0001 and ADR-0002.
The design decisions are recorded in [ADR 0009](decisions/0009-find-and-replace.md).

## Progress

| Task | Status | Commit |
|---|---|---|
| F1 | done | `bd765ee` |
| F2 | done | `bd765ee` |
| F3 | done | `bd765ee` |
| F4 | done | `bd765ee` |
| F5 | done | `bd765ee` |
| F6 | done | `bd765ee` |
| F7 | done | `bd765ee` |
| F8 | done | `bd765ee` |

## Context

The editor had no in-file search and no replace of any kind; project search existed but was always case-sensitive and reported only the first match per line.
This plan adds `Ctrl+F` / `Ctrl+R` over the active editor with regex and case options, and extends Find in Files with the same options plus a previewed, opt-in project-wide replace.

Two constraints shaped it, both recorded in ADR-0009: matching is a business rule, so it lives in Qt-free Rust; and `Document`'s rope is stale between saves, so in-editor search runs over the text the widget hands in.

## Task breakdown

| Task | Scope | Verification |
|---|---|---|
| F1 | `editor-core::search`: `find_matches`/`replacements` over `regex`, UTF-16 offsets, `$1` capture expansion, zero-length-match guard; `regex` dependency | `cargo test -p editor-core`: literal vs regex, both case flags, capture expansion, UTF-16 offsets across accented and non-BMP text, invalid pattern |
| F2 | Bridge: `FfiTextMatch`/`FfiReplacement`, `DocumentManager::findMatches`/`replacementsFor`, `findPatternInvalid` signal | Builds; exercised through F3 |
| F3 | `find_bar.{h,cpp}`: floating find/replace bar per `CodeEditor`, live highlighting via `CodeEditor::setMatchSelections`, next/previous, Replace / Replace All in one edit block | Xvfb: match count, navigation, single-undo Replace All |
| F4 | Keymap: `edit.find` (`Ctrl+F`), `edit.replace` (`Ctrl+R`), `edit.findNext` (`F3`), `edit.findPrevious` (`Shift+F3`) + Edit-menu wiring | `cargo test -p app-config`; Xvfb: shortcuts open the bar |
| F5 | `index-core`: `case_sensitive` on `search`, no ngram narrowing when case-insensitive, every occurrence per line | `cargo test -p index-core` |
| F6 | `index-core::replace_in_files` + `ReplaceReport`, re-index of touched files, skip-not-corrupt on changed files | `cargo test -p index-core` |
| F7 | Find in Files panel: Match case checkbox, checkable results, replace row, confirmation dialog; `SearchModel::replaceInFiles` + `replaceFinished`/`replaceFailed` | Xvfb: case-insensitive hit, previewed replace writes to disk, open tab prompts to reload |
| F8 | ADR-0009, `layering.md` dependency row, this plan doc | Reviewed |

## Verification performed

- `cargo test --workspace` green in the `ide-linux-builder` container; `cargo fmt --all --check` and `cargo clippy --workspace --all-targets` clean.
- Qt-leakage gates (`cargo tree -p editor-core|project-model|app-core -e normal | grep -i qt`) still empty.
- Driven end to end under Xvfb with screenshots: `Ctrl+F` counts `1/3` and highlights every match; next/previous navigate; `Ctrl+R` with `(\w+)_old` → `$1_new` rewrites both matches and a single `Ctrl+Z` reverses the whole Replace All; a lowercase `widget` query finds `Widget factory` only with Match case off; a previewed project replace rewrote the file on disk, reported `Replaced 1 match(es) in 1 file(s).`, and the open tab raised the existing "modified outside the editor" prompt.
