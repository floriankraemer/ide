//! xdotool wrappers.
//!
//! Input is always X11 input (ADR-0024): driving the app through MCP would
//! skip the tree widget, the shortcut and the dialog — the exact layer this
//! suite exists to cover — and produce a green run proving nothing about
//! `cpp/`.
//!
//! `--sync` is passed everywhere xdotool offers it. Without it the command
//! returns as soon as the X request is queued, and an assertion made
//! immediately after is asserting against the past.

use std::process::Command;

/// Run xdotool and return its trimmed stdout, panicking on a non-zero exit.
pub fn run(args: &[&str]) -> String {
    let output = Command::new("xdotool")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("xdotool {args:?}: {e}"));
    if !output.status.success() {
        panic!(
            "xdotool {args:?} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Run xdotool, returning `None` for a non-zero exit. Used only where "no
/// answer yet" is an expected poll outcome — `search` before the window is
/// mapped, `getactivewindow` before anything has focus.
pub fn try_run(args: &[&str]) -> Option<String> {
    let output = Command::new("xdotool").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Every visible toplevel whose title matches `pattern`.
pub fn visible_windows(pattern: &str) -> Vec<String> {
    try_run(&["search", "--onlyvisible", "--name", pattern])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .filter(|line| !line.is_empty())
        .collect()
}

/// The window holding the input focus.
///
/// `getwindowfocus`, not `getactivewindow`: the latter reads
/// `_NET_ACTIVE_WINDOW`, which needs a window manager. Under bare Xvfb there
/// is none, and X input focus is what xdotool's synthetic keys follow anyway
/// — so this is both the available answer and the correct one.
pub fn focused_window() -> Option<String> {
    try_run(&["getwindowfocus"]).filter(|id| !id.is_empty() && id != "0")
}
