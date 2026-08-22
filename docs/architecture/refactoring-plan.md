# Refactoring: rename, extract via code actions, and a signature on hover

## Progress

Living status table — update the relevant row(s) **in the same commit** that finishes a task, so status and code never drift apart.
A fresh session should read this table (and `git log`) before picking up work, per `CLAUDE.md`.

| Task | Status | Commit |
|---|---|---|
| RF1 | done | `07eb425` (#42) |
| RF2 | done | `63fc452` (#43) |
| RF3 | done | `f1bb231` (#44) |
| RF4 | done | `a05e18f` (#45) |
| RF5 | done | `21fe569` (#46) |
| RF6 | done | `e3c7ff9` (#47) |
| RF7 | done | `be85663` (#48) |
| RF8 | done | `d497453` (#49) |
| RF9 | done | `d497453` (#49, shipped with RF8) |
| RF10 | done | `3faa3d7` (#50) |
| RF11 | done | `3faa3d7` (#50) |
| RF12 | done | `5e9040d` (#52) |
| RF13 | done | this change (#53) — a commit cannot name its own hash, so the PR is the reference |

One defect was found by driving RF10/RF11 end to end and fixed in `da13e3b` (#51): the name-based rename wrote open files to disk, so the editor prompted about its own change and the rename could not be undone where the user could see it.

## Context

After the language platform (ADR-0016 and its plan), the IDE could read code well — 31 tree-sitter languages, a project symbol index, name-based navigation, and a working LSP client with diagnostics, hover, go-to-definition and completion.
It could not change code structurally: no rename, no code actions, and no way to apply an edit spanning more than one file except Find and Replace.

This plan added three things: Extract Method and Extract Class through LSP code actions, rename with a name-based fallback, and a signature tooltip for the languages no language server serves.

The full reasoning lives in [ADR-0019](decisions/0019-lsp-refactoring.md); this document carries the task breakdown and status forward.

## Key design decisions

1. **The servers refactor; we apply.**
   No refactoring logic is written here. Extraction exists where a language server implements it, and a hand-written tree-sitter engine is backlog rather than a gap.
2. **Rename prefers the server, falls back to the index, and is honest about the difference.**
   The fallback labels every site, unticks the uncertain ones when a name is ambiguous, and refuses outright while any buffer is unsaved.
3. **`workspace/applyEdit` is routed, never answered on the read thread**, and its gate can be answered exactly once — with closing beating a late claim, so the reply is never a lie.
4. **Open buffers are spliced, closed files are written**, and which is which is decided in Rust. One Ctrl+Z undoes a refactoring in each file the user can see.
5. **A `WorkspaceEdit` is not a `FileReplacement`.** That type is single-line; an extract-method range is not.
6. **Everything is all-or-nothing** at every level, with three separate staleness checks, because half an extract is a corrupted program.

## Tasks

| # | Task | Deliverable |
|---|---|---|
| RF1 | `workspace_edit.rs` | `WorkspaceEdit` parsing (both payload shapes), resource-operation rejection, `apply_to_text` with UTF-16→byte conversion, descending order and all-or-nothing application |
| RF2 | Plan + gate | `EditPlan` splitting a parsed edit into buffers and files, `touches_other_files`, the version rule, and `EditGate` |
| RF3 | Stub-server support | Canned `codeAction`, `codeAction/resolve`, `prepareRename`, `rename`, `executeCommand`, and a command that sends `workspace/applyEdit` back and blocks |
| RF4 | `code_action.rs` | Both entry shapes, dotted-kind prefix matching, the `only`-then-unfiltered retry, `needs_resolve`, edit-then-command ordering, disabled items |
| RF5 | Inbound `applyEdit` | `apply_edit.rs`'s gate and session counter, `LspEvent::ApplyEdit`, and the routing that keeps the read thread free |
| RF6 | `rename.rs` + manager surface | Both outcome rules, the four `prepareRename` shapes, five typed requests, `REFACTOR_TIMEOUT`, and the capabilities that make servers answer |
| RF7 | `index-core` fallbacks | `plan_index_rename` with its confidence and refusal rules, `write_files`, `declaration_signature` |
| RF8 | Bridge: language service | Code actions, rename, one pending refactoring, and the `settle` discipline that never leaves a server waiting |
| RF9 | Bridge: index side | `applyFileEdits`, the name-based rename plan, and its per-path buffer/disk split |
| RF10 | View: apply + preview | The `QTextCursor` splice in one edit block per file, and `RefactorPreviewDialog` |
| RF11 | View: controller + menu | `RefactorController`, the Refactor menu, and four `ActionDef`s |
| RF12 | Hover fallback | `hover_outcome`, the index leg with its own `HoverTracker`, and the no-server path that previously asked nobody |
| RF13 | Docs | ADR-0019, this document, the ADR-0016 status paragraph, and the `layering.md` bullet |

## What a human should click through

None of this has been seen on a real machine with a real language server; the environment that built it has no display beyond Xvfb and no server installed.
What *was* driven end to end under Xvfb, with no language server: the Refactor menu and its shortcuts, Shift+F6 on a function, the preview naming its three sites and stating they were found by name, the rename applying to all three, one Ctrl+Z putting them back, and a hover showing `fn helper(a: i32) -> i32`.

Worth a human's attention, in rough order of risk:

1. **Extract Method with rust-analyzer**, on a selection inside a function. Then Ctrl+Z.
2. **Extract with a command-driven server** — jdtls (Java), intelephense (PHP) or omnisharp (C#). This is the `workspace/applyEdit` path, and the only one where a server is left blocked while a dialog is open.
3. **A rename that crosses files** with a server running: the preview should list them, and unticking one should leave that file untouched.
4. **A rename in a language with no server** (Zig, Lua, Haskell): the preview must say the sites were found by name, and unticked rows must stay unticked when the name is ambiguous.
5. **Leaving the preview open for a minute** during a command-driven extract: the server should be told the edit was not applied, nothing should be written, and the application should stay responsive.
6. **Killing a server mid-rename**: the status bar should report a failure and nothing should change.

## Known gaps

- **No extraction without a language server.** Backlog by decision (ADR-0019); the alternative is a per-language refactoring engine.
- **No file create, rename or delete in a `WorkspaceEdit`.** Advertised as unsupported and refused whole. Supporting it means teaching `AppSession`'s tab policy to follow a renamed file.
- **Undo is per-file**, and a closed file's rewrite is not undoable at all — the same ceiling project-wide Replace has. The preview says so.
- **`codeAction/resolve` is used for `edit` only**; a command-carrying unresolved item is executed as it arrived.
- **The name-based rename refuses while any buffer is unsaved.** A per-file rule would be friendlier and much harder to explain; this one is a single sentence.
- **The signature fallback is a bracket-balance heuristic** capped at five lines. It will include a trailing comment and can miscount a bracket inside a string. Asking the grammar for the declaration node's range is the upgrade.
- **`openFileAtLine` still treats the index's byte column as a character offset** — pre-existing, inherited by the preview's jump-to-site, and wrong only on lines with multi-byte characters before the symbol.
- **The Class View outline does not follow an unsaved refactoring.** After a rename is spliced into a buffer, the outline still lists the old name until the file is saved. Pre-existing behaviour for any unsaved edit, more noticeable now.
