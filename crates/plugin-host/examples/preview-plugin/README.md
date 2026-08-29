# `preview-plugin` — a worked example of the wasm tier's `render` export

A plugin that contributes a preview for `.txt` files: wraps the source in
`<pre>`, escaped, no diagrams.
Everything the `preview-plugin` world's `render` export can do in about
thirty lines — `hello-plugin`'s twin, exercising `render` instead of
`on-command`.

It is **not** part of the workspace build, for the same reason
`hello-plugin` is not: compiling a component needs a
`wasm32-unknown-unknown` target and `wasm-tools`, neither of which
`docker/Dockerfile` installs.
Its own `[workspace]` table is what keeps the parent workspace from
adopting it.

This example **has** been built and run end to end during development —
compiled for `wasm32-unknown-unknown`, turned into a real component with
`wasm-tools component new`, validated, and loaded through the real
`plugin_host::load` → `WasmTier::start` → `WasmTier::render` path, not just
against hand-written WAT text.
The result: `<pre>hello &lt;world&gt; &amp; friends</pre>`, zero images,
zero disabled plugins — proof the `preview-plugin` world, the bindgen
`with`-sharing between it and the base `plugin` world, and the host's
world-selection rule (`contributes.previews` + `[wasm]` → the wider world)
all hold against a real, `rustc`-compiled component.

## Building it

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-tools            # once
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
    target/wasm32-unknown-unknown/release/preview_plugin.wasm \
    -o plugin.wasm
```

## Installing it

Copy `plugin.toml` and the `plugin.wasm` you just built into
`<config_dir>/plugins/example-preview/`, and restart the editor.
Opening a `.txt` file previews it through this plugin's `render` export.

## What to notice

- `plugin.toml` contributes `previews`, not `commands`, and still needs
  `[wasm]` — a `previews` contribution served natively (the built-in
  Markdown preview) needs no component, but a *third-party* one does, so
  the host tries the wider `preview-plugin` world whenever both are
  present.
- The component implements **both** `Guest` (the base world's
  `activate`/`deactivate`/`on-command`, via `include plugin;`) and
  `exports::ide::plugin::preview::Guest` (`render`) on the same type — the
  `export!` macro wires up both automatically.
- `render` returns SVG-shaped `images`, never pixels: rasterising belongs
  to the host, which already owns a rasteriser and the bundled font
  (ADR-0033). This example returns none, since a `.txt` preview has no
  diagram to draw.
- A component that claims `contributes.previews` but never implements
  `preview.render` fails to instantiate — one disabled plugin, not a
  crash — the same fail-soft guarantee ADR-0028 gives `commands`.
