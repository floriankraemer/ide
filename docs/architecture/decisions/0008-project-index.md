# 0008. Project index: hybrid tantivy(ngram) + ripgrep-crates for text, name-based symbol/reference schema

## Status

Proposed.
Pending a `devops-expert` verification pass of tantivy's `zstd-sys` C dependency building clean
under the existing MXE Windows cross-build stage, per the plan doc's "Gate before starting
F1/G1 implementation".
Do not mark this Accepted until that check comes back clean.
The Linux `linux-builder` build is verified clean as of this task (G1) — `zstd-sys` compiled
without needing any Dockerfile change beyond what was already present in the image.

## Context

The language-folding/Class-View/terminal/search plan (task G, tracks G1→H and E1→J) calls for
project-wide search covering both "Find in Files" (text) and symbols/references (go-to-symbol,
find-usages).
A naive full-text index (index every substring or every word) is either too slow to build on a
large repo or too imprecise to give exact match spans; a naive on-demand `grep`-style scan over
the whole repo on every keystroke doesn't scale past a small project.
Task G1 builds the text half of this; the symbol/reference half is task E1, deliberately not
built yet.

## Decision

New Qt-free crate `index-core` (`crates/index-core`, mirrors `editor-core`/`project-model`/
`syntax-core`/`pty-core`), two-stage hybrid search rather than a single full-text engine:

- **Index build** (`TextIndex::build`): walk the project root via the `ignore` crate (the same
  crate ripgrep itself uses), which is gitignore-aware and skips hidden files by default —
  deliberately not reusing `project-model::DirectoryTree::build()`'s raw walk, which has no
  ignore-file awareness and would waste an index on `node_modules`/`target`/etc. Each readable
  UTF-8 text file becomes one tantivy document: a `path` field (`STRING | STORED`, exact-match
  term for delete/update) and a `content` field indexed with a custom **ngram(3) tokenizer** —
  not tantivy's default word tokenizer, which would miss mid-word substring matches. The index
  lives at `<project_root>/.ide-index/`.
- **Search** (`TextIndex::search(pattern, is_regex)`): the tantivy ngram index narrows the whole
  project down to a small set of candidate files (or, if the pattern is too short to produce
  ngram terms, or the query fails to parse — regex metacharacters aren't literal ngram terms —
  every indexed file, correctness-preserving but not narrowed). Each candidate is then
  re-scanned with `grep-searcher`/`grep-regex`/`grep-matcher` (ripgrep's own library crates) to
  produce exact line numbers and byte-offset match spans. This two-stage split is what gives
  ripgrep-grade correctness (real regex semantics, exact spans) with index-backed speed on large
  repos, instead of either a slow full-repo regex scan every keystroke or an imprecise
  pure-tantivy-scored result.
- **Incremental updates**: `reindex_file`/`remove_file` delete-then-reinsert (or just delete) a
  single document by its `path` term and commit — no full rebuild. The eventual `ui-shell`
  integration (task H) calls these from `project_model::ProjectWatcher`'s callback, same
  structural-vs-content-only distinction (`is_structural_change`) the project tree sidebar
  already uses.
- **Symbol/reference schema is out of scope for this task.** `index-core`'s module doc marks
  where it lands (Task E1: name/kind/file/line/container/`is_definition` fields fed by
  `syntax-core::outline()` and `identifier_occurrences()`) so E1 can add a second tantivy
  schema/segment without reworking the text half built here.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| Pure full-text tantivy index (word tokenizer, tantivy-scored results only) | Wrong tool for exact substring/regex search — word tokenization misses mid-identifier substrings (e.g. searching `Handler` inside `RequestHandlerFactory`), and tantivy's own match scoring doesn't give exact byte-offset spans a "Find in Files" UI needs to highlight. |
| Pure on-demand `grep`-style scan (no index at all) | Correct and simple, but doesn't scale — a full-repo regex scan on every keystroke is the exact "fastest IDE on earth" ceiling this plan is trying to avoid on a large repo. |
| Reusing `project-model::DirectoryTree`'s walk for indexing | Not confirmed gitignore-aware, and indexing `node_modules`/`target`/build output would be both slow to build and full of noise in search results. The `ignore` crate is ripgrep's own dependency for exactly this walk. |
| Cross-file type/binding resolution for symbols (deferred to E1, noted here since it shapes the schema) | A real language-server-grade binding resolver is out of scope for this plan — the symbol schema is deliberately name-based (same-name matches across files), a documented scope boundary short of a language server, not an oversight. |

## Consequences

- Positive: text search stays correct (ripgrep-grade regex/span semantics) while getting
  index-backed candidate narrowing instead of a full-repo scan per query.
- Positive: incremental re-index is a single-document delete+insert, not a full rebuild, so a
  save doesn't cost the whole project's indexing time.
- Negative / accepted risk: tantivy pulls in `zstd-sys`, a C dependency, whose Windows/MXE
  cross-build behavior is not yet verified — this is the open item this ADR stays Proposed on.
  Linux build/tests are clean under Docker `linux-builder` as of this task.
- Negative / accepted scope boundary: regex search candidate narrowing falls back to "every
  indexed file" (no speed gain, correctness unaffected) whenever the tantivy query parser can't
  parse the raw regex pattern as a query string — extracting a literal prefix/substring from the
  regex to narrow with instead is a documented future upgrade, not built here (see the
  `ponytail:` comment in `crates/index-core/src/lib.rs`).
- `.ide-index/` is written inside the project root; it is not gitignored by this crate itself —
  whichever task wires this into `ui-shell` (task H) should make sure a project's own
  `.gitignore` excludes it, or `index-core` will index its own index directory's stale copies on
  a later rebuild of a *different* project that happens to nest inside this one. Not a concern
  for a single-root project indexing itself, since `build()` always removes and rebuilds
  `.ide-index/` before walking.

## Related

- [ADR-0003: FFI seam conventions](0003-ffi-conventions.md) — typed-error convention `index-core`
  follows internally now, ahead of crossing the FFI seam in task H.
- [ADR-0007: Embedded terminal](0007-embedded-terminal.md) — same "Qt-free crate, `std::thread` +
  `CxxQtThread::queue()` for background work" shape this crate's eventual `ui-shell` integration
  will reuse for background indexing.
- `crates/index-core/src/lib.rs` — the crate this ADR documents.
- `docs/architecture/language-folding-classview-terminal-search-plan.md` — decision 4 and the
  G1/E1/H/I/J task breakdown this ADR covers.
