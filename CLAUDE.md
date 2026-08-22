# IDE — Agent Rules

Cross-platform IDE: Rust core + Qt6 Widgets via cxx-qt (QML planned later).
Architecture authority: `docs/architecture/layering.md` and the ADRs in `docs/architecture/decisions/`. Read them before structural work.

## Development environment

Always use Docker containers for development (builds, tests, running the app) — never the bare host.

```sh
make linux-image     # build/refresh the builder image
make test            # cargo test --workspace inside it
make lint            # clippy -D warnings + rustfmt --check
```

Go through the Makefile rather than a hand-written `docker run`: its `RUN_LINUX` mounts named volumes for the crate registry and the ccache object store, and a bare `docker run --rm` throws both away — re-downloading 390-odd crates and recompiling every C++ translation unit each time.

Debug builds carry line tables only, so backtraces keep file and line but a debugger sees no variable or type information.
When you need to step through something — usually the cxx-qt seam — build with `cargo build --profile debugging -p app`, which is `dev` plus full DWARF.

`linux-builder` has the full Qt6 dev toolchain (`docker/Dockerfile`); the workspace is mounted rather than baked in, so edits are picked up without an image rebuild. For anything the Makefile has no target for, reuse `RUN_LINUX`'s mounts, e.g. `make shell` then `cargo build --release -p app`.

## Starting a session

Before starting work, read the Progress table of the newest plan doc in `docs/architecture/` (currently `refactoring-plan.md`) and `git log` to find the next open task. Update the task's row (status + commit hash) in the same commit that finishes it — status and code must never drift apart.

Check `git status`/`git log` before starting; if another session's work is uncommitted or mid-flight, don't overwrite it — coordinate or work on a different task instead.

## Project map

- `crates/editor-core` — domain: rope `Document`, `TabList`, binary detection. Qt-free.
- `crates/project-model` — domain: `ProjectSession`, directory tree, filesystem watcher. Qt-free.
- `crates/app-core` — application layer: `AppSession`, commands, `TabId`, `AppError`. Qt-free.
- `crates/ui-shell` — adapter (`src/bridge.rs` cxx-qt QObjects) + view (`cpp/main_window.cpp` Qt Widgets).
- `crates/app` — main entry point.

## Hard layering rules

| Crate | May depend on |
|---|---|
| editor-core | (std + small utility crates only) |
| project-model | (std + notify, dirs) |
| app-core | editor-core, project-model |
| ui-shell | app-core, editor-core, project-model, cxx-qt |
| app | ui-shell |

- `editor-core`, `project-model`, `app-core` MUST NOT depend on cxx-qt/Qt in any form.
- Business rules and orchestration live ONLY in Qt-free crates.
- `bridge.rs` QObjects: translation only — slot → `AppSession` call → signal/model refresh. No rules, no owned domain state.
- `cpp/` is a humble view: widget construction, layout, dialogs, signal wiring only. Never add an `if` that encodes a business decision to C++.

## FFI seam rules (ADR-0003)

- Errors cross the bridge as typed code + message; never a `QString` sentinel ("" = success is banned).
- Tabs are identified by `TabId`; widget indices are mapped only at the model/adapter edge.
- Rust `Document` owns dirty state; the view forwards edits and reads flags.

## Testing

- Every new rule or behavior gets unit tests in the Qt-free crate it lives in.
- C++ stays thin and is untested by design — if you feel you need a C++ test, the logic is in the wrong layer.
- Gate: `cargo test --workspace` must pass before commit.

## Docs

- Structural change (new crate, moved responsibility, new dependency) ⇒ update `docs/architecture/layering.md` and add/update an ADR (numbering continues from the last).
- Keep `docs/architecture/overview.md` truthful to the code; fix drift when you see it.
- Long markdown files: one full sentence per line.

## Verification

```sh
cargo test --workspace
cargo tree -p editor-core  -e normal | grep -i qt   # must be empty
cargo tree -p project-model -e normal | grep -i qt  # must be empty
cargo tree -p app-core     -e normal | grep -i qt   # must be empty
```
