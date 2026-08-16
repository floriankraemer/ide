# ide

Experiment building fast open source multi-language IDE.

Rust core, Qt6 Widgets UI via cxx-qt (QML planned later).
Early stage, MVP text editor shell.

## Crates

- `crates/editor-core` — domain: rope `Document`, `TabList`, binary detection.
- `crates/project-model` — domain: `ProjectSession`, directory tree, filesystem watcher.
- `crates/app-core` — application layer: `AppSession`, commands, `TabId`, `AppError`.
- `crates/ui-shell` — Qt adapter (cxx-qt bridge) + view (Qt Widgets).
- `crates/app` — main entry point.

See `docs/architecture/` for layering rules and ADRs.

## Build

```sh
cargo build --workspace
cargo test --workspace
```

## License

GPLv3, see [LICENSE](LICENSE).
