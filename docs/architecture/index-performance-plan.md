# Index performance: a faster project index and a visible build

## Progress

Living status table — update the relevant row(s) **in the same commit** that finishes a task, so status and code never drift apart.
A fresh session should read this table (and `git log`) before picking up work, per `CLAUDE.md`.

| Task | Status | Commit |
|---|---|---|
| IP0 — build-time benchmark | done | this change |
| IP1 — cheap wins: line-start table, size cap, binary sniff, walker stat reuse, writer threads | done | this change |
| IP2 — parallel file processing | done | this change |
| IP3 — packed symbol documents (schema change) | done | this change |
| IP4 — sidecar stamps, batched reindex | open | |
| IP5 — progress indicator in the status bar | open | |

### Measurements

Same corpus each time: this workspace's `crates/` directory copied to a temp dir, 281 files, ~1.8 MiB of source.
Taken with `cargo test --release -p index-core --test index_build_bench -- --ignored --nocapture` inside `linux-builder`.

| After | Cold build | Warm open | Index size |
|---|---|---|---|
| baseline (`640b7b4`) | 4.03 s | 5.8 ms | 3702 KiB |
| IP1 + IP2 | 1.46 s | 7.7 ms | 4374 KiB |
| IP3 | 0.61 s | 7.8 ms | 2344 KiB |

## Context

Opening a project starts a full index build on a background thread, and until it finishes the user sees nothing at all — no counter, no bar, not even a hint that work is happening.
`IndexSlot::Building` existed but nothing rendered it, so the only way to discover the state was to run a search and be told to try again later.

The build was also slower than the work it does justifies, for reasons that were measurable rather than suspected:

1. `line_and_col_at` scanned from byte 0 for **every identifier occurrence**, making symbol extraction quadratic in file size.
2. One tantivy document **per identifier occurrence**, each re-allocating the file's path string.
3. The whole walk — read, parse, document build — ran on **one thread**.
4. A flat 50 MB writer budget left tantivy with **three** indexing threads regardless of core count, because it divides the total budget by its 15 MB-per-thread minimum.
5. Warm open deserialised **every stored document** just to recover `(mtime, size)`.
6. No size cap and no binary sniff, so a minified bundle or a multi-megabyte log was ngram(3)-tokenized byte by byte.
7. A redundant `fs::metadata` per file, and a `delete_term` per file even on a from-scratch build.

## Key design decisions

1. **Walk first, then index.**
   The walk is cheap and stays single-threaded; splitting it out means the number of files needing work is known before any file is read, which is the difference between a progress bar and a spinner.
2. **The tantivy writer is shared by reference, not locked.**
   `IndexWriter::add_document` and `delete_term` both take `&self`, so the parallel pass needs no mutex — reading, parsing and document building all happen on rayon workers while tantivy's own threads tokenize.
3. **Oversized files stay findable by name, and by nothing else.**
   Past `MAX_INDEXED_BYTES` a file contributes no documents at all, but still counts as indexable so it keeps its place in the file-name tier.
   The tempting alternative — a stamped document with empty content — was tried and rejected: that document carries the file's path back into the candidate list whenever the ngram stage falls back to "every text doc", so Find in Files would reach into an over-cap file on some patterns and not on others. Consistency is worth re-deciding the file's size on each open, which is a comparison against a stat the walk already did.
6. **One symbol document per (file, name).**
   The five per-occurrence fields are multi-valued and index-aligned; `symbol_docs` appends all five per row and `collect_symbol_matches` zips them back apart. Because a packed document matches `sym_is_definition:1` when *any* row is a definition, every definition query re-filters the expanded rows — that flag is correctness, not an optimisation.
4. **A binary file is dropped entirely**, as before — the sniff just decides it before reading the whole thing rather than after.
5. **Progress is Qt-free.**
   `index-core` takes a `&(dyn Fn(IndexProgress) + Sync)`; the adapter is what knows about Qt threads and throttling. `open_or_build`/`build` keep their old signatures and delegate with a no-op, so no existing caller changed.

## Verification

```sh
make lint
make test
cargo tree -p index-core -e normal | grep -i qt   # must stay empty
cargo test --release -p index-core --test index_build_bench -- --ignored --nocapture
```
