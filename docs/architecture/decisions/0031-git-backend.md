# 0031. Git backend: `gix` for reads, the `git` binary for anything touching the user's world

## Status

Accepted

## Context

`docs/architecture/next-five-features-plan.md`'s F3 ("Git v1") lane needs a Git backend for `vcs-core`: repository discovery, status, HEAD reads, working-tree hunks, staging, commit, branches, remotes, history and blame.
Three backends exist in the Rust ecosystem: `gix` (pure Rust), `git2`/`libgit2` (a C library with Rust bindings), and shelling out to the user's own `git` binary.
None of them is a complete answer alone, and the plan's own ADR-0027 draft laid out why before this crate was written; this ADR records the decision as built, against the actual `gix 0.87.1`, not the `gix 0.87` the plan assumed.

Two things make this decision structural rather than a library choice.
The MXE Windows cross-build has already refused a C dependency once, for OpenSSL, on exactly this ground (ADR-0021).
`git2`/`libgit2` would reopen that refusal for a feature the plan ranks below editor ergonomics.
The gutter (`vcs_core::hunks::HunkCache`, F3-4) runs on every keystroke once a Git-aware editor exists.
A subprocess per keystroke is not a performance ceiling to tune later; it is a different architecture.

## Decision

### 1. Pure reads of object/index state go through `gix`, in-process

Repository discovery (`gix::discover`), HEAD resolution and reads (`Repository::head`, `Repository::head_blob`), status (`Repository::status`, via `gix::Repository::status(progress)`), local branch listing (`Repository::branches`, via `gix::Repository::references().local_branches()`), and commit history (`Repository::log`, `Repository::file_history`, via `Id::ancestors().all()`) never spawn a process.

Checked directly against `gix 0.87.1` rather than assumed from the plan's `0.87`-era description.
`Repository::status(progress)` is real, documented and non-experimental (`gix-0.87.1/src/status/mod.rs:99`), covering both index-vs-worktree and HEAD-vs-index in one call.
There is still no path-filtered revwalk anywhere in `gix-0.87.1/src/revision/` — `rev_walk().selected(pred)` takes a commit-id predicate, not a pathspec.
`Repository::file_history` is therefore the ~40-line walk-and-compare the plan anticipated: for each ancestor commit, look up the path's tree entry against its first parent's and record the commit when the object id differs.
No C dependency appears in the resolved dependency graph: `gix-zlib` routes compression through `zlib-rs`, pure Rust.
This was confirmed by inspection of the actual `cargo build -p vcs-core` output, not by reading `Cargo.toml` declarations — the MXE cross-build claim in the plan holds for the version actually pinned.

`vcs-core`'s own types (`HeadInfo`, `RepoStatus`, `FileStatus`, `ChangeKind`, `LogEntry`) wrap `gix`'s rather than re-exporting them.
This is the same reason `settings-model` wraps `syntax-core`'s and `lsp-core`'s vocabularies rather than leaking them past its own boundary: a future `gix` major-version bump, or a swap to a different read backend, stays inside this crate.

`gix` is taken with `default-features = false` and an explicit feature list (`max-performance-safe`, `sha1`, `status`, `revision`, `blob-diff`, `index`, `dirwalk`, `excludes`, `attributes`) — the crate's defaults pull in write paths (checkout, merge) this layer must never reach for.

### 2. Anything touching the user's configuration, credentials, hooks or signing shells out to `git`

Staging (`git add`, `git apply --cached`), commit, branch create/checkout/delete, and fetch/pull/push all go through `vcs_core::cli::run`, a thin wrapper around `std::process::Command`.
It always sets `GIT_TERMINAL_PROMPT=0`, so a missing credential fails fast on stderr instead of blocking on a prompt nothing can answer.
It applies a 60-second timeout, generous since fetch/push are network calls, not local reads.
It turns a nonzero exit into `VcsError::GitFailed { command, stderr }`, carrying `git`'s own message verbatim rather than a bare exit code.
It reports a missing `git` binary as a distinct `VcsError::GitNotInstalled`, never folded into some other failure.

This is the same reasoning the plan's ADR-0027 draft gave: re-implementing credential helpers, SSH agents, `insteadOf` rewriting, hooks and GPG signing is five different ways to be subtly wrong in a way that looks like this IDE's bug, and the user's already-configured `git` already gets all five right.

Per-hunk staging builds a real unified-diff patch (`staging::hunk_patch`) and feeds it to `git apply --cached`/`--reverse --cached`, tested against a real `git` binary in a scratch repository.
Testing found that a zero-context patch reliably fails with "patch does not apply" even when line numbers are exact, so the patch carries three lines of context on each side, matching `diff -u`'s own default — a deviation from the task doc's original framing ("no context lines"), documented in `staging.rs` itself.

### 3. Hunks are computed in-process, against a `gix`-read blob, never by a subprocess

`vcs_core::hunks::HunkCache` reads a file's `HEAD` blob via `gix` (§1) and diffs it against the caller-supplied working text with `editor_core::diff::diff_lines` — the same Git-free diff engine ADR-0028/0030 already established, not a second implementation.
The cache is keyed by `(path, head_oid, revision)`, where `revision` is a monotonic counter the caller supplies, since this crate has no idea about the live buffer on the other side of the FFI seam.

### 4. Blame shells out, deliberately, even though it is a read

`Repository::blame` runs `git blame --porcelain` rather than using `gix`'s own blame implementation.
This is the one place in this crate where a read goes through `git` rather than `gix`, for a different reason than write operations do: `gix`'s blame is young, blame is not on the hot path (nothing calls it per keystroke, unlike hunks), and `git`'s own rename-following is more mature than a reimplementation would be.
`blame::parse_porcelain` is a real parser against the documented porcelain format, tested against literal fixture output covering the format's real subtlety: a commit's full metadata block is only printed the first time that commit is seen in one invocation, and every later line from the same commit carries just the header and the tab-prefixed content line.

### 5. History and blame are cached, but with a simpler key than the gutter's

`HistoryCache` and `BlameCache` key on `(path, head_oid)` (or `(head_oid, max)` for the whole-repository log) rather than `HunkCache`'s `(path, head_oid, revision)`.
A history or blame view is opened on demand by the user, not recomputed on every keystroke, so there is no live-buffer revision to invalidate against — copying the gutter's cache key here would imply a staleness problem this data does not have.

### 6. A hunk revert is an edit, not a write

`Repository::revert_hunk_edit` returns a `vcs_core::TextEdit` (a half-open line range plus replacement text) rather than writing a reverted file to disk.
This mirrors the shape `lsp_core::workspace_edit::TextEdit` already established for "one Ctrl+Z undoes it" (ADR-0019), in line units rather than UTF-16 characters, since a hunk never touches part of a line.
The future bridge task therefore has an edit-shaped value ready to splice into the open buffer inside one `beginEditBlock`, the same seam every other edit source in this IDE already uses.

## Consequences

- `vcs-core` has no Qt/cxx-qt dependency, direct or transitive (`docs/architecture/layering.md`'s new row), and depends only on `editor-core`, `gix`, `serde`, and std.
- Every `vcs-core` operation that shells out is tested against a real `git` binary in a `tempfile` scratch repository (staging round-trips, commit with a real rejecting hook, branch force-delete, fetch/pull/push over a real filesystem-transport clone) rather than against a mocked `Command`, matching this repo's stated preference for testing as close to real behaviour as the layer allows.
- `VcsError` carries stable numeric codes in the 700-799 range `next-five-features-plan.md` §5 reserves for `vcs-core`, laid out now even though no FFI seam crosses yet, so the future bridge (F3-12) does not have to translate a wrong shape.
- Two speculative error variants from the task breakdown were not built, and are recorded here rather than left as silent scope-narrowing.
  There is no distinct "hook rejected the commit" error: a pre-commit or commit-msg hook's own stderr is inherited by `git commit` and already lands verbatim in `VcsError::GitFailed`'s `stderr`, and there is no reliable, non-heuristic signal in `git`'s exit code that distinguishes "a hook said no" from any other commit failure — inventing one would mean guessing from stderr text, exactly the fragile parsing this crate exists to avoid doing to a future UI layer.
  There is no distinct "no upstream configured" error either: `vcs_core::Repository::push` always names an explicit remote and branch, and `git push` only refuses for a missing upstream on a bare `git push` with neither named, a shape this crate's own argv construction cannot produce — a `NoUpstream` variant would have been permanently unreachable dead code.

## Alternatives rejected

**Pure `gix`/`git2` for everything, including writes.**
Credential helpers, SSH agents, `insteadOf` rewriting, hooks and GPG signing are five separate re-implementations, each likely to fail in a way that looks like this IDE's bug rather than a known Git limitation, and some (a leaked credential helper invocation) fail by leaking rather than just erroring.

**Pure `git` CLI, including for hunks.**
A subprocess per keystroke for gutter diffs is not a ceiling to raise later; the gutter is the reason this split exists at all.

**`git2`/`libgit2` over `gix`.**
A C dependency in an MXE Windows cross-build ADR-0021 already refused on exactly this ground for OpenSSL.
`gix` and `editor_core::diff`'s `imara-diff` are both pure Rust, which is half of why `gix` was chosen.

**Bundling a `git` binary.**
Then this project owns `git`'s own CVEs and per-platform builds, and overrides the user's own configured `git` (their credential helpers, their `insteadOf` rewrites, their hooks) with one they did not choose.

**`gix`'s own blame implementation, once it matures.**
Deferred, not rejected outright: revisit if `gix`'s blame becomes as capable as `git blame`'s rename-following, since it would remove the one read this crate still shells out for.
Not worth blocking F3-10 on.
