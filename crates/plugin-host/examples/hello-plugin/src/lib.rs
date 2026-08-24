//! The smallest plugin that exercises every part of the tier.
//!
//! See `README.md` for how to build it; it is not part of the workspace.

wit_bindgen::generate!({
    // The same world the host generates its side from, so a mismatch is a
    // compile error on one side or the other rather than a failed
    // instantiation.
    path: "../../../plugin-api/wit",
    world: "plugin",
});

use ide::plugin::host::{log, notify, read_file, workspace_root, HostError, LogLevel};

struct HelloPlugin;

impl Guest for HelloPlugin {
    /// Called once, after instantiation. Returning `Err` here disables the
    /// plugin and shows the message on the Plugins page.
    fn activate() -> Result<(), String> {
        log(LogLevel::Info, "hello-plugin activated");

        // Not granted in `plugin.toml`, so this comes back as a refusal
        // that names the capability — a value, not a link error.
        if let Err(HostError::Denied(capability)) = workspace_root() {
            log(
                LogLevel::Debug,
                &format!("no workspace root: `{capability}` was not granted"),
            );
        }
        Ok(())
    }

    /// Called before the plugin is dropped. Best-effort on the host side.
    fn deactivate() {
        log(LogLevel::Info, "hello-plugin deactivated");
    }

    /// One command, identified by the id `plugin.toml` contributes.
    fn on_command(id: String, _args: Vec<String>) -> Result<(), String> {
        if id != "example-hello.say-hello" {
            return Err(format!("unknown command `{id}`"));
        }

        // Inside `${plugin_dir}/data`, which is the one prefix granted.
        // `data/../plugin.toml`, or a symlink out of `data/`, would be
        // `denied` instead.
        let greeting = match read_file("data/greeting.txt") {
            Ok(bytes) => String::from_utf8_lossy(&bytes).trim().to_string(),
            Err(err) => return Err(format!("{err:?}")),
        };

        notify(&greeting).map_err(|err| format!("{err:?}"))?;
        Ok(())
    }
}

export!(HelloPlugin);
