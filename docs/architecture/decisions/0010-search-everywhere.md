# 0010. Search Everywhere: one popup over ranked tiers, a persistent index, and batched results

## Status

Accepted.
Implemented as tasks T1–T10 of [the Search Everywhere plan](../search-everywhere-plan.md).
Verified under Xvfb end to end and measured on this repository: a warm project open costs 25 ms against 987 ms for a full rebuild, and a file-name query answers in ~60 µs.

## Context

The IDE already had project search: `index-core` (ADR-0008), a Find in Files dock, a Find Usages dock, and a `QuickOpenDialog` that merged symbol hits with a capped "Text matches" section (ADR-0009 added replace on top).

What it did not have was a JetBrains-style *Search Everywhere*: no file/path search at all, no action search, no fuzzy ranking (`find_definitions` was `str::contains`, ordered by `(path, line)`), and no recent-files tier.

It also carried several structural performance problems, all visible in the code rather than inferred:

- `TextIndex::build` deleted and rebuilt `.ide-index/` on **every** project open.
- `reindex_file`/`remove_file` existed with **zero callers** — the watcher was never wired to them, so results went stale until the project was reopened.
- `SearchModel` emitted **one `CxxQtThread::queue()` hop per match**, and re-read the whole file **once per match** to recover the line for the snippet.
- Quick Open had no debounce and no request id (a documented `ponytail:` comment), so fast typing interleaved stale results.
- `index-core::search` had no limit and no cancellation: an abandoned keystroke's scan ran the whole project to completion.

## Decision

**One popup, five tiers, ranked in Rust.**
`SearchModel::searchEverywhere(query, tiers, generation, limit)` runs Recent → Files → Symbols → Actions → Text and streams `FfiSearchHit` rows back. Every tier produces the same row shape, so the view renders one list rather than four. Which tiers run is decided by the popup's active tab and passed in as `FfiTierFilter`: the Files tab never greps the project and the Text tab never scans symbols — the work is *skipped*, not filtered out afterwards.

**Fuzzy ranking uses `nucleo-matcher`.**
fzf-grade scoring with SIMD, pure Rust, no `-sys` crate — which matters because the Windows artifact cross-builds under MXE, where every native dependency is a risk (ADR-0005 and ADR-0008 both hit that). Hand-rolling a subsequence scorer would have been ~100 lines of worse ranking, and ranking quality *is* the feature for a file tier.

**The file tier is an in-memory `Vec<PathBuf>`, not a tantivy query.**
100k paths is a few megabytes; nucleo over a flat slice is microseconds, while a tantivy round trip per keystroke buys nothing. Measured at 60 µs over this repository's 345 files. Only the best `limit` hits get their match positions resolved, so a large project pays one cheap scoring pass rather than an allocation per candidate.

**Results cross the FFI seam in batches, tagged with a generation.**
`resultsBatch(generation, Vec<FfiSearchHit>)` replaces the per-match signal, and `SearchMatch` now carries the `line_text` the grep pass already had — together these remove one cross-thread hop *and* one whole-file read per match. `generation` is the view's monotonic query id: a newer query cancels the running one through an `AtomicBool`, and the view drops any batch that is not the generation it is waiting for. Search Everywhere and Find in Files keep **separate** generation counters so typing in the popup never cancels the results panel's search.

**The index persists across launches.**
`TextIndex::open_or_build` reuses `.ide-index/` and re-reads only files whose `(mtime, size)` stamp changed, falling back to a full build on a first run, a schema change, or a corrupt directory. The project watcher now drives `reindex_file`/`remove_file` through a 300 ms coalescing timer, so results stay live while the project is open.

**The index filters its own writes.**
`.ide-index/` lives *inside* the project it indexes, so the watcher sees every commit the index makes. Acting on those events re-indexes the index, which writes more index files, which produces more events — an unbounded loop that starved every reader behind the index lock. `TextIndex::is_index_internal` gates every mutating entry point, so no caller can reintroduce it. The sidebar tree hides the same directory.

**The index moves from `Mutex` to `RwLock`.**
Every query path takes `&self`; only re-indexing and replace need `&mut`. Several searches now run concurrently and only writes serialise them.

**The Find in Files dock becomes the Search Results dock**, grouped file → matches with highlighted spans, rather than a second parallel results window. Its checkable rows, replace row and confirmation dialog are untouched — that behaviour is shipped and ADR-0009 covers it.

## Alternatives rejected

- **A new `search-core` crate.** It would have wrapped exactly one existing crate. `index-core` already owns the walk, the schema and the symbol data.
- **Filtering tiers in the view.** Simpler to write, but it means doing the work and throwing it away — the opposite of the stated goal.
- **A second results dock.** Two places for results, and a duplicate of the replace UI.
- **Global cross-tier score normalisation.** JetBrains groups by tier, and a normalisation nobody can tune is worse than a fixed, predictable order.
- **A content hash instead of `(mtime, size)`.** Hashing every file on open is the cost the persistent index exists to avoid. The residual gap — an edit made while the IDE was closed that preserves byte length within the same second — is documented at the call site; live edits are covered by the watcher.

## Consequences

- `index-core` and `app-config` gain a `nucleo-matcher` dependency. Both stay Qt-free.
- `app-config` owns action matching (`keymap::search_actions`) because it owns the action catalogue, and gains a `recent_files` list.
- `FfiHitKind` and `FfiTierFilter` are append-only enums crossing the seam, the same discipline `AppError`'s numeric codes follow.
- Symbol rows carry no highlight positions yet: `find_definitions_ranked` scores without reporting match indices. Marked `ponytail:` at the call site.
- Find-in-Files results are capped at 10,000 matches with no "showing first N" affordance. Marked `ponytail:`.
- `quickOpenTextSearch` and `symbolSearch` and their signal trios are **removed** — Search Everywhere subsumes both. `projectSymbols` and `findUsages` are untouched; Class View and Find Usages still use them.
