# 0002. Application layer (`app-core`) and humble Qt view

## Status

Accepted

## Context

The lower boundary of the workspace is clean: `editor-core` and `project-model` are Qt-free and hold nearly all tests.
The upper boundary is not.
Business rules currently live in the C++ view and in the QObject bodies:

- The binary-file open rule is implemented in the tree-view click handler (`crates/ui-shell/cpp/main_window.cpp:283-306`) — C++ decides whether a file may open, calling `isBinaryFile` and branching.
- Rename path construction is done in C++ (`main_window.cpp:377-383`), duplicating path logic that Rust (`project_model::rename_path`) already performs.
- Delete orchestration — call `deletePath`, then remember to call `notifyPathDeleted` — is a two-step protocol the C++ handler must get right (`main_window.cpp:399-404`); forgetting the second call silently breaks tab state.
- QObjects own domain state directly: `ProjectTreeModelRust` owns the `ProjectSession` (`crates/ui-shell/src/bridge.rs:401`) and `DocumentManagerRust` owns the `TabList` plus suppression map (`bridge.rs:667`), so none of this orchestration is unit-testable without Qt.

The forces in tension are testability and view-swappability (QML is the planned future UI, see `layering.md`) versus the ceremony of an extra layer in a ~2.2k LOC MVP.

## Decision

Introduce a Qt-free application crate `app-core` and make the Qt side a humble view.

- `app-core` provides `AppSession`, which owns the `ProjectSession` and an open-document table keyed by stable `TabId`, and exposes command methods (`open_file`, `save_tab`, `rename_entry`, `delete_entry`, …) returning `Result<T, AppError>`.
- All orchestration and rules move into `AppSession`: binary-open rule, rename path construction, delete → tab-invalidation, watcher-event → tab policy, config-dir fallback.
- The QObjects in `bridge.rs` become thin adapters holding a shared `AppSession`: slot → translate types → session call → emit signal / refresh model. No domain state, no rules.
- `main_window.cpp` becomes a passive humble view: widget construction, layout, menus, dialogs, and signal wiring only — zero business rules.

This is the Humble Object pattern at the cxx-qt seam, the same document/view split KTextEditor, Zed, and the JetBrains platform use: the view never decides, it only displays and forwards intent.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| Status quo (logic in QObjects + C++) | The four findings above are the cost, live today: rules untestable without a running Qt app, path logic duplicated across languages, multi-step protocols enforced only by convention. Every new feature grows the god-file and the untested surface. |
| Full MVVM framework (view-models, bindings, commands) | At 2.2k LOC this is machinery without a payer. cxx-qt's property/signal system already covers the binding role; a formal view-model layer would add a third representation of every piece of state to keep in sync. |
| Event/command bus | Indirection with no second subscriber in sight. A plain `AppSession` with `Result`-returning methods is the command layer; a bus makes control flow untraceable and buys nothing until multiple independent consumers exist (YAGNI). |

## Consequences

- Positive: every rule currently in C++ becomes a unit-tested Rust function; the view can be swapped (Widgets → QML) without touching logic; the two-step C++ protocols (rename/delete + tab notification) collapse into single `AppSession` commands.
- Positive: the FFI surface shrinks to translation, making the conventions in [ADR-0003](0003-ffi-conventions.md) enforceable.
- Negative / accepted trade-offs: one more crate and one more hop for every user action (view → adapter → session); some signals must be re-plumbed so the adapter emits them from session results rather than from local state. Accepted because the hop is mechanical and the alternative is untestable logic.

## Related

- [ADR-0001: core tech stack](0001-core-tech-stack.md)
- [ADR-0003: FFI seam conventions](0003-ffi-conventions.md)
- [Layering rules](../layering.md)
