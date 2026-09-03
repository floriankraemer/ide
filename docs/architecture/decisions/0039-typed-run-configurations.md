# 0039. Typed run configurations: one toolchain table in `run-core`, macros everywhere, run from context

## Status

Accepted

Amends ADR-0032 (run configurations: a PTY-backed console, ANSI-stripped for v1, one supervisor QObject for N sessions).

## Context

ADR-0032 shipped a run configuration as a name, a program, arguments, a working directory and an environment, detected from `Cargo.toml`, `package.json` and `Makefile`.
That is enough to launch something and nothing more: a configuration does not know what kind of thing it is, so nothing downstream can ask.

Three consumers now need that answer at once.
The IntelliJ parity work (`docs/architecture/run-build-debug-parity-plan.md`) adds a build step, which has to know whether to run `cargo build` or `./gradlew build`; a debugger, which has to know whether to start codelldb or debugpy; and running from context, which has to know that `src/bin/tool.rs` means `cargo run --bin tool`.
Each of those could detect the project's build tool for itself. Doing so would put three tables of marker files in three crates, which is the duplication `docs/architecture/layering.md` already forbids for file-to-language detection (ADR-0018) and for icon art.

## Decision

### 1. One toolchain table, in `run-core`

`run_core::toolchain` owns `ToolchainId` (Cargo, CMake, Python, Maven, Gradle, npm, Make) and, per toolchain: the marker files that mean it is in use, the build and clean invocations, the JS package manager or JVM wrapper to prefer, and the debug adapter it implies.
`run_core::detect` is written on top of it, and `build-core` and `dap-core` will read it rather than detecting again.

It lives in `run-core` rather than in a new `toolchain-core` crate because a table with three consumers and no rules of its own does not earn a crate, and because `run-core` is already the crate that turns a project into something launchable.
`build-core` and `dap-core` depending on `run-core` is the same shape `run-core` already has with `pty-core` and `terminal-core`: a support crate reusing a support crate, rather than a join that has to be lifted into `app-core`.

Detection stays marker-file presence only — no invoking `cargo metadata`, `cmake` or `mvn` — exactly the promise `detect` already made about target names.
The cost is a Maven or Gradle run task that may not work until the project applies the right plugin. An unusable default a user edits beats no entry to edit, and neither tool can be asked without running it.

### 2. The persisted shape gains strings, not an enum

A configuration persists `toolchain` and `target` as optional strings, plus `temporary` and `allow_parallel` as defaulted bools.
The plan predicted a `RunConfigKind` enum on `RunConfig`. `RunConfig` *is* `app_config::RunConfigSetting`, and `app-config` depends on nothing — an enum owned by `run-core` on a struct owned by `app-config` would have inverted that row.
So persistence stays dumb, `run-core` maps the string back through `ToolchainId::from_id`, and an identifier a newer version writes loads as "no toolchain" instead of failing the whole settings file.

Every field is `#[serde(default)]` and skipped when empty, so an existing `.ide/settings.toml` round-trips byte-for-byte.

### 3. Macros expand in arguments too, and an unresolvable token stays visible

`run_core::macros` replaces the single `$PROJECT_DIR` replacement with `$PROJECT_DIR$`, `$FILE_PATH$`, `$FILE_DIR$`, `$FILE_NAME$`, `$FILE_NAME_WITHOUT_EXTENSION$` and `$USER_HOME$`, expanded in `cwd`, in environment values **and** in arguments.
ADR-0032's note that arguments were deliberately left alone was a scope decision, not a principle: a run-from-context configuration whose argument is the file it was started from cannot exist without it.

Both spellings are accepted — the IntelliJ `$TOKEN$` form and the bare `$TOKEN` that F4 shipped — because the bare form is what saved configurations contain, and dropping it would silently change what they run.

A token the context cannot resolve, such as `$FILE_PATH$` in a configuration launched from the toolbar, is left exactly as written rather than replaced with an empty string.
A visibly unexpanded token in the console's command line is a legible failure; a command that looks right and runs against `/` is not.

### 4. Running from context creates a temporary configuration, capped at five

`run_core::context::config_for_file` answers what running a file would launch, for the cases where the mapping is unambiguous: `src/main.rs`, `src/bin/*`, `examples/*` (both the flat and the `main.rs` layout), and a Python file in a Python project.
A CMake source belongs to whichever `add_executable` lists it and a JVM class needs the module's classpath, so both return nothing rather than a guess that runs the wrong binary.

The result is marked `temporary` and its id is derived from the target, so clicking the same gutter icon twice reuses one entry.
`remember_temporary` keeps at most five, evicting oldest-first and never touching a saved configuration — which is what makes "save this temporary configuration" mean "stop it being thrown away". Editing one in the run-configuration dialog clears the flag, IntelliJ's own behaviour.

### 5. The parallel-run policy is enforced in the adapter, not in the supervisor

`Supervisor` already carries N consoles; nothing about it needed to change.
`RunService::launch` stops a configuration's still-running consoles before starting a new one unless `allow_parallel` is set — IntelliJ's default, and the reason a second Run does not leave two servers holding the same port.

That check is in `bridge/run/mod.rs` rather than in `run-core` because it is about consoles the adapter owns, not about launching, and it is Rust rather than C++, so the humble-view rule is intact.

### 6. The gutter Run icon sits on the first line

The gutter grows a Run-icon column, present only for a file `RunService::canRunFile` says has a run target, so a file without one keeps exactly the gutter it had.
The icon is drawn on the first line rather than beside the entry point's own declaration: naming that line needs the symbol index, and "run this file" is what the click means either way.
`run.runContext` (Ctrl+Shift+F10) is the same call from the keyboard.

## Consequences

- A fourth consumer of "which build tool is this" extends `run_core::toolchain` or it is a bug. There is no second marker table anywhere, and `build-core` and `dap-core` are written against this one from their first commit.
- `run-core`'s dependency row is unchanged: the table is data and `std`, and needs nothing new.
- A configuration's identity (`id`, `toolchain`, `target`) is read-only in the dialog. `updateConfiguration` takes the whole `FfiRunConfig` rather than a field per argument, so growing the form does not grow a positional parameter list.
- Detection now emits configurations for four more toolchains, which means a polyglot project's list is longer than it was. The merge rule is unchanged, so nothing a user edited is overwritten.
- Recent-first ordering in the toolbar's configuration picker is deferred: the settings file records no recency to sort by, and inventing a timestamp field to sort a combo box is not worth a schema change yet.

## Alternatives rejected

**A `toolchain-core` crate.** Rejected: a table with no rules of its own and three readers does not earn a crate, and every reader would still have to depend on `run-core` for `LaunchSpec`.

**Detecting by invoking the build tool** (`cargo metadata`, `mvn help:evaluate`). Rejected for detection: it makes opening a project run arbitrary project code, and it is slow enough to need its own thread and progress reporting for an answer a marker file gives immediately. Revisit only if a target list a marker file cannot produce becomes a stated requirement.

**A `RunConfigKind` enum on the persisted struct.** Rejected: it inverts the `app-config`-depends-on-nothing row (see decision 2).

**Expanding an unresolvable macro to an empty string.** Rejected: it turns a misconfigured launch into a plausible-looking one.

**Per-toolchain subclasses of the run configuration** (IntelliJ's own configuration *types*, each with its own form). Deferred, not rejected: the current form plus a toolchain tag covers every configuration this IDE can produce today. Revisit when a toolchain needs a field the generic form has no place for — a JVM main class, say.

## Related

- [ADR-0032: run configurations](0032-run-configurations.md) — the ADR this amends; `LaunchSpec` as the debugger-agnostic seam is unchanged and is what `build-core` and `dap-core` will reuse.
- [ADR-0018: single-source language detection](0018-single-source-language-detection.md) — the rule this decision applies to build tools rather than to languages.
- [ADR-0022: per-project settings](0022-per-project-settings.md) — where a run configuration is persisted, and why the new fields are project-scoped.
- `docs/architecture/run-build-debug-parity-plan.md` — the roadmap this is the first phase of.
- `crates/run-core/src/toolchain.rs`, `crates/run-core/src/macros.rs`, `crates/run-core/src/context.rs` — the code this ADR documents.
