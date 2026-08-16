# 0003. FFI seam conventions: typed errors, stable TabId, Rust-owned dirty state

## Status

Accepted

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
