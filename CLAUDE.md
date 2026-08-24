# IDE — Agent Rules

Cross-platform IDE: Rust core + Qt6 Widgets via cxx-qt (QML planned later).
Architecture authority: `docs/architecture/layering.md` and the ADRs in `docs/architecture/decisions/`. Read them before structural work.
Docs index: `docs/README.md` lists every architecture, decision, plan, design, and product doc with a one-line summary.
Orientation for a cold start: `docs/architecture/overview.md` (what the system is) and `docs/architecture/project-structure.md` (where things live).

## Default working mode

Load these skills before the work they cover — do not wait for a prompt to name the language:

- `clean-code-solid` — before any non-trivial implementation, refactor, or review.
- `rust` — before writing or reviewing Rust in any crate.
- `qt` and `cpp` — before touching `crates/ui-shell/cpp/` or the cxx-qt bridge.
- `conventional-commits` — before writing a commit message or PR title.
- `debugging-root-cause` — before any bug fix, alongside the E2E reproduction rule.
- `architecture-decision-records` — when a change requires a new or updated ADR.

Apply senior-software-engineer standards inline rather than delegating to the subagent:
small verified increments, `cargo test --workspace` green before every commit, patterns named only where they drive a decision, no abstraction for a hypothetical second implementation.

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

Before starting work, read the Progress table of the newest plan doc in `docs/architecture/` (see the plan list in `docs/README.md`) and `git log` to find the next open task. Update the task's row (status + commit hash) in the same commit that finishes it — status and code must never drift apart.

Check `git status`/`git log` before starting; if another session's work is uncommitted or mid-flight, don't overwrite it — coordinate or work on a different task instead.

## Project map

Domain (Qt-free):
- `crates/editor-core` — rope `Document`, `TabList`, load/save/dirty state, find/replace matching.
- `crates/project-model` — `ProjectSession`, directory tree, filesystem watcher, last-project persistence.

Application (Qt-free):
- `crates/app-core` — `AppSession`, commands, `TabId`, `AppError`.

Support (Qt-free):
- `crates/app-config` — settings, theme, fonts/colors, recents, window state (TOML).
- `crates/syntax-core` — tree-sitter platform: highlighting, language registry, runtime grammars, theme.
- `crates/index-core` — project index: text search, symbols, declaration resolution.
- `crates/lsp-core` — blocking LSP client: framing, `LspManager`, server catalog, feature modules.
- `crates/settings-model` — rules behind the language-platform settings pages.
- `crates/edit-ops` — language-aware editing: comment toggle, expand selection, indent, bracket pairing and matching.
- `crates/pty-core` — cross-platform PTY transport.
- `crates/terminal-core` — VT100 grid state over `alacritty_terminal`.
- `crates/mcp-server` — local Streamable-HTTP JSON-RPC MCP server over the shared index.

Adapter + view:
- `crates/ui-shell` — adapter (`src/bridge.rs` cxx-qt QObjects) + view (`cpp/` Qt Widgets).

Main:
- `crates/app` — main entry point.

## Hard layering rules

The authoritative per-crate import table lives in `docs/architecture/layering.md`; check it before adding any dependency.

- Every crate except `ui-shell` and `app` MUST NOT depend on cxx-qt/Qt in any form.
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
- New ADR, plan, design, or product doc ⇒ add its line to the index in `docs/README.md`.
- Keep `docs/architecture/overview.md` truthful to the code; fix drift when you see it.
- Long markdown files: one full sentence per line.

## Verification

```sh
cargo test --workspace
cargo tree -p editor-core  -e normal | grep -i qt   # must be empty
cargo tree -p project-model -e normal | grep -i qt  # must be empty
cargo tree -p app-core     -e normal | grep -i qt   # must be empty
```
