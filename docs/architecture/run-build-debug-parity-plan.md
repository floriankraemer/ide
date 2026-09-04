# Run, build and debug parity plan

## Context

The IDE ships a working but minimal run story, and has no build or debug story at all.

What exists at the time this plan was written:

- `run-core` — `RunConfig` (id, name, program, args, cwd, env), `LaunchSpec`, `ConsoleKind::{Pty, Pipes}` with `Pipes` declared but unused, a PTY `Supervisor` carrying N consoles, `detect` for Cargo binaries, `package.json` scripts and Makefile phony targets, output batching over a bounded ring, and `resolve_link` for Ctrl+Click on `file:line[:col]`.
- `ui-shell` — `RunServiceRust` with an ANSI stripper, `RunConfigEditorRust`, and the C++ `RunToolbar`, `RunConsolePanel`, `run_menu` and `run_config_dialog`.
- Per-project persistence of `[[run_config]]` blocks in `<project>/.ide/settings.toml`, scoped by `settings-model` as `runConfigs`.

ADR-0032 recorded those decisions and left three openings on purpose: SGR-coloured console output, a `Pipes` console for a future DAP client, and `LaunchSpec` as the debugger-agnostic launch seam.

What does not exist: typed run configurations, run from context, before-launch tasks, any build-tool invocation, any compiler-diagnostic parsing, and any debugger at all.
`RunToolbar` deliberately omits the mockup's Debug and Build buttons because neither has a backing command anywhere in the codebase.

This plan closes that gap against three IntelliJ IDEA help pages — running applications, compiling applications, and debugging code — across four toolchains: Cargo, CMake/C++, Python, and Maven/Gradle on the JVM.

## Progress

Living status table — update the relevant row(s) **in the same commit** that finishes a task, so status and code never drift apart.
A fresh session should read this table (and `git log`) before picking up work, per `CLAUDE.md`.

Task ids are stable; titles may change.
`blocked on X` means the task cannot start until X lands, not that it is unscheduled.
`open` means not started; this plan is the first in the repo to need that status, because it is written before the work rather than alongside it.

### R0 — the plan itself

| Task | Status | Commit |
|---|---|---|
| R0-1 — this plan doc + `docs/README.md` index line | done | (#179) |

### R1 — typed run configurations, macros, run from context

| Task | Status | Commit |
|---|---|---|
| R1-1 — `run_core::toolchain`: the toolchain table, `detect` rewritten on top of it | done | this branch |
| R1-2 — typed configurations: `toolchain` + `target` on `RunConfig`, back-compatible serde in `app-config` | done | this branch; a pair of strings rather than the enum this plan predicted, so `app-config` keeps depending on nothing |
| R1-3 — `run_core::macros`: `$PROJECT_DIR$`, `$FILE_PATH$`, `$FILE_DIR$`, `$FILE_NAME$`, `$USER_HOME$` in cwd, args and env | done | this branch |
| R1-4 — `run_core::context`: the configuration a file implies, temporary configurations with a cap | done | this branch |
| R1-5 — `allow_parallel` policy on top of the existing N-session supervisor | done | this branch; enforced in `RunService::launch`, exposed as a checkbox in the run-configuration dialog |
| R1-6 — adapter: `canRunFile`, `runContext`, toolchain/target/temporary/parallel across the FFI | done | this branch; one `runContext(path)` rather than a separate `runCurrentFile` — the menu action and the gutter icon are the same call |
| R1-7 — view: gutter Run icon, `run.runContext` (Ctrl+Shift+F10), allow-parallel checkbox | done | this branch; the icon sits on the first line (naming the entry point needs the symbol index) and the recent-first combo ordering is deferred — it needs a recency the settings file does not record |
| R1-8 — ADR-0039 + `layering.md` bullet + `docs/README.md` index line | done | this branch |
| R1-9 — E2E: `e2e_run_from_context_creates_a_temporary_configuration` | done | this branch; the run flows moved to their own `e2e_run.rs` binary because `e2e.rs` is at its size ceiling, and the flow drives `run.runContext` rather than a pixel-located gutter click |

### B1 — `build-core` and the Build dock

| Task | Status | Commit |
|---|---|---|
| B1-1 — new crate `build-core`: `BuildSpec` → the steps a request runs | done | this branch; over a PTY rather than `ConsoleKind::Pipes` — only the PTY transport can kill a build's process tree, so `Pipes` stays reserved for `dap-core` |
| B1-2 — Cargo diagnostics via `--message-format=json` | done | this branch |
| B1-3 — text diagnostic parsers (javac/Maven/Gradle, CMake/gcc/clang) + the streaming `DiagnosticParser` | done | this branch; its own table rather than `run_core::links`' — the two want different fields out of the same text |
| B1-4 — `BuildDiagnostic` in the shape `problems_panel` already renders | done | this branch |
| B1-5 — Build, Rebuild, build one target | done | this branch; "build file" dropped — no toolchain here addresses a single source file, they address targets |
| B1-6 — adapter: `BuildServiceRust`, one QObject for N builds, a thread per build | done | this branch |
| B1-7 — view: Build dock, the "&Build" menu, the Build toolbar button, build rows in the Problems dock | done | this branch; the Problems dock already had a Source column, so build rows only had to be a second source |
| B1-8 — ADR-0040 + `layering.md` row, verification gates, CI layering gate | done | this branch; the gate also gained the `run-core` rows it never had |
| B1-9 — E2E: `e2e_build_failure_populates_problems_dock` | blocked on B1-7 |  |

### B2 — before-launch tasks

| Task | Status | Commit |
|---|---|---|
| B2-1 — `BeforeLaunchTask` in `run-core`; persistence in `.ide/settings.toml` | blocked on B1-1 |  |
| B2-2 — sequential fail-fast execution; a failed build cancels the launch | blocked on B2-1 |  |
| B2-3 — cycle detection for `RunAnotherConfiguration` | blocked on B2-1 |  |
| B2-4 — view: the Before launch list in the run-configuration dialog | blocked on B2-2 |  |

### D1 — `dap-core` foundation

| Task | Status | Commit |
|---|---|---|
| D1-1 — new crate `dap-core`: `framing` over stdio, shaped like `lsp-core`'s | blocked on R1-1 |  |
| D1-2 — `protocol`: typed requests, responses and events | blocked on D1-1 |  |
| D1-3 — `session`: initialize → launch/attach → configurationDone, capability flags | blocked on D1-2 |  |
| D1-4 — `catalog`: codelldb, debugpy, java-debug, plus user overrides | blocked on D1-3 |  |
| D1-5 — `supervisor`: adapter lifecycle, restart, crash containment | blocked on D1-3 |  |
| D1-6 — `runInTerminal` handed back to `run-core`'s PTY supervisor | blocked on D1-5 |  |
| D1-7 — ADR-0041 + `layering.md` row for `dap-core` | blocked on D1-1 |  |

### D2 — breakpoints

| Task | Status | Commit |
|---|---|---|
| D2-1 — `BreakpointStore`: line, enabled, condition, hit condition, log message, temporary, suspend policy, dependent | blocked on D1-3 |  |
| D2-2 — function breakpoints, data breakpoints, exception filters | blocked on D2-1 |  |
| D2-3 — `shift_lines` driven from the existing buffer-edit seam | blocked on D2-1 |  |
| D2-4 — persistence under `.ide/local/` | blocked on D2-1 |  |
| D2-5 — view: gutter toggle, breakpoints dialog, Mute Breakpoints | blocked on D2-2 |  |

### D3 — the debug session and its tool window

| Task | Status | Commit |
|---|---|---|
| D3-1 — adapter: `DebugServiceRust`, one QObject for N sessions | blocked on D1-5 |  |
| D3-2 — stepping: over, into, force into, out, run to cursor, smart step into, resume, pause, stop | blocked on D3-1 |  |
| D3-3 — Threads and Frames views | blocked on D3-1 |  |
| D3-4 — Variables with lazy expansion, Set Value | blocked on D3-3 |  |
| D3-5 — Watches and Evaluate Expression | blocked on D3-4 |  |
| D3-6 — the debugger console | blocked on D3-1 |  |
| D3-7 — inline values from the current frame's scopes | blocked on D3-4 |  |
| D3-8 — view: the Debug toolbar button, `run.debug`, capability-gated action enablement | blocked on D3-2 |  |
| D3-9 — E2E: `e2e_breakpoint_hits_and_variables_populate` | blocked on D3-4 |  |

### D4 — the remaining debugger surface

| Task | Status | Commit |
|---|---|---|
| D4-1 — attach to a local process | blocked on D3-1 |  |
| D4-2 — remote `attach` configurations | blocked on D4-1 |  |
| D4-3 — per-language exception breakpoints wired to the catalog | blocked on D2-2 |  |
| D4-4 — reload changed classes where the adapter exposes it | blocked on D3-1 |  |
| D4-5 — multiple simultaneous sessions with session tabs | blocked on D3-1 |  |
| D4-6 — the four-toolchain debug matrix, recorded in this doc | blocked on D3-9 |  |

### R2 — console and run-widget ergonomics

An independent second lane; it may run at any point after R1.

| Task | Status | Commit |
|---|---|---|
| R2-1 — SGR colour as `FfiStyledRun` beside the unchanged plain text, per ADR-0032's named path | blocked on R1-6 |  |
| R2-2 — `AnsiStripper` becomes `AnsiResolver` reusing `terminal-core`'s SGR state machine | blocked on R2-1 |  |
| R2-3 — console find, pin tab, scroll lock, clear | blocked on R1-6 |  |
| R2-4 — soft terminate before `kill_tree` | blocked on R1-6 |  |
| R2-5 — Show Running List over `Supervisor::active_ids` | blocked on R1-6 |  |
| R2-6 — the terminal's `linkAt()` unified with `run_core::links` | blocked on R1-6 |  |

## 1. Decisions resolved before work starts

**Build is delegated, never owned.**
We invoke `cargo`, `cmake`, `gradle` and `maven`, parse their diagnostics, and navigate them.
We do not model compilation output folders, module output paths, artifacts, or build-automatically-on-save.
LSP already gives live errors, so an auto-build would duplicate a signal the user already has, and an output-path model is meaningful almost only on the JVM.
IntelliJ's compilation-output-folders page is therefore satisfied by reading the tool's own layout rather than by configuring ours.

**The debugger is a DAP client.**
A Qt-free `dap-core` shaped like `lsp-core`: blocking threads, `Content-Length` framing, a supervised child process, a catalog plus user overrides.
One client, N adapters — codelldb for Rust and C/C++, debugpy for Python, java-debug for the JVM.
*Rejected*: driving gdb/lldb machine interface directly (a bespoke protocol per debugger, no adapter ecosystem, and nothing to reuse for the JVM).

**One toolchain table, in `run-core`.**
`run_core::toolchain` is the single source of truth for which build tool a project uses and what its run argv, build argv and default debug adapter are.
`build-core` and `dap-core` consume it rather than each detecting again, per the layering rule against a second detection table.
File-to-language detection still comes from `syntax-core`'s registry (ADR-0018).

**`LaunchSpec` stays the seam.**
Run uses it with `ConsoleKind::Pty`, build with `Pipes`, and debug turns the same struct into a DAP `launch` body.
Nothing re-derives how a process is started.

**Three new E2E flows, and only three.**
The budget is 12–15 flows forever, and stands at 12.
R1, B1 and D3 take one each; no phase gets a fourth.

Out of scope, stated once: code coverage, CPU and memory live charts, run targets (Docker, SSH, WSL), artifact packaging, the run dashboard and Services tool window, stream and async debugging, and hot swap beyond whatever an adapter offers for free.

## 2. Architecture

### New crates

| Crate | Layer | Why not an existing crate |
|---|---|---|
| `build-core` | support | `run-core` launches a *user program* for a console; build invokes a *tool* and parses structured diagnostics out of it. Folding the parsers into `run-core` would make every run console carry a compiler-diagnostic dependency it never uses. |
| `dap-core` | support | The debug adapter protocol is a client with its own framing, session state machine and catalog, exactly parallel to `lsp-core`. It is not an adapter concern and must be unit-testable without Qt. |

### Layering rows

| Crate | Allowed imports | Qt/cxx-qt allowed |
|---|---|---|
| `run-core` | `pty-core`, `app-config`, `terminal-core` (+ std, serde, toml, serde_json, regex) — unchanged | **No** |
| `build-core` | `run-core`, `app-config`, `syntax-core` (+ std, serde, serde_json, regex) | **No** |
| `dap-core` | `run-core`, `app-config`, `syntax-core` (+ std, serde, serde_json) | **No** |

`dap-core` takes a normal dependency on `syntax-core` for the same reason ADR-0035 gave `lsp-core` one: the adapter for a session is chosen by language id, and that id must not be re-derived from a second extension table.

Both new crates take the tokio gate as well as the Qt gate: long work runs on a `std::thread` and returns through `CxxQtThread::queue()`, never on an ambient runtime.

### Where logic may live

- `run-core` owns the toolchain table, macro expansion, before-launch task ordering, and the run console's link table. It must not grow a diagnostic parser.
- `build-core` owns build invocation and diagnostic parsing. It must not re-derive file-to-language detection and must not open a second Problems model — its diagnostics reach the view through the shape `lsp_core::DiagnosticStore` already uses.
- `dap-core` owns the protocol, the session state machine, the adapter catalog and the breakpoint store. It must not own an editor buffer; line shifting is driven from the existing buffer-edit seam in `ui-shell`.

### QObjects

`BuildServiceRust` and `DebugServiceRust` each follow the ADR-0032 precedent: one registered `#[qobject]` type owning a `HashMap<u64, Session>`, not one QObject per session, because cxx-qt registers a type's `QMetaObject` once at build time.

## 3. ADRs to write

**ADR-0039: typed run configurations, macros and before-launch tasks.**
Amends ADR-0032.
Records why the toolchain table lives in `run-core` rather than in a new crate, why `RunConfigKind` defaults to `Custom` so existing `.ide/settings.toml` files keep loading, and why macro expansion covers args as well as cwd and env.

**ADR-0040: `build-core` — delegate to the build tool and parse its diagnostics.**
Records why there is no IDE-owned output-folder, artifact or auto-build model, why Cargo is parsed from JSON while the other toolchains are parsed from text, and why build diagnostics join the existing Problems dock instead of getting a second panel.

**ADR-0041: `dap-core` — a DAP client, its adapter catalog, and who owns breakpoints.**
Records DAP over gdb/lldb MI, the `lsp-core`-shaped structure, the `syntax-core` dependency on the ADR-0035 precedent, and why the breakpoint store is Qt-free and shifted through the existing buffer-edit seam rather than through a new editor hook.

## 4. Risks

| # | Risk | Mitigation |
|---|---|---|
| 1 | Debug adapters are external binaries the user may not have. | The catalog reports a missing adapter as a typed error with the install hint, exactly as `lsp-core`'s server catalog already does for language servers. |
| 2 | Four toolchains multiply the test matrix. | Only Cargo is exercised by an E2E flow and by CI; the other three are a manual matrix recorded in D4-6, mirroring how the LSP conformance suite is kept out of the per-PR gate. |
| 3 | Text diagnostic parsing is brittle across tool versions. | Cargo, the toolchain we dogfood, is parsed from JSON. The text parsers are table-driven with fixture files, so a broken format is a fixture change, not a rewrite. |
| 4 | The E2E budget is full on arrival. | Exactly three flows are added, taking the budget from 12 to 15, its stated ceiling. Any further flow means deleting one. |
| 5 | `dap-core` could drift into a second `lsp-core`. | Framing is the same wire shape; if the second implementation is byte-identical in behaviour it is extracted into a shared module rather than duplicated, and that extraction is the decision ADR-0041 must state either way. |

## 5. Verification

Per task, before every commit:

```sh
make lint
make test
cargo tree -p build-core -e normal | grep -i qt      # must be empty
cargo tree -p build-core -e normal | grep -i tokio   # must be empty
cargo tree -p dap-core   -e normal | grep -i qt      # must be empty
cargo tree -p dap-core   -e normal | grep -i tokio   # must be empty
```

End to end, per phase, in the headless harness under Xvfb:

- R1 — click the gutter run icon on a `fn main`, see a temporary configuration appear in the toolbar combo and its output in the console.
- B1 — introduce a deliberate compile error, press Build, confirm the Problems dock lists it at the right file and line and that double-clicking navigates there.
- B2 — attach a Build before-launch task to a run configuration, break the build, confirm the run never starts and the failure is visible.
- D3 — set a breakpoint in a Rust binary, press Debug, confirm the session suspends on that line, that Variables shows locals, that step over advances one line, and that resume runs to exit.
- D4 — repeat the D3 walk once per toolchain and record the result in this document.
