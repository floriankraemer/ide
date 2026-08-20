# Implementation plan: Search Everywhere and the Search Results dock

## Status

Done.
Builds on the shipped project index (ADR-0008 / `index-core`) and find & replace (ADR-0009).
The design decisions are recorded in [ADR 0010](decisions/0010-search-everywhere.md).

## Progress

| Task | Status | Commit |
|---|---|---|
| T1 | done | `pending` |
| T2 | done | `pending` |
| T3 | done | `pending` |
| T4 | done | `pending` |
| T5 | done | `pending` |
| T6 | done | `pending` |
| T7 | done | `pending` |
| T8 | done | `pending` |
| T9 | done | `pending` |
| T10 | done | `pending` |

## Context

Project search existed but was scattered across three surfaces and carried several structural performance ceilings: the index was rebuilt from scratch on every project open, was never updated while the project stayed open, streamed one cross-thread signal *and* one whole-file read per match, and had no debounce, no result limit and no cancellation.
There was no file-name search at all, no action search, and no fuzzy ranking anywhere.

This plan replaces `QuickOpenDialog` with a JetBrains-style Search Everywhere popup over five ranked tiers, promotes the Find in Files dock to a grouped Search Results window, and removes each of those ceilings.
ADR-0010 records why.

## Task breakdown

| Task | Scope | Verification |
|---|---|---|
| T1 | `index-core`: `mtime_secs`/`size_bytes` schema fields, `open_or_build`, delta `sync_from_disk`, `is_index_internal` guarding every mutating entry point | `cargo test -p index-core`: delta open (changed/deleted/added file), fallback build, index-internal paths never re-indexed |
| T2 | `index-core`: in-memory file list + `find_files` fuzzy ranking via `nucleo-matcher`, best-`limit` selection by heap, match positions only for the winners | `cargo test -p index-core`: ranking, cross-segment match positions, limit, file list follows reindex/remove |
| T3 | `index-core`: `search_with` (limit + `AtomicBool` cancellation), `line_text` on `SearchMatch`, `find_definitions_ranked` | `cargo test -p index-core`: limit truncates, cancelled search returns early, `line_text` matches the file, fuzzy symbol order |
| T4 | `app-config`: `keymap::search_actions` (fuzzy, reports the effective shortcut), `Settings::recent_files` + `push_recent_file` | `cargo test -p app-config`: ranking, category matching, user override reported, dedupe/cap |
| T5 | Bridge: `RwLock` index, `QueryGuard` (generation + cancellation), `FfiSearchHit`/`FfiHitKind`/`FfiTierFilter`, `searchEverywhere`, batched `resultsBatch`/`searchBatch`, `openIndex`/`reindexFile`/`removeIndexedFile`/`noteRecentFile`/`refreshKeymap`; `quickOpenTextSearch`/`symbolSearch` removed | Builds; exercised through T7/T8 |
| T6 | Watcher → index: 300 ms coalescing timer over dirty paths, reindex or remove per path; `tabOpened` feeds the Recent tier | Xvfb: edit a file on disk and find the new text without reopening; delete a file and watch its hits vanish |
| T7 | `search_everywhere_dialog.{h,cpp}`: popup, tab bar, 60 ms debounce, generation guard, section headers, match highlighting, Enter opens, Ctrl+Enter hands off to the dock, arrow keys drive the list from the query box | Xvfb: every tier, fuzzy path query, handoff |
| T8 | `search_results_panel.{h,cpp}`: Find in Files extracted from `main_window.cpp` and promoted to a grouped file → matches tree with highlighted spans and batched inserts; replace UI unchanged | Xvfb: grouped results, highlighted spans, replace preview intact |
| T9 | Keymap: `view.searchEverywhere` (`Ctrl+Shift+E`), `view.goToFile` (`Ctrl+Shift+N`), `view.findAction` (`Ctrl+Shift+A`); `view.goToSymbol` retargeted; double-Shift gesture on the main window; `.ide-index/` hidden from the sidebar tree | `cargo test -p app-config`; Xvfb: View menu shows all four with their shortcuts |
| T10 | `build.rs` entries, `layering.md` dependency rows, ADR-0010, this doc, `.gitignore` | Reviewed |

## Measurements

Taken on this repository (345 indexed files) in the `ide-linux-builder` container, release profile:

| What | Before | After |
|---|---|---|
| Project open | 987 ms (full rebuild, every launch) | **25 ms** warm (delta scan); 987 ms only on a first run |
| File-name query | *(feature did not exist)* | **60 µs** |
| Symbol query | substring scan, unranked | **4.4 ms**, fuzzy-ranked |
| Text query (limit 30) | unbounded, uncancellable | **2.1 ms** |
| Per-match cost | one cross-thread hop + one whole-file read | one hop per 256-match batch, no re-read |

Extrapolating the file tier's per-item cost (~0.17 µs) puts a 100k-file project at roughly 17 ms per keystroke — comfortably inside the 60 ms debounce window, and the tier that would need attention first if that ever changes.

## Verification performed

- `cargo test --workspace` green in the `ide-linux-builder` container (index-core 29, app-config 28, project-model 24); `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean.
- Qt-leakage gates (`cargo tree -p editor-core|project-model|app-core|index-core|app-config -e normal | grep -i qt`) still empty.
- Driven end to end under Xvfb against a fixture project, confirmed by screenshot: double-Shift and the View menu both open the popup; `utlhlp` ranks `src/util/helpers.rs` first with every matched character highlighted across path segments; Enter opens it and Class View follows; `widget` returns ranked Symbols (with kind and container) then highlighted Text hits with project-relative paths; Ctrl+Enter moves the full result set into the Search Results dock grouped by file; an empty query shows Recent Files then project files; appending a function to a file on disk makes it searchable within seconds without reopening the project; deleting a file drops its hits.
- One real bug found and fixed during that pass: watcher events on the index's own directory re-indexed the index in an unbounded loop, starving every reader — see ADR-0010 and `TextIndex::is_index_internal`.

## Known ceilings

Marked `ponytail:` at their call sites:

- Symbol rows carry no highlight positions (`find_definitions_ranked` scores without reporting match indices).
- Find-in-Files results cap at 10,000 matches with no "showing first N of M" affordance.
- Change detection is `(mtime, size)`: an edit made while the IDE was closed that preserves byte length within the same second is missed on open. Live edits go through the watcher.
- Regex and case-insensitive text searches still skip ngram narrowing and scan every indexed file (inherited from ADR-0008).
