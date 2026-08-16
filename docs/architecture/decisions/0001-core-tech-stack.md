# 0001. Core tech stack: Rust core + Qt6 UI via cxx-qt, hybrid plugin system

## Status

Accepted

## Context

New IDE, PHPStorm-like layout, with these hard requirements:

- Performance is the top priority, not development speed or cost.
- Cross-platform: Windows, macOS, Linux.
- Fully open source.
- Modular architecture.
- Third-party plugin system.
- Native look and feel on each target OS.

These forces are partly in tension. A pure C++/Qt6 build gets native widget
rendering and a mature plugin-loading model (QPluginLoader) fastest, but
C++ gives up memory safety for the perf-critical editor/indexing core.
A fully custom GPU-rendered UI (the Zed/GPUI approach) can beat both on raw
rendering throughput, but forfeits native OS widgets (menus, file pickers,
dialogs) and requires building an entire UI toolkit from scratch — a much
larger and riskier undertaking for an open-source project without a
dedicated systems-UI team. A GC'd stack (C#/Avalonia) trades peak
performance and memory-safety-without-GC-pauses for faster iteration, which
does not match the "performance must be KING" requirement. A web/Electron
stack was ruled out immediately — it cannot meet the performance bar.

## Decision

Build the IDE with:

- **Editor/engine core in Rust** — text buffer/rope, syntax highlighting,
  indexing, LSP client, debugger adapter layer, project/VCS model. Rust
  gives near-C++ performance with memory safety and a strong concurrency
  story, both valuable in a long-running desktop process handling large
  codebases.
- **UI layer in Qt6**, bridged to the Rust core via `cxx-qt`. Qt6 provides
  native-feeling widgets, menus, dialogs, and file pickers on Windows,
  macOS, and Linux, plus mature theming (QSS) and QML for richer panels.
  `cxx-qt` lets the Rust core drive Qt models/signals directly instead of
  going through a second IPC layer.
- **Hybrid plugin system**:
  - Native dynamic libraries with a stable ABI for core/perf-critical
    integrations shipped or tightly coupled to the IDE itself (LSP clients,
    debugger adapters, syntax/indexing engines) — these need full-speed,
    zero-copy access to the editor core.
  - WASM (via `wasmtime`) for third-party plugins — sandboxed, crash-isolated,
    and open to any language that compiles to WASM, at the cost of a
    hand-maintained host-API surface and a small perf overhead on the
    host/plugin boundary.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| Qt6 + C++ only | Fastest path to native polish and a mature plugin model, but the perf-critical core loses Rust's memory safety and modern tooling; C++ was judged worse long-term for a project prioritizing robustness and maintainability over dev speed. |
| Rust + custom GPU UI (Zed/GPUI-style) | Highest theoretical raw rendering performance, but no native widgets — every menu, dialog, and file picker must be built from scratch, and the ecosystem for IDE-scale GPU UI in Rust is still immature. Too large an upfront lift for this stage. |
| C# + Avalonia | Fastest development velocity and a mature plugin ecosystem (.NET/MEF), but GC'd runtime performance ceiling and pause behavior conflict with the "performance must be KING" requirement. |
| Electron / web-based UI | Rejected outright — cannot meet the performance requirement for a PHPStorm-class IDE (large-project indexing, editor responsiveness). |
| WASM-only plugin system | Sandboxing and portability are attractive, but core integrations (LSP, debuggers) need low-latency, high-frequency calls into the editor core that a WASM boundary would tax; native ABI was kept for that tier. |
| Native-only plugin system | Maximum performance for all plugins, but no crash isolation or sandboxing for third-party code — unacceptable for an open plugin ecosystem where plugin quality varies. |

## Consequences

- Positive: memory-safe, high-performance core; native OS look and feel via
  real Qt widgets; clean separation between trusted perf-critical
  integrations (native) and untrusted third-party plugins (sandboxed WASM);
  fully open-source toolchain (Rust, Qt6 LGPL, wasmtime).
- Negative / accepted trade-offs: `cxx-qt` is a smaller, less battle-tested
  project than either raw Qt/C++ or a pure-Rust UI stack, so the Rust↔Qt FFI
  boundary carries more integration risk and a smaller community to draw on.
  The team must maintain expertise in both Rust and Qt/QML. The WASM
  host-API surface (what capabilities are exposed to third-party plugins)
  must be designed and versioned by hand, and calls across that boundary
  carry a small but nonzero perf overhead versus native plugins.

## Related

- [Architecture overview](../overview.md)
- Follow-up ADRs not yet written: text buffer/rope data structure choice,
  LSP client library choice, build/packaging tooling (Cargo workspace +
  CMake for the Qt side), CI matrix for three target OSes.
