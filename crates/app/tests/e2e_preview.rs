//! End-to-end flows for the preview: the in-tab edit/view toggle over a
//! Markdown file, and a standalone Mermaid file previewing at all.
//!
//! Their own test binary rather than more of `e2e.rs`, which sits at its
//! ratcheted size ceiling (`scripts/check-file-size.sh`) — the same reason
//! `e2e_run.rs` and `e2e_panes.rs` exist. `make e2e` runs all four.

use std::path::{Path, PathBuf};

use e2e::{mcp::Mcp, Ide, Mark};
use serde_json::json;

const APP: &str = env!("CARGO_BIN_EXE_app");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn wait_for_index(mcp: &Mcp) {
    e2e::wait_for("the project index to finish building", || {
        (mcp.call("index_status", json!({}))["ready"] == true).then_some(())
    });
}

/// Open one file through Go to File, returning its `tab_added` marker.
/// Duplicated from `e2e.rs`, as `e2e_run.rs` and `e2e_panes.rs` already
/// duplicate it: a helper this small is not worth a crate between four test
/// binaries.
fn open_file(ide: &Ide, name: &str) -> serde_json::Value {
    let main_window = ide.window().to_string();
    let mark = ide.mark();
    ide.key("ctrl+shift+n");
    ide.wait_for_event(mark, "the search popup to open", |e| {
        e["ev"] == "dialog_shown" && e["name"] == "search_everywhere"
    });
    ide.wait_for_focus_change(&main_window);
    ide.wait_for_ev(mark, "search_results");

    let mark = ide.mark();
    ide.type_text(name);
    ide.wait_for_event(mark, "results for the query", |e| {
        e["ev"] == "search_results" && e["count"].as_u64().unwrap_or(0) > 0
    });
    ide.key("Return");
    ide.wait_for_event(mark, "the search popup to accept", |e| {
        e["ev"] == "dialog_closed" && e["name"] == "search_everywhere" && e["accepted"] == true
    });
    ide.focus_main();
    ide.wait_for_event(mark, &format!("a tab for `{name}`"), |e| {
        e["ev"] == "tab_added" && e["title"] == name
    })
}

/// View mode: `Ctrl+Shift+M` renders the current tab in place, Escape puts
/// the source back.
///
/// The load-bearing assertion is not the toggle itself but what happens to
/// typing on either side of it. View mode keeps keystrokes out of the buffer
/// with focus alone — deliberately, because making the editor read-only
/// instead would make `EditorTabs::saveEditor` return "nothing to save" and
/// lose the file's edits silently. So this asserts against the *file on
/// disk*: the character typed in view mode must not be in it, and the one
/// typed after leaving view mode must be. An absence-of-marker check would
/// pass just as happily against an app that ignored every keystroke.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_preview_mode_toggle() {
    let name = "e2e_preview_mode_toggle";
    let mut ide = Ide::launch(name, APP, fixture("markdown"));
    let mcp = ide.mcp();
    ide.wait_for_ev(Mark::start(), "project_opened");
    wait_for_index(&mcp);

    let tab = open_file(&ide, "demo.md");
    let tab_id = tab["tab_id"].as_u64().expect("tab_id");

    let mark = ide.mark();
    ide.key("ctrl+shift+m");
    ide.wait_for_event(mark, "the tab to enter view mode", |e| {
        e["ev"] == "preview_mode" && e["tab_id"].as_u64() == Some(tab_id) && e["on"] == true
    });
    ide.wait_for_event(mark, "the in-tab render", |e| {
        e["ev"] == "preview_ready" && e["tab_id"].as_u64() == Some(tab_id)
    });

    // Must not reach the buffer: the preview has the focus.
    ide.type_text("X");

    let mark = ide.mark();
    ide.key("Escape");
    ide.wait_for_event(mark, "the tab to leave view mode", |e| {
        e["ev"] == "preview_mode" && e["tab_id"].as_u64() == Some(tab_id) && e["on"] == false
    });

    // Must reach the buffer: proves focus actually came back, which no
    // marker on its own can tell apart from an app ignoring every key.
    let mark = ide.mark();
    ide.type_text("Y");
    ide.wait_for_event(mark, "the tab to go dirty", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });

    let mark = ide.mark();
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the save to clean the tab", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });

    let saved =
        std::fs::read_to_string(ide.project_root().join("demo.md")).expect("the saved file");
    assert!(
        saved.contains('Y'),
        "the character typed after leaving view mode never reached the buffer:\n{saved}"
    );
    assert!(
        !saved.contains('X'),
        "a keystroke leaked into the buffer while the tab was in view mode:\n{saved}"
    );

    assert_eq!(ide.quit(), 0);
}

/// A standalone `.mermaid` file — not Markdown, no fences — previews
/// through the same built-in plugin, and toggles into view mode like any
/// other previewable file.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_standalone_mermaid_file_previews() {
    let name = "e2e_standalone_mermaid_file_previews";
    let mut ide = Ide::launch(name, APP, fixture("markdown"));
    let mcp = ide.mcp();
    ide.wait_for_ev(Mark::start(), "project_opened");
    wait_for_index(&mcp);

    let tab = open_file(&ide, "erd.mermaid");
    let tab_id = tab["tab_id"].as_u64().expect("tab_id");

    let mark = ide.mark();
    ide.key("ctrl+shift+m");
    ide.wait_for_event(mark, "the diagram file to enter view mode", |e| {
        e["ev"] == "preview_mode" && e["tab_id"].as_u64() == Some(tab_id) && e["on"] == true
    });
    ide.wait_for_event(mark, "the diagram to render", |e| {
        e["ev"] == "preview_ready" && e["tab_id"].as_u64() == Some(tab_id)
    });

    let mark = ide.mark();
    ide.key("ctrl+shift+m");
    ide.wait_for_event(mark, "the diagram file to leave view mode", |e| {
        e["ev"] == "preview_mode" && e["tab_id"].as_u64() == Some(tab_id) && e["on"] == false
    });

    assert_eq!(ide.quit(), 0);
}
