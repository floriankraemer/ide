# 0029. Workspace-edit resource operations, performed by `app-core` as `FileOp`

## Status

Accepted

## Context

`textDocument/codeAction` and `textDocument/rename` do not only edit text.
rust-analyzer's "move to module", TypeScript's file rename and most Java IDEs' extract-to-file all ask the client to create, rename or delete a file as part of the same `WorkspaceEdit` a text edit rides in.
This client advertised `resourceOperations: []` and refused any edit that carried one outright — `lsp_core::workspace_edit::parse_workspace_edit` still does, on purpose, for the callers that splice straight into an open buffer and have nowhere to put a file rename.
The refusal was total: a server offering a text edit *and* a file move got nothing applied, with a message the user could not act on.

F2-1/F2-2 built the two Qt-free halves before this ADR: `lsp_core::workspace_edit::{ResourceOp, WorkspaceChanges, parse_workspace_changes}` parses and orders create/rename/delete steps interleaved with text edits, and `app_core::{FileOp, apply_file_ops}` performs one, including retargeting an open tab whose file moved.
Neither had a caller. This is the seam between them.

## Decision

### 1. `app-core`'s `FileOp`, not `lsp-core`'s `ResourceOp`, is what gets performed

Performing a resource operation means deciding what happens to an open tab whose file moved under it — a rule about this application's state, not about the protocol.
`app-core` may not depend on `lsp-core` (`docs/architecture/layering.md`), so the bridge maps `ResourceOp` onto `FileOp` field for field and decides nothing else — the same line `rename_entry`'s tree-driven rename already draws between "what the tree asked for" and "what renaming a file means for open tabs."

### 2. Every resource operation runs first, as one all-or-nothing step, before any text edit is written

The protocol's own rule is subtler — `documentChanges` is applied in the order the server sent it, and a server that renames a type and then renames its file to match sends the text edit first and the rename second on purpose.
This client does not implement that full interleaving.
`lsp_core::EditPlan` gained an `ops: Vec<ResourceOp>` field and a `plan_changes()` sibling to `plan_edit()` that fills it; at apply time (`LanguageService::take_pending_edits`) every op in that list runs through `AppSession::apply_file_ops` — itself already all-or-nothing, aborting before any operation past the first failure — and only once that whole step succeeds are the plan's text edits spliced or written.

Chosen over faithful step-by-step interleaving because no server in the conformance suite depends on it, and the failure mode of getting it wrong is asymmetric: an out-of-order resource operation here means "the refactoring did nothing" (`ResourceOpError` refuses cleanly), while a naive interleaved apply that got the order wrong risks writing a text edit against a path that operation had already renamed out from under it — corrupting a file, the bar ADR-0019 set for "worse than refusing."

### 3. A resource operation always shows the preview

`EditPlan::touches_other_files` — the flag that decides "apply straight away" from "show the preview first" — is set whenever `ops` is non-empty, whatever else is in the plan.
Creating, renaming or deleting a file is never a same-file change, even when the only *other* thing in the plan is that one operation and the accompanying text edit is confined to the file the gesture started in.

### 4. The preview lists a resource operation as what it is

`RefactorPreviewDialog` already groups rows by file path for a rename or a multi-file edit.
A pending create/rename/delete gets its own row — "Create file", "Rename to X", "Delete file" — ahead of the text-edit rows, from a new `pendingOps()`/`FfiResourceOp` pair alongside the existing `pendingEdits()`.
The confirmation text also says how many files are being created, renamed or deleted (`FfiRefactorSummary::op_count`), so "this also creates two files" is said before the user commits, not discovered after.

### 5. A renamed or deleted open tab is retargeted the same way a tree-driven rename retargets it

`AppSession::apply_file_ops` returns the same `Vec<RetitledTab>` `rename_entry`/`delete_entry` already return, and `LanguageService` gained the same `tabTitleChanged` signal `ProjectTreeModel` already emits for it — wired once, in `RefactorController`'s constructor (it already owns both `LanguageService` and `EditorTabs`), rather than growing `main_window.cpp`, which had no headroom left under ADR-0025's ceiling.

## Consequences

- A quick fix or refactoring that moves code to a new file — "move to module", "extract to file", a rename that renames its file to match — now applies instead of being refused outright.
- Resource operations are not individually excludable from the preview the way a text-edit row is (unticking a file only removes its text edit from `to_ffi_edits`); the checkbox on an operation's own row is currently cosmetic. Acceptable for now because every resource-op-bearing action seen in the conformance suite is one atomic operation, not a batch a user would want to partially apply — revisit if that changes.
- `crates/lsp-core/src/bin/stub_server.rs` gained a canned resource-operation reply (`codeAction` at line 5: create a file, write its content, then edit the original) so this path has offline coverage the way every other F2 surface does, and one E2E flow (`e2e_intention_creates_a_file_through_the_preview`) drives it end to end: `Alt+Enter` → the preview lists two files → accepting writes the created file to disk and splices the edit into the open buffer's undo stack.

## Alternatives rejected

**Continuing to refuse resource operations wholesale.** The status quo before this ADR — correct, and useless for exactly the refactorings that need it.

**Performing resource operations in `lsp-core`.** It would own tab-retargeting policy, which is `app-core`'s, and `app-core` may not depend on `lsp-core` to get that decision back.

**Performing them in `bridge.rs` (the C++ seam).** Deciding what happens to an open dirty buffer whose file was renamed is a rule, and rules get unit tests — `app_core::file_ops`'s test module, not `cpp/`, is where that lives.

**Faithful step-by-step interleaving of resource operations and text edits.** Discussed in Decision §2. The right answer eventually, if a server that needs it shows up in the conformance suite; not needed yet, and a bigger change than the client's actual gap justified.
