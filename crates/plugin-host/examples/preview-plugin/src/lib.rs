//! A previews contribution with no command — the wasm tier's other export.
//!
//! See `README.md` for how to build it; it is not part of the workspace.
//! `hello-plugin` exercises `on-command`; this one exercises `render`,
//! wrapping the source in a `<pre>` and returning no diagrams — the
//! smallest thing that proves a third-party component can serve a preview
//! the editor was never built to understand (ADR-0033).

wit_bindgen::generate!({
    path: "../../../plugin-api/wit",
    world: "preview-plugin",
});

use exports::ide::plugin::preview::{Guest as PreviewGuest, PreviewImage, Rendered};
use ide::plugin::host::{log, LogLevel};

struct PreviewPlugin;

impl Guest for PreviewPlugin {
    fn activate() -> Result<(), String> {
        log(LogLevel::Info, "preview-plugin activated");
        Ok(())
    }

    fn deactivate() {
        log(LogLevel::Info, "preview-plugin deactivated");
    }

    fn on_command(id: String, _args: Vec<String>) -> Result<(), String> {
        Err(format!("`{id}`: this example contributes no commands"))
    }
}

impl PreviewGuest for PreviewPlugin {
    /// `id` is always `"plain-text"`, the one contribution `plugin.toml`
    /// declares. `source` is escaped by hand rather than pulled in a
    /// crate: the whole point of this example is what the wasm tier's own
    /// four host imports and this one export can do without anything
    /// else, and comrak's escaping already lives on the host side of this
    /// same feature.
    fn render(id: String, source: String) -> Result<Rendered, String> {
        if id != "plain-text" {
            return Err(format!("unknown preview id `{id}`"));
        }
        let escaped = source
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        Ok(Rendered {
            html: format!("<pre>{escaped}</pre>"),
            images: Vec::<PreviewImage>::new(),
        })
    }
}

export!(PreviewPlugin);
