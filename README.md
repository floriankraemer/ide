# ide

A fast, open-source, multi-language IDE — built as a Rust core with a Qt6 Widgets UI, bridged via [cxx-qt](https://github.com/KDAB/cxx-qt) (QML planned later).

The goal is a JetBrains-like experience with the performance of a native Rust core: low typing latency, large files that stay smooth, and business logic that is fully testable without a display.

## What it can do

- Open a project folder, browse the tree, edit and save tabs.
- Tree-sitter syntax highlighting, folding, and a Class View outline — 29 bundled grammar crates covering roughly 35 languages.
- Project-wide text and symbol index: search, Go to Declaration, Find Usages, Go to Implementation, jump history.
- An LSP client with diagnostics, hover, completion, and refactoring (rename, Extract Method/Class via code actions).
- Find and replace, an embedded terminal, ADS-based docking, theming, settings and keymap.
- A built-in MCP server, so an AI agent can read and drive the editor and query the project index.

## How it is built

The workspace is 13 crates in a strict layered architecture: only the UI shell touches Qt, everything else is plain Rust that runs under `cargo test` with no display server.

- New here? Start with the [architecture overview](docs/architecture/overview.md) and the [project structure](docs/architecture/project-structure.md).
- The binding dependency rules live in [docs/architecture/layering.md](docs/architecture/layering.md).
- All architecture, decision, plan, and product docs are indexed in [docs/README.md](docs/README.md).

## Building

Development happens inside Docker containers, driven by the Makefile:

```sh
make linux-image     # build/refresh the builder image
make test            # cargo test --workspace inside it
make lint            # clippy -D warnings + rustfmt --check
```

If you prefer the bare host and have a Qt6 dev toolchain installed:

```sh
cargo build --workspace
cargo test --workspace
```

## License

GPLv3, see [LICENSE](LICENSE).
