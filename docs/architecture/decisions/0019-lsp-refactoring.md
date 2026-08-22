# 0019. Refactoring over LSP: code actions, rename, and applying a workspace edit

## Status

Accepted.
Implemented as tasks RF1–RF13 of [the refactoring plan](../refactoring-plan.md).

## Context

The IDE could read code and navigate it, but not change its structure.
There was no rename, no code actions, and the only way to edit more than one file at a time was Find and Replace.

Two things had to be decided before any of that could be built, and neither is display logic.

The first is where the refactorings themselves come from.
Extract Method and Extract Class are semantic operations: they need to know which locals escape a block, what its result type is, which captures a closure makes, whether a name is already taken in the target scope.
ADR-0008 recorded that this index is deliberately name-based, and ADR-0011 recorded that navigation ranks same-named candidates by proximity because it has no binding resolution to appeal to.
Nothing in this codebase can answer those questions, and writing something that could — per language, for 31 languages — is a project, not a task.

The second is how an edit gets applied at all.
Qt owns the live text buffer while `index-core` owns everything on disk, so "apply this edit" splits two ways; and `workspace/applyEdit` arrives on a thread that must answer it while the work it asks for can only happen on another.

## Decision

### The servers refactor; we apply

Extract Method and Extract Class come from `textDocument/codeAction` — from rust-analyzer, jdtls, intelephense, omnisharp, pylsp and their peers.
We write no refactoring logic at all.

The consequence is honest and worth stating plainly: extraction exists only where a language server is installed and implements it.
A language with a tree-sitter grammar and no server gets rename (below) and nothing else.
A hand-written tree-sitter refactoring engine is backlog, not a gap to be closed quietly — it would be a large, per-language effort that is semantically wrong at exactly the edges that matter (borrows, closure captures, generics, visibility).

### Rename prefers the server and falls back to the index

`rename::rename_outcome` is deliberately the same shape as `navigation::definition_outcome` (ADR-0016): a server's non-empty answer wins, and no server, none running, an error, a timeout and an empty answer all resolve to ADR-0011's name-based sites.
That fallback is what makes rename work at all for most of the catalog.

Because it is name-based, it is cautious in three ways, all of them rules in `index-core`:

- It is offered only when the caret actually resolved to a declaration.
  Renaming every token that happens to share a spelling is Replace in Files, which this application already has, and offering it under the word "rename" would misdescribe what it does.
- Every site is labelled `Resolved` or `Unverified`, and when more than one symbol in the project carries the name, the unverified ones start unticked.
  The safe action is the default; widening it is a deliberate click.
- It refuses outright while any buffer is unsaved, because the index reads from disk and every line it reports for a modified file may be stale.
  One rule and one message beats per-file semantics nobody can predict — and it is what makes reading a file to convert its columns sound.

`prepareRename` gets its own distinction: a `null` result is the server saying "not this element", while an error is it saying nothing at all.
Most servers do not implement the request, and treating their `-32601` as a refusal would take rename away exactly where it works.

### `workspace/applyEdit` is routed, never answered inline

Command-driven servers (jdtls, omnisharp, intelephense) answer an Extract with a command, and ask the client to apply the edit while that command is still running.
The request arrives on the thread that reads every message from that server, and the server blocks until it is answered — but applying needs the UI thread.
Answering inline would stall diagnostics and every in-flight response behind one dialog.

So `dispatch` only routes.
An edit that no refactoring asked for is refused on the spot, with no thread and without troubling the UI.
A wanted one is published as an event and waited on by a short-lived thread, of which there is at most one per gesture, because a gesture is what makes the request legitimate.

The gate that carries the answer can be answered exactly once — by the editor claiming the right to apply, by the editor refusing, or by the wait giving up — and **closing beats a late claim**, so an editor that arrives after the timeout is told no and applies nothing.
That is what keeps the reply from being a lie in either direction, and it is why `claim()` returns a bool rather than being a statement of intent.

**The invariant that must not be broken:** the Qt thread never blocks on the LSP worker.
`push_job` is fire-and-forget, and a synchronous `LanguageService` call that waits on the worker would close the loop and deadlock.

### Open buffers are spliced; closed files are written

Which documents are open is decided in `lsp_core::plan_edit`, not in the view, and each edit crosses the seam carrying that verdict.
An open document is spliced with `QTextCursor` inside one edit block, so **one Ctrl+Z undoes the whole refactoring** in each file the user can see.
A closed one is rewritten whole and re-indexed.

This applies to the name-based rename too.
It originally wrote every file to disk, and the watcher then told the user their open file "was modified outside the editor" — about a change the editor had just made at their request, and with no undo.
Open files are now taken out of the plan and spliced like any other buffer edit.

A `WorkspaceEdit` is **not** expressed as `index_core::FileReplacement`.
That type is a single-line span, and an LSP range routinely spans lines — which is what every extract-method edit does.
Whole text in, whole text out is the only shape that can carry one.
The single place `FileReplacement` is still right is the name-based rename, whose sites genuinely are single-line spans of a known length, so `replace_in_files` applies them unchanged rather than a second applier being written.

### Everything is all-or-nothing, at every level

A range that does not fit refuses its file; a stale version refuses the whole edit; a resource operation refuses the whole edit.
Half an extract-method is a corrupted program, and a rename that reaches four files out of five is worse than one that reaches none.
This is the rule `replace_in_files` already applied to a span it could no longer place, carried up.

Three checks guard staleness, because one is not enough.
The entry points flush a `didChange` before requesting (the editor's own sync is debounced 300 ms, so the manager's version alone can be behind).
`workspace_edit` rejects an edit naming a version this client never sent.
And `EditGate` — the sibling of `HoverTracker` — compares the editor's buffer revision, because what the server was told and what the editor holds can differ.

### File create, rename and delete are refused

`workspaceEdit.resourceOperations` is advertised as empty, so a conforming server never sends one, and an edit containing one anyway is refused whole.
Supporting them means teaching `AppSession`'s tab policy to follow a renamed file and invalidating `TabId`s — a separate change with its own edge cases.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| Write our own tree-sitter extract engine | Per language, for 31 languages, and wrong at exactly the edges that matter. Backlog, and named as such rather than half-built. |
| Lower a `WorkspaceEdit` onto `FileReplacement` | Impossible, not merely awkward: that type addresses one line, and an extract-method range spans many. |
| Apply every edit by writing to disk | Loses undo in the file the user is looking at, and makes the editor prompt about its own change. This was tried, seen, and fixed. |
| Answer `workspace/applyEdit` on the read thread | Deadlocks the moment a preview dialog opens: diagnostics and every pending response queue behind it. |
| Refuse `workspace/applyEdit` entirely | Silently removes Extract from jdtls, omnisharp and intelephense — i.e. from Java, C# and PHP. |
| Rename by name with no preview | Name matching cannot tell two same-named symbols apart (ADR-0008). Writing across a project on that basis, unseen, is not a refactoring. |
| Support resource operations now | Needs tab-follow policy in `app-core`; refusing loudly is better than applying the text half of a file rename. |

## Consequences

- Positive: every rule is unit-tested without Qt and without a language server — the parsers, the plan split, the version checks, the gate's race, the rename plan's confidence and the signature heuristic. The protocol round trips are integration-tested against the X2 stub server, offline.
- Positive: `bridge.rs` and `cpp/` decide nothing. Whether a preview is needed, which pile an edit belongs to, which sites start ticked, whether an answer is still fresh — all of it arrives as a flag or a signal.
- Positive: adding Inline, Change Signature or Move later is UI work only; the pipeline underneath them already exists.
- Negative / accepted: no extraction without a language server.
- Negative / accepted: undo is per-file, and a closed file's rewrite is not undoable at all — the same ceiling project-wide Replace already has, and the preview says so.
- Negative / accepted: `codeAction/resolve` is used for `edit` only; a command-carrying unresolved item is executed as it arrived.
- Negative / accepted: the hover signature fallback is a bracket-balance heuristic capped at five lines. Asking the grammar for the declaration node's range is the upgrade if it proves annoying.

## Related

- [ADR-0016: LSP client](0016-lsp-client.md) — the client this builds on, and the LSP-over-index precedence this reuses twice more.
- [ADR-0011: code navigation](0011-code-navigation.md) — the name-based resolution the rename fallback and the hover fallback both stand on.
- [ADR-0008: project index](0008-project-index.md) — why that resolution is name-based, and what it therefore cannot promise.
- [ADR-0003: FFI seam conventions](0003-ffi-conventions.md) — the typed-result discipline the new seam types follow.
- `crates/lsp-core/src/{workspace_edit,code_action,rename,apply_edit}.rs` — the modules this ADR documents.
