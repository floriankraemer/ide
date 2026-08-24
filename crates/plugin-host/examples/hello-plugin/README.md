# `hello-plugin` — a worked example of the wasm tier

A plugin that contributes one command, reads one file it was granted, and says so.
Everything the executable tier can do in about forty lines.

It is **not** part of the workspace build, and that is deliberate.
Compiling a component needs a `wasm32-unknown-unknown` target and `wasm-tools`, neither of which `docker/Dockerfile` installs, and adding a second toolchain to the builder image so that CI can compile one example would cost every build for one file.
The tier is tested instead by components written inline in the component-model text format — see `crates/plugin-host/src/wasm/tests.rs`.
Its own `[workspace]` table is what keeps the parent workspace from adopting it.

## Building it

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-tools            # once
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
    target/wasm32-unknown-unknown/release/hello_plugin.wasm \
    -o plugin.wasm
```

## Installing it

Copy `plugin.toml` and the `plugin.wasm` you just built into
`<config_dir>/plugins/example-hello/`, along with the `data/` directory, and
restart the editor.
The command appears in the palette as "Example: Say Hello".

## What to notice

- `plugin.toml` grants `read-files = ["${plugin_dir}/data"]` and nothing else.
  Reading `../anything`, or a symlink out of `data/`, comes back as `denied("read-files")` — a value the plugin can log, not a link error it cannot see.
- `notify` is granted; `workspace-root` is not, so asking for it is refused at runtime rather than being missing at link time.
- `log` needs no grant at all.
- `activate` returning `Err` disables the plugin with that message on the Plugins page.
