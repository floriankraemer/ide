# Docs index

## Architecture

- [Architecture overview](architecture/overview.md) — arc42-lite orientation: context, quality goals, building-block view, crate table, communication, build, future scope.
- [Layering rules](architecture/layering.md) — binding dependency table, logic-placement rules, FFI seam rules, verification commands.
- [Project structure](architecture/project-structure.md) — repository layout, layer summary, where tests live, dev workflow pointers.

### Decisions

ADR numbers 0006 and 0013–0015 were never used; the gaps are historical and intentional.

- [ADR-0001: core tech stack](architecture/decisions/0001-core-tech-stack.md) — Rust core + Qt6 UI via cxx-qt; hybrid plugin system direction.
- [ADR-0002: application layer and humble view](architecture/decisions/0002-application-layer-and-humble-view.md) — `app-core` application layer; the Qt view is humble and holds zero rules.
- [ADR-0003: FFI conventions](architecture/decisions/0003-ffi-conventions.md) — FFI seam: typed errors, stable `TabId`, Rust-owned dirty state.
- [ADR-0004: MCP transport](architecture/decisions/0004-mcp-transport.md) — hand-rolled axum+tokio transport over the `rmcp` SDK.
- [ADR-0005: ADS build integration](architecture/decisions/0005-ads-build-integration.md) — docking via vendored Qt Advanced Docking System built through `cxx-qt-build`.
- [ADR-0007: embedded terminal](architecture/decisions/0007-embedded-terminal.md) — `portable-pty` + `alacritty_terminal` + custom QPainter grid widget.
- [ADR-0008: project index](architecture/decisions/0008-project-index.md) — hybrid tantivy(ngram) + ripgrep-crates text index; name-based symbol schema.
- [ADR-0009: find and replace](architecture/decisions/0009-find-and-replace.md) — matching in `editor-core`; project-wide replace through `index-core`.
- [ADR-0010: search everywhere](architecture/decisions/0010-search-everywhere.md) — one popup over ranked tiers, persistent index, batched results.
- [ADR-0011: code navigation](architecture/decisions/0011-code-navigation.md) — local-file-first declaration resolution; supertype edges as third index schema.
- [ADR-0012: MCP protocol, index and lifecycle](architecture/decisions/0012-mcp-protocol-index-and-lifecycle.md) — real MCP protocol surface, index tools, user-controlled lifecycle; index shared as `Arc<RwLock<IndexSlot>>`.
- [ADR-0016: LSP client](architecture/decisions/0016-lsp-client.md) — Qt-free `lsp-core`: blocking threads, supervised child servers, catalog + user-override config.
- [ADR-0017: settings-model crate](architecture/decisions/0017-settings-model-crate.md) — `settings-model`: Qt-free home for the settings pages' rules.
- [ADR-0018: single-source language detection](architecture/decisions/0018-single-source-language-detection.md) — one source of truth for file→language detection: `syntax-core`'s registry.
- [ADR-0019: LSP refactoring](architecture/decisions/0019-lsp-refactoring.md) — refactoring over LSP: code actions, rename, applying workspace edits.

## Plans

All plan documents are complete except the index performance plan and the large-files plan; the rest remain as historical records of how each feature phase was delivered.

- [MVP implementation plan](architecture/mvp-implementation-plan.md) — MVP editor shell; marked historical.
- [Settings, docking, theming, MCP plan](architecture/settings-docking-theming-mcp-plan.md) — settings, docking, theming, MCP foundation, line numbers, tab reorder, syntax foundation.
- [Language, folding, Class View, terminal, search plan](architecture/language-folding-classview-terminal-search-plan.md) — language expansion, folding, Class View, terminal, project index and search.
- [Find & Replace plan](architecture/find-replace-plan.md) — find and replace, in-editor and project-wide.
- [Search Everywhere plan](architecture/search-everywhere-plan.md) — Search Everywhere popup and Search Results dock.
- [Code navigation plan](architecture/code-navigation-plan.md) — Go to Declaration, Find Usages, Go to Implementation, jump history.
- [Language platform plan](architecture/language-platform-plan.md) — extensible tree-sitter languages, per-language theming, runtime grammars, LSP.
- [Refactoring plan](architecture/refactoring-plan.md) — rename, extract via code actions, signature on hover.
- [Index performance plan](architecture/index-performance-plan.md) — faster project index build and a status-bar indexing indicator.
- [Large files and the binary viewer plan](architecture/large-files-and-binary-viewer-plan.md) — no-wrap default, highlighting size ceilings, O(1) fold lookup, read-only hex view for binary files.

## Design

- [Language platform UI](design/language-platform-ui.md) — UX spec for the three language-platform settings pages and the Problems dock.

## Product

- [MVP proposal](product/mvp-proposal.md) — original MVP product proposal; draft, superseded by shipped scope.
