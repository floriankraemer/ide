# 0025. The seam split and a file-size ceiling

## Status

Accepted

## Context

Two files had grown past the point where anyone could review a change to them.

`crates/ui-shell/src/bridge.rs` held one `#[cxx_qt::bridge]` module and twelve QObjects in 9,346 lines.
`crates/ui-shell/cpp/main_window.cpp` held the tab machinery, three panels, the main window, two controllers, the settings dialog and the dock layout in 4,263 lines.

Everything lands in those two files.
Adding a feature to this codebase means a QObject in the first and a dock, a menu and a settings page in the second, and the next four planned features would have added several thousand more lines to each.
`showSettingsDialog` already took fourteen parameters.

Splitting them later is strictly worse than splitting them now: every feature branch in flight touches both files, so the merge cost grows with every week of delay.

## Decision

### 1. One bridge module, moved; every implementation out

cxx-qt permits exactly one `#[cxx_qt::bridge]` module per crate, and shared FFI structs are per-bridge — two bridges declaring `FfiResult` would produce two distinct C++ types.
So the module survives intact and moves to `src/bridge/ffi.rs`, containing **only declarations**.

Every `…Rust` state struct and every `impl` block moves to a per-feature module beside it: `tree`, `editor`, `settings`, `search`, `terminal`, `language`, `ai`, plus `registry` for the process-wide shared handles and `convert` for the cross-cutting helpers.

### 2. `main_window.cpp` splits by class, one translation unit each

There is no `Q_OBJECT` anywhere in that file — deliberately, with comments explaining that it uses `std::function` callbacks precisely to avoid a second moc target.
So each extracted unit costs one `.cpp_file(...)` line in `build.rs` and no header registration.

**Amended by F0-4a**: one translation unit per class turned out not to hold for `EditorTabs`.
The class plus the four cursor/highlighter helpers only it used is ~1,790 lines, which no single `.cpp` may be under section 3's ceiling, and granting it a baseline would have added an entry to the very list this ADR exists to empty.
It is therefore declared once in `cpp/editor_tabs.h` and defined across three sources — `editor_tabs.cpp` (the tab surface), `editor_tabs_panes.cpp` (the `QSplitter` tree of tab groups and its save/restore) and `editor_tabs_lsp.cpp` (the language-server leg).
Defining members of one class across several translation units is ordinary C++; the rule this ADR actually needs is *one class per header*, with as many sources behind it as the ceiling requires.

### 3. A ceiling, enforced as a gate

1500 lines per Rust module, 1200 per C++ translation unit, checked by `scripts/check-file-size.sh` in `make lint` and in CI.

Files already over the ceiling are **grandfathered with ratcheted baselines**: each may shrink, never grow.
A gate that is red on arrival is a gate that gets switched off within a week, so nothing fails on day one — but nothing can get worse either.

`bridge/ffi.rs` is **exempt outright** rather than baselined.
It is ~3,000 lines and cannot be split, because cxx-qt allows only one bridge module.
A cap it violates the day it is created is not a cap.

### 4. The split is proven mechanical, not asserted to be

Two checks, both decisive.

**The generated FFI headers must be byte-identical.** `#[cxx_qt::bridge]` generates C++ headers; snapshot them before, snapshot after, diff.
An empty diff proves every QObject, slot signature, signal and type mapping is unchanged — a stronger guarantee than any test, because it is about the interface rather than a sample of behaviour.
This is what makes a 9,000-line refactor reviewable at all.

**The E2E marker stream must match, including event order.** `main_window.cpp` has no compile-time invariant to lean on. A reordered `tab_added`/`project_opened` pair is exactly the `connect()`-ordering change a mechanical-looking C++ split introduces, and nothing else would catch it.

This is the concrete reason the E2E harness (ADR-0024) is sequenced *before* the C++ split rather than after.

## Consequences

- Feature work lands in a module named after the feature, and two files stop being a permanent merge conflict.
- The size gate makes the ceiling a fact rather than a convention. Conventions that are not gates rot; this one is nine lines of shell.
- Baselines must be measured against the tip of `main` at the moment a change **merges**, not when its branch was cut. The first version of the gate was measured against a main that two open pull requests then landed on top of, and it turned main red the moment it merged. The numbers were correct when written and stale by the time they were enforced.
- `ffi.rs` still grows by roughly 120 lines per new QObject, and it is exempt. If it ever becomes unreviewable on its own, the multi-bridge option below gets revisited with evidence rather than in the abstract.

## Alternatives rejected

**One `#[cxx_qt::bridge]` module per feature.** Shared FFI structs are per-bridge in cxx-qt, so `FfiResult` declared in two bridges becomes two distinct C++ types. It is possible, but it is an FFI-shape change disguised as a mechanical refactor, and it would have made the header-diff proof impossible.

**Leave it and split when it hurts.** It already hurt: fourteen parameters on one function, and four features queued to land in the same two files. Splitting after they land is a merge conflict with every branch in flight.

**A ceiling as a convention in `CLAUDE.md`.** Conventions that nothing enforces rot. The gate costs nine lines.

**Splitting `bridge.rs` by QObject size rather than by feature.** Tempting, because it balances the files. Rejected: a reader looks for the AI chat's bridge code, not for the 900-line module. Cohesion beats balance.
