# 0032. Run configurations: a PTY-backed console, ANSI-stripped for v1, one supervisor QObject for N sessions

## Status

Accepted

## Context

`docs/architecture/next-five-features-plan.md`'s F4 ("Run configurations and console") lane needed a way to define, launch, and watch a project's own run commands — `cargo run`, `npm start`, a `Makefile` target — from inside the IDE, plus a second, unrelated feature riding the same transport: a terminal dock that can hold more than one independent shell.
`run-core` (F4-3…F4-8), the `RunService`/`RunConfigEditor` bridge (F4-9/F4-10), the run console dock/toolbar/dialog/link view (F4-11…F4-13), and the terminal's move to multi-session (F4-14a/b) are already built and committed on this branch.
This ADR records the decisions actually made across those commits — several of which sharpened or narrowed the plan's original framing — plus F4-15's two E2E flows and the answer to the plan's own risk #11 about the AI agent.

## Decision

### 1. A run configuration launches over a PTY, through the same transport the terminal uses

`run_core::launch::LaunchSpec { program, args, cwd, env }` is the shared, debugger-agnostic half of a launch: `Supervisor::launch` turns one into a `pty_core::PtySession`, exactly the type `TerminalSupervisorRust` spawns for an interactive shell.
A plain-pipes console was rejected (recorded in the plan's own ADR-0029 draft, carried forward unchanged here): most build tools buffer differently, and some disable colour outright, the moment stdout is not a tty, so a pipe-backed console can sit silent until the process exits — the exact failure a working PTY transport already avoids.

`LaunchSpec` deliberately knows nothing about consoles, ANSI, or Qt.
The plan named this as `dap-core` readiness — a future Debug Adapter Protocol client turns the same `LaunchSpec` into a DAP `launch` request body — and nothing built for F4 forecloses that: `RunConfig → LaunchSpec → PtySession` is the whole path from a project's configuration to a spawned process, with no debugger-shaped assumption baked into any of the three.

### 2. Console output is ANSI-stripped in v1, not SGR-colored — a scope cut, not an oversight

`bridge/run/mod.rs` runs a new stateful `AnsiStripper` over each console's output before it reaches the view, rather than resolving SGR codes to per-run colour spans the way `terminal-core` does for the terminal grid.
This was a deliberate v1 simplification made during F4-11/F4-12 (commit efd9518), not the plan's original design: the plan's own instruction was to reuse `terminal-core`'s SGR→RGB resolution rather than write a second ANSI parser in C++, and stripping still honours that — it is the same parser's *state machine*, just used to discard escape sequences rather than resolve them to colour.

The reason to stop short of full coloring in v1: `RunService::resolveLink` (F4-13, Ctrl+Click file:line jumping) indexes into the *same cached text* the view displays, by byte offset.
Per-run `FfiStyledRun` coloring would mean either two parallel copies of a console's output (plain for link resolution, styled for display) that must never drift apart, or teaching `resolveLink` to walk styled runs instead of a flat string.
Stripping keeps one string, byte-for-byte identical between what `resolveLink` reads and what the `QPlainTextEdit` shows, for the cost of a console that reads like piped `cargo build` output rather than a real terminal.

**Follow-up path, not designed further here**: a real `FfiStyledRun` vector alongside the plain text, with `resolveLink` walking the plain copy and the view applying styled runs as `QTextCharFormat` ranges over the same offsets — deferred because nothing in F4's own test-strategy or the plan's risk table requires it for v1, and it is additive over what exists.

**Superseded by R2-1/R2-2** (`docs/architecture/run-build-debug-parity-plan.md`), along exactly that follow-up path.
The console now resolves SGR rather than stripping it: `terminal_core::SgrResolver` is a second *sink* on the parser the terminal grid already drives, `run_core::AnsiResolver` hands its styled runs on, and `AnsiStripper` survives as a wrapper that discards them for `build-core`'s diagnostic parsing.
The dilemma this section describes did not have to be resolved either way: the runs are offsets *into* the plain text rather than a second copy of it, so `resolveLink` still walks one string.
The one thing that did have to be decided is which unit those offsets are in — `run-core` measures in UTF-8 bytes and `QTextCursor` counts UTF-16 code units, so `RunService` converts at the seam, which is where the two representations meet.

### 3. One `TerminalSupervisorRust` QObject owns N sessions — not N QObject instances

The plan's F4-14a framing ("N `TerminalSession` QObjects instead of one") turned out not to be mechanically available in this codebase's cxx-qt integration, discovered during that task (commit 4036e21) rather than assumed up front: cxx-qt registers a `#[qobject]`-tagged type's `QMetaObject` once, at build time.
C++ can `new` more `QObject`s of a *type* the bridge declares, but there is no mechanism for the view to ask Rust for a fresh, independently-backed instance of a cxx-qt bridge type at runtime — every other multi-instance object in this codebase (dock widgets, dialogs) is a plain `QWidget`/`QDialog`, never a `#[qobject]` type constructed more than once.

The shape actually built mirrors `RunServiceRust`, which had already solved the real underlying problem — N independent backend lifecycles behind one adapter — with a `HashMap<u64, ..>` keyed by an id the view carries per tab.
`TerminalSupervisorRust` does the same: one QObject, a map of session id to an independent `PtySession` + `TerminalEmulator` pair, one reader thread per session queuing `gridUpdated(sessionId)`.
`TerminalSessionsPanel` (view) owns a `QTabWidget`, creating a session via `newSession()`/`start()` per tab and closing it via `kill_tree` (not `kill`) on tab close, so a shell's backgrounded children never outlive their tab — the same "no orphan" guarantee `pty_core::PtySession::kill_tree` already gives run consoles.

### 4. The terminal's existing `linkAt()` is left alone, not retrofitted to `run_core::links`

The plan named unifying the terminal's Ctrl+Click link detection with `run_core::links` as a possible F4-8 extension.
It was judged out of this branch's time budget (recorded in the plan's own task table) rather than attempted and reverted: `linkAt()`/`TerminalSession` already resolves URLs via a distinct, working, tested mechanism unrelated to `file:line` locations, and retrofitting it was not required for anything F4 actually shipped.
`run_core::links::resolve_link` is used only by the run console (F4-13); the terminal dock's own link handling is unchanged by this branch.

### 5. The E2E flows (F4-15) — what they cover, and what they intentionally do not

`run-core`'s own unit tests already cover batching's time/size triggers, Cargo/npm/pnpm/yarn/Makefile detection, `resolve_link`'s file:line[:col] table, and the supervisor's kill-tree/escaped-child reporting — all of it against plain values, with no Qt event loop.
Per the plan's placement rule ("if a test would still be meaningful with the Qt event loop removed, it must not be an E2E test"), F4's two flows were chosen to cover only what a Qt-free crate structurally cannot reach:

- **`e2e_run_and_stop_shows_console_output`** — `run.run`/`run.stop` turning into the right `RunService` call for whichever configuration `RunToolbar` has selected, and a background reader thread's output actually reaching the console dock's `QPlainTextEdit` across the cxx-qt thread boundary. The configuration is pre-seeded in the fixture's `.ide/settings.toml` rather than detected, since detection's own correctness is exactly what `run_core::detect`'s unit tests already prove, and re-proving it here would only add a compiler's worth of runtime for nothing new.
- **`e2e_run_config_dialog_persists_across_relaunch`** — the Run Configurations dialog's own widgets driving `RunConfigEditor` to a real commit on disk, and a **second, cold process** picking the result back up through `RunToolbar`'s own `configurationsChanged` wiring. `RunConfigEditor`'s draft/commit logic is already unit-tested against a temp directory; nothing but a real dialog and a real relaunch can catch a picker that only ever reflects what happened to be in memory.

Driving the second flow surfaced a real bug in `run_config_dialog.cpp` (fixed in this same change, not deferred): `repaintList`'s `QSignalBlocker` meant `list->setCurrentRow()` after Add or Remove never fired `currentRowChanged`, so the form fields stayed on whatever they last showed — blank and disabled on a dialog's first Add — instead of loading the newly-selected row. The fix calls `loadForm()` explicitly after each `repaintList()`, matching the pattern the dialog's own initial setup already used.

Terminal multi-session's independent-lifecycle claim is **not** covered by an E2E flow: it already has a Qt-free unit test (`terminal.rs`'s shutdown-order test, reusing `pty-core`'s own `kill_tree_reaches_a_grandchild` idiom) proving that dropping the supervisor kills every open session's process tree, plus the manual Xvfb+xdotool verification recorded in commit 4036e21 (two tabs, two independent shells, closing one leaves the other's process running). With F4's flow budget fixed at two, the run console's cross-thread wiring and the settings-dialog round trip were judged the higher-value gap: the terminal's tab-per-session shape is structurally the same pattern `RunConsolePanel` already exercises end-to-end in flow one, so it carries lower marginal risk than an untested settings page.

## Risk #11 — does the AI agent gain a "run" tool from this?

No. ADR-0021 rejected shell execution from the AI agent's tool catalog structurally, not as a placeholder pending some future capability: the agent reads, searches, navigates and edits, and running commands stays the human's.
Owning a process-supervision machinery for the human-facing run console does not change that argument — a user-authored run configuration, reviewed and knowingly triggered by a person through `run.run`, and a shell command a model composed from a prompt (including one carrying injected instructions from a file the agent read) are not the same action with different UIs.
Nothing in `run-core`, `RunService`, or `RunConfigEditor` is reachable from `ai-chat-core`'s tool catalog, and no task in this plan proposes adding it there.

## Consequences

- `run-core` stays Qt-free (`docs/architecture/layering.md`'s existing row is unchanged by this ADR); `LaunchSpec` has no console-kind field yet, since only `Pty` is built — a `Pipes` variant or a `dap-core` consumer can extend the enum without touching `RunConfig`, `Supervisor::launch`, or the bridge.
- The run console's plain-text cache and `resolveLink`'s byte offsets stay coupled by construction (§2): any future colored-console work must keep them derived from the same string, not two.
- `TerminalSupervisorRust`'s one-QObject-many-sessions shape is now precedent, not a one-off: any future feature needing N independently-lifecycled backend objects behind cxx-qt should reach for a `HashMap<u64, ..>`-keyed adapter, the way `RunServiceRust` and `TerminalSupervisorRust` both do, rather than exploring per-instance QObject construction again.
- `run_config_dialog.cpp`'s Add/Remove handlers now call `loadForm()` explicitly after `repaintList()`, closing the gap `QSignalBlocker` opened; any future settings dialog built on the same `commitForm`/`loadForm`/`repaintList` idiom (`language_servers_page.cpp`'s own template) should audit for the same gap rather than assume `currentRowChanged` fires on a blocked signal.

## Alternatives rejected

**Plain pipes for the run console.** Rejected in the plan's own ADR-0029 draft and unchanged here: colour and buffering both change the moment stdout is not a tty, and a working PTY transport already exists in `pty-core`.

**A second ANSI/SGR parser in C++, for the console view.** Rejected. `AnsiStripper` reuses the same state-machine shape `terminal_core::TerminalEmulator`'s SGR resolution already established, rather than standing up an independent implementation in `run_console_panel.cpp` — the plan's explicit instruction, and the reason stripping (§2) was a scope cut on *what the parser's output is used for*, not a decision to parse ANSI twice.

**Full per-run `FfiStyledRun` coloring for v1.** Deferred, not rejected outright: real value (a console that reads like a terminal, not piped output), but it forces a choice between two copies of a console's text or a link resolver that walks styled runs instead of a flat string, and nothing in F4's test-strategy required it. Revisit if a colored console becomes a stated requirement rather than a nice-to-have.

**N independently-constructed `TerminalSessionRust` QObject instances.** Rejected as mechanically unavailable, not merely undesirable: cxx-qt's one-`QMetaObject`-per-type-per-build model has no runtime path to a fresh instance of a `#[qobject]` bridge type. `RunServiceRust`'s already-proven `HashMap<u64, ..>`-behind-one-adapter shape was reused instead of inventing new cxx-qt multi-instance plumbing.

**Retrofitting the terminal's `linkAt()` to `run_core::links`.** Deferred for time, not rejected on the merits: the plan named it as a possible unification, but the terminal's existing link handling already works and is tested, and nothing in F4 needed the two to share a table. Revisit if the two catalogues drift enough to become a maintenance cost.

**A `run` tool in the AI agent's catalog, now that the machinery exists.** Rejected, per risk #11 above and consistent with ADR-0021: a shell/exec tool converts every prompt-injected comment in a source file into arbitrary code execution, which is exactly what ADR-0021 already ruled out for the agent's reading and editing surface. Building the run console does not weaken that argument.
