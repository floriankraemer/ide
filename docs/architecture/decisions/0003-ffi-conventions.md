# 0003. FFI seam conventions: typed errors, stable TabId, Rust-owned dirty state

## Status

Accepted.
Amended by [§4, error code ranges](#4-error-code-ranges-amendment), added when the seam had grown from one error type to seven.

## Context

The cxx-qt seam currently uses three ad-hoc conventions that scale badly:

- Errors cross FFI as `QString`, with `""` meaning success (`openFolder`, `saveTab`, `renamePath`, …) or as a `-1` return plus a separate `lastError()` call (`openFile`). The UI cannot branch on error kind, only display the string, and "empty means OK" is invisible at call sites.
- Tab identity is an `int` index kept in lockstep by convention across three structures: `QTabWidget` pages, the C++ `titles_` list, and Rust's `TabList` (`main_window.cpp:37-43`). Every close shifts every later index; correctness depends on both sides mutating in the same order.
- Dirty state lives twice: in each `QTextDocument` (`isModified`) and in the Rust `Document` flag, mirrored via `setTabModified`. Divergence is possible and neither side is authoritative.

## Decision

Three conventions govern everything crossing the cxx-qt seam:

1. **Typed errors.**
   `app-core` defines `AppError`; each variant maps to a stable `i32` code.
   FFI-crossing commands return a small result struct (code + user-facing display message) instead of a `QString` sentinel.
   Code `0` is success; the UI branches on code and displays the message verbatim.
2. **Stable `TabId(u64)` identity.**
   `app-core` issues a `TabId(u64)` per open document, never reused within a session.
   All FFI calls and signals identify tabs by `TabId`; the `TabId` → widget-index mapping exists only at the Qt model/tab-strip edge, in one place.
   The int-index lockstep across `QTabWidget`/`titles_`/`TabList` is removed.
3. **Rust `Document` is the single source of truth for dirty state.**
   The `QTextDocument` forwards edit notifications to `app-core`; the view reads the dirty flag (and title decoration) back from Rust.
   The view never maintains its own authoritative modified flag.

### 4. Error code ranges (amendment)

The original decision named one error type, `app_core::AppError`, and one rule: code `0` is success and every other number is stable.
Seven error types cross the seam now, each numbered from 0 or 1 upwards, so `1` meant "no such tab" from one QObject, "no provider configured" from another, and "something went wrong" from twenty-five hand-written literals in the adapter.
A code the view cannot interpret without also knowing which QObject produced it is not a typed error; it is a boolean with extra steps.

So each error type owns a **range**, and a code identifies its kind on its own:

| Range | Owner | Type |
|---|---|---|
| 0 | — | success, and only success |
| 1–99 | `app-core` | `AppError` |
| 100–199 | `ai-chat-core` | `ChatError` |
| 200–299 | `build-core` | `BuildError` (claimed by ADR-0040 out of the headroom below) |
| 600–699 | `lsp-core` | `LspError`, `EditError` |
| 700–799 | `vcs-core` | `VcsError` |
| 800–899 | `run-core` | `RunError` |
| 900–999 | `editor-core` / `edit-ops` | editing refusals, starting with `SelectionError` |
| 1000–1099 | `ui-shell` | adapter refusals — see below |

`AppError` keeps its existing 1–10 rather than moving into a new range: those numbers are the oldest part of the contract, nothing else has claimed them, and renumbering them would buy nothing.
`ChatError` is renumbered from 1–20 into 100–120, which is the only renumbering this amendment performs.
The 200–599 gap is deliberate headroom for the crates between the two, so a future error type does not have to squeeze in beside an existing one.
`build-core` took 200–299 out of it (ADR-0040), which is exactly what the headroom was for; 300–599 remains free.

**The adapter range exists because the adapter has refusals of its own.**
"No project is open", "unknown run configuration", "unknown console" and "the settings file could not be written" are not domain errors — no Qt-free crate has an opinion about them — and inventing a domain variant to carry each one would push view-shaped conditions down into the domain to satisfy a numbering scheme.
They live in `ui_shell::bridge::errors` instead, one named constant each, in 1000–1099.

**Rules.**
Codes are append-only within a range, exactly as before.
No literal number is ever written at a call site — neither in Rust (`FfiResult { code: 1, … }`) nor in C++ (`if (error.code == 705)`); both sides name the constant, and the C++ side reads it from the enum the bridge exports rather than repeating the number.
Success is `FfiResult::default()`, never `code: 0` spelled out.
Each owning crate has a test asserting its codes are unique and inside its range, so the next addition cannot silently collide.

*Rejected*: renumbering everything into ranges (the churn touches every branch in the view for no gain on codes that already do not collide); one flat global enum in a shared crate (every crate that can fail would depend on it, and `app-core` would import `vcs-core`'s failures to see its own); keeping per-QObject numbering and documenting the ambiguity (it was already documented, and the twenty-five `code: 1` literals are what documentation-instead-of-a-gate produces); a string error kind (ADR-0003's original argument against sentinels applies unchanged — a reworded kind breaks every branch).

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| Keep `QString` sentinel errors | No error kinds means no differentiated UI behavior (e.g. retry vs. give up), and success-as-empty-string is an easy silent bug at every new call site. |
| Serialize errors as JSON/QVariant maps across FFI | Heavier than needed; a fixed code + message struct covers the MVP and stays cheap and ABI-simple at the cxx boundary. |
| Keep int-index identity | Every tab close renumbers all later tabs in three structures at once; one missed mutation desynchronizes silently. Index lockstep is exactly the class of bug a stable id removes. |
| Pointer/handle identity (raw `Document*` across FFI) | Lifetime and ownership across the FFI boundary become unsafe-by-construction; an opaque `u64` id is as cheap and cannot dangle. |
| Keep dual dirty state with mirroring | Two writable copies of one fact guarantee eventual divergence; the mirror call (`setTabModified`) already exists only to paper over it. |

## Consequences

- Positive: call sites are explicit about failure, tab identity survives reorder/close, and dirty state has one owner — three whole classes of seam bugs become unrepresentable.
- Positive: error codes and `TabId` are view-agnostic, so the QML view swap ([layering.md](../layering.md)) inherits the same seam unchanged.
- Negative / accepted trade-offs: a one-time migration of every existing invokable and signal; the adapter must maintain the single `TabId` → index map for `QTabWidget`. Accepted as the cost of removing convention-based correctness.

## Related

- [ADR-0002: application layer and humble view](0002-application-layer-and-humble-view.md)
- [Layering rules](../layering.md)
