# Architecture overview

Arc42-lite overview of the current system.
The binding rules live in [layering.md](layering.md) and the ADRs; this page orients a new contributor.

## 1. Context

The project is a cross-platform IDE with a JetBrains-like layout.
It is a Rust Cargo workspace with a Qt6 Widgets UI, bridged via `cxx-qt` per [ADR-0001](decisions/0001-core-tech-stack.md).

It has grown past the original MVP (open a folder, browse a tree, edit and save tabs — see the [MVP proposal](../product/mvp-proposal.md)).
Shipped since: a settings window and keymap, ADS-based docking, theming, tree-sitter syntax highlighting and folding, a Class View outline, an embedded terminal, a project-wide text and symbol index, find and replace, and code navigation (Go to Declaration, Find Usages, Go to Implementation, jump history).

## 2. Quality goals

1. **Testability without a display.**
   All business logic lives in Qt-free crates and runs under plain `cargo test`, with no Qt runtime or display server.
2. **View swappability.**
   The view is Qt Widgets today; QML is the planned future view.
   Because the view holds zero rules ([ADR-0002](decisions/0002-application-layer-and-humble-view.md)), swapping it must not touch `app-core` or the domain crates.
3. **Performance** (from ADR-0001): typing latency and large-file handling drive the Rust-core decision.

## 3. Building-block view

Four layers plus the binary crate, per [layering.md](layering.md).
Only `ui-shell` and `app` touch Qt; every other crate is Qt-free and unit-tested with no display.

```mermaid
graph TB
    app["app (main)"] --> view
    subgraph uishell["ui-shell"]
        view["view: cpp/*.cpp<br/>widgets, layout, wiring"] --> adapter["adapter: src/bridge.rs<br/>thin QObject translation"]
    end
    adapter --> appcore["application: app-core<br/>AppSession, commands, AppError"]
    adapter --> support["support: app-config, syntax-core,<br/>index-core, terminal-core, mcp-server"]
    appcore --> editorcore["domain: editor-core"]
    appcore --> projectmodel["domain: project-model"]
    support --> editorcore
    support --> projectmodel
```

| Crate | Layer | Responsibility | Qt |
|-------|-------|----------------|----|
| `editor-core` | domain | Rope-backed `Document`, tab list, load/save/dirty state, find/replace matching | No |
| `project-model` | domain | `ProjectSession`, directory tree, `notify` watcher, last-project persistence | No |
| `app-core` | application | `AppSession`: orchestration, command methods, typed `AppError`, jump history | No |
| `app-config` | support | `settings.toml` load/save, theme, editor font/colors, keymap | No |
| `syntax-core` | support | tree-sitter parsing: highlighting, folding, outline, occurrences, supertype edges | No |
| `index-core` | support | Project index: text search (tantivy + ripgrep crates) and symbols/references, plus declaration resolution ([ADR-0011](decisions/0011-code-navigation.md)) | No |
| `pty-core` | support | Cross-platform PTY transport for the embedded terminal | No |
| `terminal-core` | support | VT100/grid state over `alacritty_terminal` | No |
| `mcp-server` | support | MCP transport so an agent can read and drive the editor | No |
| `ui-shell` | adapter + view | `src/bridge.rs`: cxx-qt QObject translation; `cpp/`: Widgets, layout, menus, dialogs, `QApplication` | Yes |
| `app` | main | Thin binary; hands off to `ui-shell` | Yes |

The view never decides, it only displays and forwards intent.
Rules crossing the FFI seam (typed errors, `TabId`, Rust-owned dirty state) are fixed by [ADR-0003](decisions/0003-ffi-conventions.md).

## 4. Cross-boundary communication

- UI actions call invokable slots on the `cxx-qt` QObjects; the adapter translates and delegates to `AppSession`.
- Rust-side changes (dirty flags, watcher events) surface as Qt signals; tree data is exposed via a `QAbstractItemModel` backed by Rust data.
- Filesystem watcher events arrive on a background thread and are marshalled onto the Qt event loop via `CxxQtThread` queuing — no shared-mutex model.
- Long-running work (index build, search, symbol resolution, PTY reads) runs on a plain `std::thread` and streams results back through the same `CxxQtThread::queue()` hop, so the UI thread never blocks on it.
- The filesystem watcher also drives incremental re-indexing, which is what keeps navigation targets from drifting after an edit.

## 5. Build and deployment

`docker/Dockerfile` is a single multi-stage file: a `linux-builder` stage (apt Qt6) and a `windows-builder` stage cross-compiling with MXE's mingw-w64 + Qt6 toolchain.
Artifacts land in `dist/`.

## 6. Future scope (not implemented)

The following are documented direction per ADR-0001 but have no code and no crates today; add them when the work starts, not before:

- **LSP client** — language-server integration in the Rust core.
  This is also the upgrade path past ADR-0011's deliberately name-based declaration resolution.
- **Debugger adapter (DAP)** — same placement rationale as LSP.
- **Plugin host** — hybrid model: native dylib loader (stable C ABI) for trusted, perf-critical integrations; sandboxed WASM runtime (wasmtime) with a narrower, capability-based API for third-party plugins.
- **QML view** — the planned replacement for the Widgets view; the humble-view split exists so this swap stays cheap.

Each of these gets its own ADR under `decisions/` when it becomes real.

## Related

- [Layering rules](layering.md) — binding dependency and logic-placement rules.
- [ADR-0001](decisions/0001-core-tech-stack.md), [ADR-0002](decisions/0002-application-layer-and-humble-view.md), [ADR-0003](decisions/0003-ffi-conventions.md) — the binding stack, layering and FFI decisions.
- The remaining ADRs under `decisions/` cover MCP transport, docking, the terminal, the project index, find and replace, and code navigation.
- [MVP implementation plan](mvp-implementation-plan.md) — historical.
