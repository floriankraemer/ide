# Docs index

## Architecture

- [Architecture overview](architecture/overview.md) — arc42-lite orientation: context, quality goals, building-block view, crate table.
- [Layering rules](architecture/layering.md) — binding dependency table, logic-placement rules, FFI seam rules, verification commands.
- [MVP implementation plan](architecture/mvp-implementation-plan.md) — historical plan for the delivered MVP; superseded by layering.md and the ADRs where they disagree.

### Decisions

- [ADR-0001: core tech stack](architecture/decisions/0001-core-tech-stack.md) — Rust core + Qt6 (cxx-qt) UI, hybrid native/WASM plugin system as future direction.
- [ADR-0002: application layer and humble view](architecture/decisions/0002-application-layer-and-humble-view.md) — Qt-free `app-core` crate; the Qt view displays and forwards intent only.
- [ADR-0003: FFI seam conventions](architecture/decisions/0003-ffi-conventions.md) — typed error codes, stable `TabId(u64)`, Rust-owned dirty state across the cxx-qt seam.

## Product

- [MVP proposal](product/mvp-proposal.md) — scope and user stories for the minimal text-editor shell.
