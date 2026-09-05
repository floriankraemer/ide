//! End-to-end flows for the editor's split panes: splitting a tab
//! into a pane of its own, dragging a tab between panes, and the layout
//! surviving a restart.
//!
//! Their own test binary rather than more of `e2e.rs`, which sits at its
//! ratcheted size ceiling (`scripts/check-file-size.sh`) — the same reason
//! `e2e_run.rs` exists. `make e2e` runs all three.

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
/// Duplicated from `e2e.rs` rather than shared, as `e2e_run.rs` already
/// duplicates it: a helper this small is not worth a crate between three
/// test binaries.
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

/// The centre of a tab's label on screen, from its `tab_added` (or
/// `tab_moved`) marker.
fn tab_centre(tab: &serde_json::Value) -> (i32, i32) {
    let rect: Vec<i64> = tab["rect"]
        .as_array()
        .expect("the marker carries the tab's rect")
        .iter()
        .map(|v| v.as_i64().expect("an integer"))
        .collect();
    (
        (rect[0] + rect[2] / 2) as i32,
        (rect[1] + rect[3] / 2) as i32,
    )
}

/// Split the tab `tab` into a pane of its own through its context menu,
/// returning the `tab_moved` marker that says where it landed.
fn split_tab_out(ide: &Ide, tab: &serde_json::Value) -> serde_json::Value {
    let mark = ide.mark();
    let (x, y) = tab_centre(tab);
    ide.click_at(x, y, 3);
    ide.wait_for_event(mark, "the tab context menu", |e| {
        e["ev"] == "dialog_shown" && e["name"] == "tab_context_menu"
    });
    for _ in 0..3 {
        ide.key("Down"); // Close, Close Others, (separator), Split Vertical
    }
    ide.key("Return");
    ide.wait_for_ev(mark, "tab_moved")
}

/// Every `group` node in a persisted editor layout, in tree order.
fn groups_of(node: &serde_json::Value) -> Vec<&serde_json::Value> {
    match node["type"].as_str() {
        Some("group") => vec![node],
        Some("splitter") => node["children"]
            .as_array()
            .map(|children| children.iter().flat_map(groups_of).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Split the editor, quit, and come back to the same layout.
///
/// `app-config`'s TOML round-trip is unit-tested; *the view reconstructing
/// itself from it* is not, and that is the whole flow. The persisted layout
/// is read back with `app-config`'s own types rather than a regex, so the
/// test cannot pass by agreeing with a re-implementation of the format.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_split_editor_persistence() {
    let name = "e2e_split_editor_persistence";
    let mut ide = Ide::launch(name, APP, fixture("tiny"));
    let mcp = ide.mcp();
    ide.wait_for_ev(Mark::start(), "project_opened");
    wait_for_index(&mcp);

    open_file(&ide, "main.rs");
    let second = open_file(&ide, "greeting.rs");
    assert_eq!(
        second["pane"].as_i64(),
        Some(0),
        "both tabs start in one pane"
    );

    // Split through the tab's own context menu, at the coordinates the view
    // reported for that tab — not at a rectangle computed from the window
    // geometry and the style's metrics.
    let mark = ide.mark();
    split_tab_out(&ide, &second);

    let split = ide.wait_for_ev(mark, "split_created");
    assert_eq!(
        split["orientation"], "h",
        "Split Vertical puts panes side by side"
    );
    e2e::wait_for("the editor to report two panes", || {
        ide.events_since_of(mark, "pane_count")
            .last()
            .and_then(|e| e["n"].as_u64())
            .filter(|n| *n == 2)
            .map(|_| ())
    });
    ide.focus_main();

    assert_eq!(ide.quit(), 0);

    // Read the persisted layout with the app's own types.
    let settings = app_config::load(&ide.config_dir()).expect("the settings the app just wrote");
    let layout: serde_json::Value =
        serde_json::from_str(&settings.editor_layout).expect("editor_layout is JSON");
    let panes = groups_of(&layout);
    assert_eq!(
        panes.len(),
        2,
        "the persisted layout does not describe two panes"
    );
    assert_eq!(panes[0]["files"].as_array().map(Vec::len), Some(1));
    assert_eq!(panes[1]["files"].as_array().map(Vec::len), Some(1));

    // And back.
    ide.relaunch();
    ide.wait_for_ev(Mark::start(), "project_opened");
    let restored: Vec<_> = e2e::wait_for("both tabs to be restored", || {
        let tabs = ide.events_since_of(Mark::start(), "tab_added");
        (tabs.len() == 2).then_some(tabs)
    });
    assert_eq!(restored[0]["title"], "main.rs");
    assert_eq!(restored[0]["pane"].as_i64(), Some(0));
    assert_eq!(restored[1]["title"], "greeting.rs");
    assert_eq!(restored[1]["pane"].as_i64(), Some(1));
    assert_eq!(
        ide.events_since_of(Mark::start(), "pane_count")
            .last()
            .and_then(|e| e["n"].as_u64()),
        Some(2),
        "the restored window has a different number of panes"
    );

    assert_eq!(ide.quit(), 0);
}

/// Drag a tab out of one pane and into another.
///
/// The gesture is the product feature: `setMovable(true)` only ever
/// reordered tabs inside one strip, so this is the first thing that carries
/// a tab across a splitter. Driven with the real pointer — a synthetic
/// call to the move function would prove nothing about the drag handshake,
/// which is the only part that was missing.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_drag_tab_between_panes() {
    let name = "e2e_drag_tab_between_panes";
    let mut ide = Ide::launch(name, APP, fixture("tiny"));
    let mcp = ide.mcp();
    ide.wait_for_ev(Mark::start(), "project_opened");
    wait_for_index(&mcp);

    let first = open_file(&ide, "main.rs");
    let second = open_file(&ide, "greeting.rs");

    // Split the second tab into a pane of its own, so there is somewhere to
    // drag it back from.
    let split = split_tab_out(&ide, &second);
    assert_eq!(
        split["pane"].as_i64(),
        Some(1),
        "the split made a second pane"
    );

    // Drag it back onto the first pane's tab strip, at that strip's own
    // reported coordinates.
    let mark = ide.mark();
    ide.drag(tab_centre(&split), tab_centre(&first));

    let moved = ide.wait_for_ev(mark, "tab_moved");
    assert_eq!(moved["title"], "greeting.rs");
    assert_eq!(
        moved["pane"].as_i64(),
        Some(0),
        "the tab landed in the other pane"
    );
    e2e::wait_for("the emptied pane to collapse", || {
        ide.events_since_of(mark, "pane_count")
            .last()
            .and_then(|e| e["n"].as_u64())
            .filter(|n| *n == 1)
            .map(|_| ())
    });

    // The layout the app persists agrees: one pane, both files in it.
    ide.focus_main();
    assert_eq!(ide.quit(), 0);
    let settings = app_config::load(&ide.config_dir()).expect("the settings the app just wrote");
    let layout: serde_json::Value =
        serde_json::from_str(&settings.editor_layout).expect("editor_layout is JSON");
    let panes = groups_of(&layout);
    assert_eq!(
        panes.len(),
        1,
        "the persisted layout still describes two panes"
    );
    assert_eq!(panes[0]["files"].as_array().map(Vec::len), Some(2));
}
