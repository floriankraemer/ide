# ide

Experiment building fast open source multi-language IDE.

Rust core, Qt6 Widgets UI via cxx-qt (QML planned later).

## Crates

The workspace is 13 crates in a layered architecture; see the crate table in [docs/architecture/overview.md](docs/architecture/overview.md) and the import rules in [docs/architecture/layering.md](docs/architecture/layering.md).

All docs are indexed in [docs/README.md](docs/README.md).

## Build

```sh
cargo build --workspace
cargo test --workspace
```

## License

GPLv3, see [LICENSE](LICENSE).
