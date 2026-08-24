# 0027. Icon themes: our own pack format, rasterised in Rust

## Status

Proposed.
Implemented by [the plugin host and icon themes plan](../plugin-host-and-icon-themes-plan.md); this ADR covers tasks P3 and P4 (`icon-theme`, the Material import).
The first consumer of the host in [ADR-0026](0026-plugin-host.md), and bound by the single-detection rule of [ADR-0018](0018-single-source-language-detection.md).

## Context

The IDE draws no icons.
`ProjectTreeModel::data` returns an invalid `QVariant` for `Qt::DecorationRole` on purpose, and there is no `QIcon`, no `.qrc` and no bundled asset anywhere under `crates/`.
Every file row, editor tab and search result is a line of text, and a directory of a hundred files is a wall of them.

The intended source is the MIT-licensed Material icon theme: roughly 900 SVGs plus a generated mapping table.
That is pure data, which is what makes it the right first contribution point for the plugin host — and it leaves four questions this decision answers.

**Who turns an SVG into pixels.**
Qt can do it, through `QSvgRenderer` and the `qsvg` image-format plugin, but Qt6Svg is in neither toolchain: `docker/Dockerfile` installs `qt6-base-dev` on Linux and cross-builds `qt6-qtbase` — base only — for Windows.
Adding it costs two Dockerfile changes, a cross-built `qt6-qtsvg` through MXE, and shipping `Qt6Svg.dll` plus the `qsvg` imageformat plugin into `dist/windows/`, which today carries `platforms/qwindows.dll` and nothing else.

**Which crate decides that `main.rs` is Rust.**
ADR-0018 makes `syntax-core`'s registry the single source of file-to-language detection and forbids a second extension table anywhere.
An icon theme is, however, *mostly* an extension table — the upstream Material data has around 700 extension entries — so the boundary needs stating rather than assuming.

**What the pack file is.**
The upstream data is a VS Code `iconTheme` JSON: `iconDefinitions` keyed by an opaque id, then `fileNames`, `fileExtensions`, `folderNames`, `folderNamesExpanded`, `languageIds`, and a parallel `light` block, plus fields for features we do not have (file-icon-less "hidesExplorerArrows", per-icon font definitions, clone/colour directives).

**What the pixels look like at the seam.**
The bytes have to reach a `QImage` without a per-pixel fixup on the C++ side.

## Decision

A Qt-free crate `icon-theme` (`crates/icon-theme`), depending on `serde`, `toml` and `resvg`, in two halves that cost very different amounts.

`IconPack` is `pack.toml` and the resolution order over it — exact file name, then the longest multi-part extension, then the single extension, then the language id, then the pack default; folders resolve by name against a closed and an open table.
It is pure data, cheap enough to run per visible row, and every rule in it is unit-tested.
`IconRenderer` rasterises with `resvg` into premultiplied RGBA8 and memoises by `(pack id, icon id, px)`.

Four rules carry the weight.

**SVG is rasterised in Rust, not by Qt.**
`resvg` costs one dependency tree that already cross-builds for `x86_64-pc-windows-gnu` through MXE, against the Qt route's two Dockerfile changes, a cross-built `qt6-qtsvg`, and two more files in `dist/windows/`.
It is also unit-testable without a `QGuiApplication`, which the Qt route is not: the rasteriser's tests are ordinary `cargo test`, not part of the headless E2E pass.
`resvg` is taken with `default-features = false`, dropping `text`, `system-fonts`, `memmap-fonts` and `raster-images` — an icon is paths, and each of those features is something more that would have to cross-build.

**The resolver is handed a language id; it does not detect one.**
`IconPack::file_icon` takes `language_id: Option<&str>`, and `icon-theme` does not depend on `syntax-core`.
The extension table it *does* own is an icon table, not a detection table: it answers "which art", never "which language", and nothing in the IDE may ask it the second question.
The join happens in `ui-shell`/`app-core`, which already know both crates, so ADR-0018's single detection table stays single.

**The pack format is ours, generated from upstream rather than read from it.**
`pack.toml` is TOML with `deny_unknown_fields`, the same shape every other manifest in this tree has, and it maps icon *ids* directly to file names rather than through the `iconDefinitions` indirection.
P4's import script performs the conversion once, at import time, and the result is committed under `third_party/`; re-running the script against a newer upstream version is how the pack is updated.
Reading the upstream JSON at runtime would mean carrying `serde_json`, implementing the parts of the VS Code icon-theme contract we do not use well enough to ignore them safely, and tracking a format we do not control in a file we cannot fix.

**Pixels are premultiplied RGBA8.**
That is tiny-skia's native byte order, and it is exactly `QImage::Format_RGBA8888_Premultiplied`.
`Format_ARGB32_Premultiplied` is BGRA on little-endian and would need a per-pixel swizzle at the seam, which is the kind of thing that gets simplified back out and ships as every icon with its red and blue channels exchanged.

Two smaller choices are worth recording because they look arbitrary.
The icons directory is the fixed name `icons`, not a pack field: a pack-supplied directory would be an untrusted path joined to a plugin directory, and `plugin-api`'s path-safety rules cover the manifest, not a path a file the manifest points to then names.
Every name table — file names, folder names and extensions — matches case-insensitively, which is upstream Material's own rule.
This corrects what this ADR said when it was written, and the correction came from the import at P4: 5.38.1 ships `dockerfile` and `makefile` in lower case, alongside 21 mixed-case `fileNames` keys and 35 mixed-case `folderNames` keys, because VS Code lowercases a row's name before the lookup.
Matching by exact case left `Dockerfile`, `Makefile`, `LICENSE` and every `META-INF` with no icon at all.
The import script lowercases those keys, refusing to proceed if two of them ever fold together, so the mixed-case keys that are dead in VS Code itself work here.

## Consequences

- The layering table gains one row, and `ui-shell` will gain one dependency at P5.
- Icon resolution is testable as data: the whole precedence order, the folder states and the light substitution are covered without a window, a theme, or a filesystem.
- Reading a pack's files is the caller's job through the `IconAssets` trait, so a built-in plugin's embedded SVGs and an installed plugin's on-disk ones go through one renderer. `icon-theme` therefore depends on neither `plugin-host` nor `std::fs` for assets, which is what ADR-0026 means by joining the two in `app-core`.
- An icon id with no art behind it falls back to the pack's default file icon rather than failing the row: one wrong icon beats an error dialog over a repaint.
- The Windows bundle is unchanged — no new DLL, which was the cross-platform gate this plan set itself.
- The cost is that a new upstream Material release is an import-script run and a committed diff, not a version bump. That is the same trade `syntax-core`'s vendored grammars already make, and it is what keeps the format ours.

### Rejected alternatives

**Render SVG with Qt.**
Rejected on toolchain cost, as above, and because it would put the rasteriser in the one crate that cannot be unit-tested.
The escape hatch remains open: nothing outside `IconRenderer` knows how the pixels were produced.

**Ship pre-rendered PNGs at a few fixed sizes.**
No dependency at all, and the smallest possible seam.
Rejected because the sizes are not knowable — the tree, the tabs and the result lists want different ones, and a HiDPI display wants a non-integer multiple of all of them — and because 900 icons times four sizes times two appearances is a repository full of generated binaries.

**Read the VS Code `material-icons.json` at runtime.**
Tempting, since it is what upstream ships and it would make an update a file copy.
Rejected because it makes an external, undocumented format part of our runtime contract, and because the parts of it we would ignore are not obviously ignorable.

**Depend on `syntax-core` for detection inside the resolver.**
It would let a caller pass a path and nothing else.
Rejected by ADR-0018: the two extension tables would then disagree about some file eventually, and the icon table — the bigger of the two — would be the one people edit.

**A hardcoded icon table with no pack format.**
Covered by ADR-0026's rejected alternatives: it has no incremental path to a host, and the extension mechanism was asked for explicitly.
