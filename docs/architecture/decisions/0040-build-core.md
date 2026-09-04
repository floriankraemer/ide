# 0040. `build-core`: delegate to the project's build tool, and parse what it says

## Status

Accepted

## Context

The IDE has no build story at all: nothing invokes `cargo`, `cmake`, `mvn` or `gradle`, nothing reads compiler output, and the Problems dock is fed only by `lsp-core`'s diagnostics.
IntelliJ's compiling-applications page describes two quite different things — driving a build, and *owning a model of compilation* (per-module output paths, artifacts, build-automatically-on-save, its own incremental compiler).

Only the first is worth having here.
A model of compilation is a second opinion about something the build tool already decides, and the two drift: an output path configured in the IDE and a different one in `build.gradle` is a support question, not a feature.
It is also almost entirely a JVM concern; Cargo, CMake and Python have no equivalent to configure.

## Decision

### 1. A new Qt-free crate, `build-core`, that delegates

`build-core` turns a build request into the steps that satisfy it and turns what those steps print back into diagnostics.
It owns no notion of a compiler, an output folder, a module path or an artifact.

Its dependency row is `run-core` (+ serde_json, regex) and nothing else.
`run-core` because both halves are already there: the toolchain table (ADR-0039) says what a build invocation looks like, and `LaunchSpec` says how a process is started. A second detection table or a second launch shape would be exactly what `docs/architecture/layering.md` forbids.

**Not** in scope, stated once: compiler output folders, artifact/packaging configuration, build-automatically-on-save, and an IDE-owned incremental build.
Auto-build is the one that looks tempting and is not: LSP already reports errors as the user types, so an auto-build would duplicate a signal the user already has, at the cost of a compiler running on every save.

### 2. Rebuild is the tool's own clean, then its build

`BuildKind::{Build, Rebuild, Target}`. `Rebuild` emits two steps rather than looking for a per-tool "force" flag, and a toolchain with no clean step of its own refuses the request instead of silently doing half of it.
A target is spelled per toolchain (`cargo build -p`, `cmake --target`, `gradle :module:build`); a toolchain with no spelling for one builds everything rather than being handed a flag it would reject.

### 3. Cargo is parsed from JSON; everything else from a small pattern table

Cargo builds get `--message-format=json`, and their diagnostics are read from the structured spans — exact file, line, column, level and code, with no chance that a message *containing* something shaped like `foo.rs:12:3` is mistaken for a second diagnostic.
It is the toolchain we dogfood, so it is the one worth being exact about.

Every other toolchain prints for a human, so `build_core::text` recovers diagnostics with three patterns: `path:line:col: severity: message` (gcc, clang, CMake, modern javac), `path:line: severity: message` (classic javac), and `[SEVERITY] /path/File.java:[line,col] message` (Maven and Gradle).
The table is data, so a tool that changes its format is a fixture and a row rather than a rewrite.

Those shapes overlap `run_core::links`' catalogue, which makes `file:line` clickable in a run console, and they are deliberately *not* shared: a link resolver needs the location and nothing else, while this needs the severity and the message too, and a shared table would have to carry both consumers' needs to serve either.

A severity word we do not recognise becomes a note, never an error: a red row in the Problems dock for something the build was happy with is worse than an uncategorised one.

### 4. Build diagnostics join the existing Problems dock

`BuildDiagnostic` is deliberately the shape the Problems dock already renders for `lsp_core::DiagnosticStore`.
A compiler error delivered by a build is not a different kind of thing from the same error delivered over LSP, and a second panel would make a user look in two places for one answer.

### 5. Output is read as a stream, and a build runs over a PTY

`DiagnosticParser` is fed chunks and holds back the partial trailing line until its newline arrives, so a diagnostic split across two reads is reported exactly once, and `finish()` flushes a last line that never got one.
The Problems dock therefore fills while the build is still running.

Builds run through `run_core::Supervisor`, which is PTY-backed. A build would rather have pipes — that is what `ConsoleKind::Pipes` was reserved for — but the PTY transport is the only one in the repo and it is the one that can kill a build's *whole process tree*, which matters precisely because `cargo` spawns `rustc` and `gradle` spawns a daemon.
The cost is that tools colour their output; the adapter already strips ANSI out of console text before caching it (ADR-0032), and the parsers are fed that stripped text.
`ConsoleKind::Pipes` stays reserved for `dap-core`, whose adapter really does speak a protocol over stdio.

### 6. Error codes 200-299

ADR-0003 §4 left 200-599 as headroom. `BuildError` takes 200-299, with the usual per-crate test asserting its codes are unique and inside the range.

## Consequences

- Whatever the build tool decides is what the IDE reports. There is no second place to configure an output path, and no way for the two to disagree.
- A toolchain gains a build by gaining rows in `run_core::toolchain`, not by gaining code here.
- The Problems dock now has two sources. It has to say which, or a stale build diagnostic and a live LSP one look identical — that is the view's job, and B1-7's.
- Parsing text is inherently version-sensitive for four of the five toolchains. The one we dogfood is not text-parsed, and the rest are covered by fixtures, so a format change is a visible test failure rather than a silent loss of diagnostics.
- Killing a build kills its process tree, including a Gradle daemon it started. That is the honest reading of "stop the build" and is what the PTY transport gives.

## Alternatives rejected

**Owning compilation output folders and artifacts** (IntelliJ's model). Rejected: a second opinion about something the build tool already decides, meaningful almost only on the JVM, and a drift bug generator. Revisit only if a toolchain appears that cannot report its own layout.

**Build automatically on save.** Rejected: LSP already reports errors while typing; an auto-build spends a compiler on a signal the user has.

**A second Problems panel for build diagnostics.** Rejected: one question, two places to look.

**Sharing one regex table with `run_core::links`.** Rejected: the two consumers want different fields out of the same text, and a shared table would have to satisfy both to serve either.

**Parsing every toolchain structurally** (`javac -Xdiags`, Gradle's tooling API, `cmake --output-format`). Deferred: each is a per-tool integration, and the pattern table covers what the Problems dock needs today. Revisit per toolchain when its text output proves too lossy.

**A `runner` of its own in `build-core`, spawning `std::process::Command` with pipes.** Rejected: it would be a second process-launching path in a repo that already has one, and it could not kill a build's process tree, which is the difference between "stop" working and appearing to.

## Related

- [ADR-0039: typed run configurations](0039-typed-run-configurations.md) — the toolchain table this crate reads.
- [ADR-0032: run configurations](0032-run-configurations.md) — `LaunchSpec`, the supervisor, and the ANSI stripping this reuses.
- [ADR-0003: FFI conventions](0003-ffi-conventions.md) — §4's error-code ranges; 200-299 is claimed here out of its headroom.
- `docs/architecture/run-build-debug-parity-plan.md` — the roadmap this is phase B of.
- `crates/build-core/src/` — the code this ADR documents.
