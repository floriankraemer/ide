//! End-to-end flows: the real binary, under Xvfb, driven with xdotool.
//!
//! Every test here is `#[ignore]`d, so `cargo test --workspace` is exactly as
//! fast as it was before this file existed. `make e2e` runs them.
//!
//! They live in `crates/app` rather than `crates/e2e` for one reason:
//! `CARGO_BIN_EXE_app` is only defined for integration tests of the crate
//! that declares the binary. The harness itself is `crates/e2e`, which
//! depends on no workspace crate.
//!
//! What these cover is what nothing else can: signal/slot wiring, cross-
//! thread delivery, widget lifetime, index-identity mapping at the model
//! edge, keyboard and focus routing, dialog flows, and the view restoring
//! itself from persisted state. Everything that would still be meaningful
//! with the Qt event loop removed is a unit test somewhere else.
//!
//! One product limitation shapes what can be asserted where: MCP's
//! `read_buffer` answers from `editor_core::Document`'s rope, which is
//! populated on open and refreshed only on save — it does not see unsaved
//! typing. Dirty state is therefore observed through the marker stream and
//! content through the file on disk.

use std::path::{Path, PathBuf};

use e2e::{mcp::Mcp, Ide, Mark};
use serde_json::json;

const APP: &str = env!("CARGO_BIN_EXE_app");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The fixture file's content as it is checked in — the independent answer
/// to "did the app change this file", read without going through the app.
fn fixture_text(name: &str, relative: &str) -> String {
    std::fs::read_to_string(fixture(name).join(relative)).expect("fixture file")
}

/// Wait until the project index reports itself complete.
///
/// The step a naive harness would spell `sleep`. Everything that searches —
/// Go to File, Search Everywhere, the name-based rename — answers "still
/// being built" until this is true, and a fixed delay only makes that
/// answer intermittent.
fn wait_for_index(mcp: &Mcp) {
    e2e::wait_for("the project index to finish building", || {
        (mcp.call("index_status", json!({}))["ready"] == true).then_some(())
    });
}

/// Open the Search Everywhere popup with `shortcut` and wait until its
/// opening query has been answered, so a later `search_results` marker can
/// only be one the test's own typing caused.
fn open_search_popup(ide: &Ide, shortcut: &str) -> Mark {
    let main_window = ide.window().to_string();
    let mark = ide.mark();
    ide.key(shortcut);
    ide.wait_for_event(mark, "the search popup to open", |e| {
        e["ev"] == "dialog_shown" && e["name"] == "search_everywhere"
    });
    ide.wait_for_focus_change(&main_window);
    ide.wait_for_ev(mark, "search_results");
    ide.mark()
}

/// Type a query into an open search popup and take the top hit.
fn accept_top_hit(ide: &Ide, mark: Mark, query: &str) {
    ide.type_text(query);
    let hits = ide.wait_for_event(mark, &format!("results for `{query}`"), |e| {
        e["ev"] == "search_results" && e["count"].as_u64().unwrap_or(0) > 0
    });
    assert!(hits["count"].as_u64().unwrap() > 0);
    ide.key("Return");
    ide.wait_for_event(mark, "the search popup to accept", |e| {
        e["ev"] == "dialog_closed" && e["name"] == "search_everywhere" && e["accepted"] == true
    });
    ide.focus_main();
}

/// Open one file through Go to File, returning its `tab_added` marker.
fn open_file(ide: &Ide, name: &str) -> serde_json::Value {
    let mark = open_search_popup(ide, "ctrl+shift+n");
    accept_top_hit(ide, mark, name);
    ide.wait_for_event(mark, &format!("a tab for `{name}`"), |e| {
        e["ev"] == "tab_added" && e["title"] == name
    })
}

fn buffer(mcp: &Mcp, tab_id: u64) -> String {
    mcp.call("read_buffer", json!({ "tab_id": tab_id }))["content"]
        .as_str()
        .expect("read_buffer returns a string")
        .to_string()
}

// ---------------------------------------------------------------------------

/// The harness itself: a seeded launch reaches a mapped, focused window with
/// its project open, and Ctrl+Q exits cleanly. Every flow below assumes this,
/// so it is worth failing on its own.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_launches_with_its_project_and_quits() {
    let mut ide = Ide::launch(
        "e2e_launches_with_its_project_and_quits",
        APP,
        fixture("tiny"),
    );

    let opened = ide.wait_for_ev(Mark::start(), "project_opened");
    assert_eq!(
        Path::new(opened["root"].as_str().expect("root is a string")),
        ide.project_root(),
        "the app opened a different project than the one it was seeded with"
    );

    assert_eq!(ide.quit(), 0);
}

/// Open, edit, save, undo.
///
/// Two assertions here have no other net. The marker's `tab_id` and MCP's
/// view of the open buffers must agree — a disagreement is the identity
/// mapping bug class, where an off-by-one at the model edge closes the wrong
/// tab. And **one** Ctrl+Z must undo the whole edit: that is the edit-block
/// granularity guard, and C++ is where the `beginEditBlock` lives.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_open_project_edit_save() {
    let name = "e2e_open_project_edit_save";
    let mut ide = Ide::launch(name, APP, fixture("tiny"));
    let mcp = ide.mcp();
    ide.wait_for_ev(Mark::start(), "project_opened");
    wait_for_index(&mcp);

    let original = fixture_text("tiny", "src/main.rs");
    let tab = open_file(&ide, "main.rs");
    let tab_id = tab["tab_id"].as_u64().expect("tab_id");

    // The view says which widget index it put the tab at; MCP says which
    // TabId the session believes is open. They are two different layers'
    // answers to the same question and must be the same answer.
    let buffers = mcp.call("list_open_buffers", json!({}));
    let buffers = buffers.as_array().expect("a list of buffers");
    assert_eq!(buffers.len(), 1, "expected exactly one open buffer");
    assert_eq!(buffers[0]["tab_id"].as_u64(), Some(tab_id));
    assert_eq!(buffers[0]["title"], tab["title"]);
    assert_eq!(tab["index"].as_i64(), Some(0));
    assert_eq!(buffer(&mcp, tab_id), original);

    // One character, so that one Ctrl+Z is unambiguously one edit rather
    // than a bet on Qt's keystroke coalescing.
    let mark = ide.mark();
    ide.type_text("x");
    ide.wait_for_event(mark, "the tab to go dirty", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });

    let edited = format!("x{original}");
    let mark = ide.mark();
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the tab to go clean", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert_eq!(
        ide.read_project_file("src/main.rs"),
        edited,
        "Ctrl+S did not write the edit to disk"
    );
    assert_eq!(buffer(&mcp, tab_id), edited);

    // The guard: one undo, the whole edit. Observed after a save because
    // `read_buffer` answers from `editor_core::Document`'s rope, which is
    // only refreshed on save (see the note at the top of this file).
    let mark = ide.mark();
    ide.key("ctrl+z");
    ide.wait_for_event(mark, "the tab to go dirty again", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });
    let mark = ide.mark();
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the undone tab to be saved", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert_eq!(
        ide.read_project_file("src/main.rs"),
        original,
        "one Ctrl+Z did not undo the whole edit"
    );
    assert_eq!(buffer(&mcp, tab_id), original);

    assert_eq!(ide.quit(), 0);
}

/// Search Everywhere, debounced, then a jump.
///
/// The assertion worth the flow is the count: ten keystrokes must produce at
/// most two delivered result sets. That proves both that typing is debounced
/// and that superseded generations were *discarded* rather than rendered —
/// the canonical cross-thread bug, and one `bridge.rs`'s own tests cannot
/// see because they test the cancel decision, not the delivery.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_search_everywhere_jump() {
    let name = "e2e_search_everywhere_jump";
    let mut ide = Ide::launch(name, APP, fixture("tiny"));
    let mcp = ide.mcp();
    ide.wait_for_ev(Mark::start(), "project_opened");
    wait_for_index(&mcp);

    // Somewhere to come back to.
    let main_tab = open_file(&ide, "main.rs");
    let main_tab_id = main_tab["tab_id"].as_u64().expect("tab_id");
    let before_jump = cursor(&mcp, main_tab_id);

    const QUERY: &str = "shout_loud"; // ten keystrokes, exactly.
    assert_eq!(QUERY.len(), 10);
    let mark = open_search_popup(&ide, "ctrl+shift+e");
    accept_top_hit(&ide, mark, QUERY);

    let delivered = ide.events_since_of(mark, "search_results");
    assert!(
        delivered.len() <= 2,
        "{} result sets were delivered for {} keystrokes — either typing is \
         not debounced, or superseded generations are being rendered instead \
         of discarded: {delivered:?}",
        delivered.len(),
        QUERY.len()
    );

    let tab = ide.wait_for_event(mark, "a tab for the symbol's file", |e| {
        e["ev"] == "tab_added" && e["title"] == "greeting.rs"
    });
    let tab_id = tab["tab_id"].as_u64().expect("tab_id");

    // The expected line comes from the fixture, read here — not from the
    // index, which is the thing being checked.
    let declaration = fixture_text("tiny", "src/greeting.rs")
        .lines()
        .position(|line| line.contains(&format!("fn {QUERY}ly")))
        .expect("the fixture declares shout_loudly") as u32;
    e2e::wait_for("the caret to land on the declaration", || {
        (cursor(&mcp, tab_id).0 == declaration).then_some(())
    });

    // Back, and the pre-jump location is restored.
    let mark = ide.mark();
    ide.key("ctrl+alt+Left");
    e2e::wait_for("the caret to return to where the jump started", || {
        (cursor(&mcp, main_tab_id) == before_jump).then_some(())
    });
    let _ = mark;

    assert_eq!(ide.quit(), 0);
}

fn cursor(mcp: &Mcp, tab_id: u64) -> (u32, u32) {
    let position = mcp.call("get_cursor_position", json!({ "tab_id": tab_id }));
    (
        position["line"].as_u64().unwrap_or(0) as u32,
        position["column"].as_u64().unwrap_or(0) as u32,
    )
}

/// Rename through the preview dialog, cancelled and then applied.
///
/// Two dialog bug classes with no other net. Escape must leave every byte on
/// disk and in the buffer untouched — a cancel that applies anyway is
/// invisible to any test that does not look at the filesystem afterwards.
/// And the preview must list what the rule found, not a subset of it, so the
/// row count is compared against occurrences counted here from the fixture.
///
/// No language server is installed in `linux-builder`, so this exercises the
/// name-based fallback (`index_core::plan_index_rename`) — which is the path
/// the preview dialog exists for in the first place.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_rename_with_preview() {
    const SYMBOL: &str = "shout_loudly";
    let name = "e2e_rename_with_preview";
    let mut ide = Ide::launch(name, APP, fixture("tiny"));
    let mcp = ide.mcp();
    ide.wait_for_ev(Mark::start(), "project_opened");
    wait_for_index(&mcp);

    // What the rule should find, counted independently of the index.
    let occurrences: usize = ["src/main.rs", "src/greeting.rs"]
        .iter()
        .map(|f| fixture_text("tiny", f).matches(SYMBOL).count())
        .sum();
    assert_eq!(occurrences, 2, "the fixture's shape changed");

    let tab = open_file(&ide, "greeting.rs");
    let tab_id = tab["tab_id"].as_u64().expect("tab_id");
    let before = ide.project_snapshot();

    // Cancel first.
    let mark = ide.mark();
    start_rename(&ide, SYMBOL, "yell");
    let rows = ide.wait_for_ev(mark, "preview_rows");
    assert_eq!(
        rows["count"].as_u64(),
        Some(occurrences as u64),
        "the preview listed a subset of what the rename plan found"
    );
    ide.key("Escape");
    ide.wait_for_event(mark, "the preview to be cancelled", |e| {
        e["ev"] == "dialog_closed" && e["name"] == "refactor_preview" && e["accepted"] == false
    });
    ide.focus_main();
    // A reply to an editor-touching MCP call is proof the Qt event loop has
    // run everything queued before it — so if a cancelled preview had
    // applied anything, its marker would already be here.
    ide.sync(&mcp);
    assert!(
        ide.events_since_of(mark, "workspace_edit_applied")
            .is_empty(),
        "Escape on the preview applied the rename anyway"
    );
    assert_eq!(
        ide.project_snapshot(),
        before,
        "Escape changed a file on disk"
    );
    assert_eq!(
        buffer(&mcp, tab_id),
        fixture_text("tiny", "src/greeting.rs")
    );

    // Now apply.
    let mark = ide.mark();
    start_rename(&ide, SYMBOL, "yell");
    ide.wait_for_ev(mark, "preview_rows");
    ide.key("Return");
    let applied = ide.wait_for_ev(mark, "workspace_edit_applied");
    assert_eq!(applied["documents"].as_u64(), Some(2));
    ide.focus_main();

    e2e::wait_for("the closed file to be rewritten on disk", || {
        ide.read_project_file("src/main.rs")
            .contains("yell")
            .then_some(())
    });
    ide.wait_for_event(mark, "the open file's buffer to be spliced", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });

    // One Ctrl+Z for the whole rename in this buffer — that is what the
    // single `beginEditBlock` in `applyBufferEdits` buys, and it is the only
    // thing checking it.
    let mark = ide.mark();
    ide.key("ctrl+z");
    ide.wait_for_event(mark, "the buffer to return to its saved state", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.key("ctrl+s");
    ide.sync(&mcp);
    assert_eq!(
        ide.read_project_file("src/greeting.rs"),
        fixture_text("tiny", "src/greeting.rs"),
        "one Ctrl+Z did not undo the whole rename in the open buffer"
    );

    assert_eq!(ide.quit(), 0);
}

/// Put the caret inside `symbol`'s declaration and drive Shift+F6 up to the
/// preview dialog.
fn start_rename(ide: &Ide, symbol: &str, new_name: &str) {
    let mark = open_search_popup(ide, "ctrl+shift+o");
    accept_top_hit(ide, mark, symbol);
    // The jump lands at column 0; the declaration's name starts after
    // "pub fn ". One key past that is safely inside the word.
    for _ in 0..8 {
        ide.key("Right");
    }

    let main_window = ide.window().to_string();
    ide.key("shift+F6");
    // The name prompt is a QInputDialog — no marker of its own, so the
    // observable transition is the input focus moving off the main window.
    ide.wait_for_focus_change(&main_window);
    ide.type_text(new_name);
    ide.key("Return");
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
    let (x, y) = tab_centre(&second);
    ide.click_at(x, y, 3);
    ide.wait_for_event(mark, "the tab context menu", |e| {
        e["ev"] == "dialog_shown" && e["name"] == "tab_context_menu"
    });
    for _ in 0..3 {
        ide.key("Down"); // Close, Close Others, (separator), Split Vertical
    }
    ide.key("Return");

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

/// Two carets, one keystroke, one undo (F1-18).
///
/// The caret is walked to the middle of the fixture's first "world" by a
/// fixed count of `Right` presses from the start of the file — the same
/// keyboard-only positioning `start_rename` already uses, and deliberately
/// not Find-then-Escape: closing the find bar hands focus back to the
/// editor asynchronously, and a `Ctrl+D` sent before that lands collapses
/// the selection a moment earlier, which is exactly the kind of race this
/// suite exists to not paper over with a wait.
///
/// The first `Ctrl+D` selects the word the (collapsed) caret sits in; the
/// second finds the next occurrence and adds a caret for it —
/// `SelectionSet::add_next_occurrence`'s own two-step rule.
///
/// Asserted through a save each time, not `read_buffer` on its own: MCP
/// answers from `editor_core::Document`'s rope, which `save_tab` is what
/// refreshes (see the note at the top of this file) — the same reason
/// `e2e_open_project_edit_save` saves before every `buffer()` call.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_multi_caret_edit_is_one_undo() {
    let name = "e2e_multi_caret_edit_is_one_undo";
    let mut ide = Ide::launch(name, APP, fixture("tiny"));
    let mcp = ide.mcp();
    ide.wait_for_ev(Mark::start(), "project_opened");
    wait_for_index(&mcp);

    let original = fixture_text("tiny", "src/main.rs");
    assert_eq!(
        original.matches("world").count(),
        2,
        "the fixture's shape changed"
    );

    let tab = open_file(&ide, "main.rs");
    let tab_id = tab["tab_id"].as_u64().expect("tab_id");

    // Line 4 (0-indexed from Ctrl+Home): `    println!("{}", greeting::greet("world"));`.
    // Column 38 sits between the 'o' and the 'r' of "world" — inside the
    // word, which is what makes the first Ctrl+D select it rather than an
    // empty caret.
    ide.key("ctrl+Home");
    for _ in 0..3 {
        ide.key("Down");
    }
    for _ in 0..38 {
        ide.key("Right");
    }

    // First Ctrl+D selects "world"; the second adds the next occurrence.
    ide.key("ctrl+d");
    ide.key("ctrl+d");

    let mark = ide.mark();
    ide.type_text("!");
    ide.wait_for_event(mark, "the tab to go dirty", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the tab to go clean after saving", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);

    let edited = buffer(&mcp, tab_id);
    let mut expected = original.replacen("world", "!", 1);
    expected = expected.replacen("world", "!", 1);
    assert_eq!(
        edited, expected,
        "typing at two carets did not replace both selections with one character each"
    );

    // The guard: one undo restores both occurrences, because the whole
    // two-caret replacement crossed the seam as one `Vec<FfiTextEdit>` and
    // was spliced inside one `beginEditBlock` (ADR-0023).
    let mark = ide.mark();
    ide.key("ctrl+z");
    ide.wait_for_event(mark, "the tab to go dirty again", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the undone tab to be saved", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert_eq!(
        buffer(&mcp, tab_id),
        original,
        "one Ctrl+Z did not undo the two-caret edit"
    );

    assert_eq!(ide.quit(), 0);
}

/// Comment toggle and duplicate line, each its own undo (F1-18).
///
/// Reformat is not exercised here: no language server is installed in
/// `linux-builder` (F0-14's rust-analyzer image is a separate, opt-in
/// stage), so `code.reformat` has nothing to talk to in this environment.
/// Comment toggle and the line operations need no server at all — they are
/// `syntax-core`'s registry and a rope, which is exactly why they are
/// covered here instead.
///
/// Asserted through a save each time, not `read_buffer` on its own — see
/// the note on `e2e_multi_caret_edit_is_one_undo`.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_comment_toggle_and_duplicate_line() {
    let name = "e2e_comment_toggle_and_duplicate_line";
    let mut ide = Ide::launch(name, APP, fixture("tiny"));
    let mcp = ide.mcp();
    ide.wait_for_ev(Mark::start(), "project_opened");
    wait_for_index(&mcp);

    let original = fixture_text("tiny", "src/main.rs");
    let tab = open_file(&ide, "main.rs");
    let tab_id = tab["tab_id"].as_u64().expect("tab_id");
    ide.key("ctrl+Home");

    // Comment the first line, save, then undo and save again.
    let mark = ide.mark();
    ide.key("ctrl+slash");
    ide.wait_for_event(mark, "the tab to go dirty", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the tab to go clean after saving", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert!(
        buffer(&mcp, tab_id).starts_with("// mod greeting;"),
        "Ctrl+/ did not comment the first line"
    );

    let mark = ide.mark();
    ide.key("ctrl+z");
    ide.wait_for_event(mark, "the tab to go dirty again", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the undone tab to be saved", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert_eq!(
        buffer(&mcp, tab_id),
        original,
        "one Ctrl+Z did not undo the comment toggle"
    );

    // Duplicate the first line, save, then undo and save again.
    let mark = ide.mark();
    ide.key("ctrl+alt+d");
    ide.wait_for_event(mark, "the tab to go dirty", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the tab to go clean after saving", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert!(
        buffer(&mcp, tab_id).starts_with("mod greeting;\nmod greeting;\n"),
        "Ctrl+Alt+D did not duplicate the first line"
    );

    let mark = ide.mark();
    ide.key("ctrl+z");
    ide.wait_for_event(mark, "the tab to go dirty again", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the undone tab to be saved", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert_eq!(
        buffer(&mcp, tab_id),
        original,
        "one Ctrl+Z did not undo the duplicated line"
    );

    assert_eq!(ide.quit(), 0);
}

/// Where `stub_server` lands: Cargo places every workspace binary in the
/// same `target/<profile>/` directory as `app`'s own, and
/// `CARGO_BIN_EXE_stub_server` is not an option here — Cargo only sets a
/// binary's `CARGO_BIN_EXE_*` for integration tests of the crate that
/// declares it (`lsp-core`'s own, not `app`'s; see
/// `lsp-core/tests/stub_server_session.rs:15`).
fn stub_server_path() -> PathBuf {
    Path::new(APP).with_file_name("stub_server")
}

/// Route the `rust` language id at `lsp-core`'s X2 stub server rather than a
/// real `rust-analyzer` — not installed in this image, and the point of F2's
/// flows is the client's own behaviour, which the stub is built to exercise
/// deterministically by request line (`stub_server.rs`'s own doc comment).
///
/// Requires a restart: `LanguageService` resolves the server table once, on
/// `openProject`, from whatever `app-config` already has on disk — so the
/// override has to be written before the project opens, not after.
fn route_rust_at_stub(ide: &mut Ide) {
    assert_eq!(ide.quit(), 0);
    let mut settings = app_config::load(&ide.config_dir()).expect("settings just written");
    settings
        .language_servers
        .push(app_config::LanguageServerSetting {
            language_id: "rust".to_string(),
            command: Some(stub_server_path().to_string_lossy().into_owned()),
            ..Default::default()
        });
    app_config::save(&ide.config_dir(), &settings).expect("seeding the stub server override");
    ide.relaunch();
    ide.wait_for_ev(Mark::start(), "project_opened");
}

/// F2-8/F2-10: Alt+Enter merges the diagnostic-scoped and range-scoped
/// intentions at the caret into one grouped popup, and applying one goes
/// through the same one-`beginEditBlock` pending-refactor protocol every
/// other refactoring does.
///
/// The caret sits at (0,0), which is inside `stub_server`'s canned
/// diagnostic (line 0, columns 0-4) — so this also proves
/// `intentions::assemble` dedups the one action both the diagnostic-scoped
/// and the range-scoped request return, rather than listing it twice.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_alt_enter_applies_an_intention_in_one_undo() {
    let name = "e2e_alt_enter_applies_an_intention_in_one_undo";
    let mut ide = Ide::launch(name, APP, fixture("tiny"));
    ide.wait_for_ev(Mark::start(), "project_opened");
    route_rust_at_stub(&mut ide);

    let mcp = ide.mcp();
    wait_for_index(&mcp);
    let original = fixture_text("tiny", "src/main.rs");
    let tab = open_file(&ide, "main.rs");
    let tab_id = tab["tab_id"].as_u64().expect("tab_id");
    ide.key("ctrl+Home");

    let mark = ide.mark();
    ide.key("alt+Return");
    let shown = ide.wait_for_event(mark, "the intentions menu to open", |e| {
        e["ev"] == "dialog_shown" && e["name"] == "intentions_menu"
    });
    assert_eq!(
        shown["count"].as_u64(),
        Some(1),
        "the stub's one action at (0,0) should not be listed twice"
    );
    ide.key("Down");
    ide.key("Return");
    ide.wait_for_event(mark, "the intentions menu to accept", |e| {
        e["ev"] == "dialog_closed" && e["name"] == "intentions_menu" && e["accepted"] == true
    });
    ide.wait_for_ev(mark, "workspace_edit_applied");
    ide.focus_main();

    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the tab to go clean after saving", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert!(
        buffer(&mcp, tab_id).starts_with("extracted()"),
        "the intention's edit was not applied"
    );

    let mark = ide.mark();
    ide.key("ctrl+z");
    ide.wait_for_event(mark, "the tab to go dirty again", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the undone tab to be saved", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert_eq!(
        buffer(&mcp, tab_id),
        original,
        "one Ctrl+Z did not undo the intention"
    );

    assert_eq!(ide.quit(), 0);
}

/// F2-3: a resource operation ahead of its text edits, previewed and
/// applied through `RefactorPreviewDialog` exactly like a multi-file
/// rename — creating a file is never a same-file change, so
/// `EditPlan::touches_other_files` is true even though only one *other*
/// file is a resource operation, not a text edit.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_intention_creates_a_file_through_the_preview() {
    let name = "e2e_intention_creates_a_file_through_the_preview";
    let mut ide = Ide::launch(name, APP, fixture("tiny"));
    ide.wait_for_ev(Mark::start(), "project_opened");
    route_rust_at_stub(&mut ide);

    let mcp = ide.mcp();
    wait_for_index(&mcp);
    assert!(
        !fixture("tiny").join("src/extracted.rs").exists(),
        "the fixture's shape changed"
    );

    let tab = open_file(&ide, "main.rs");
    let tab_id = tab["tab_id"].as_u64().expect("tab_id");
    ide.key("ctrl+Home");
    for _ in 0..5 {
        ide.key("Down"); // Line 5 (0-indexed from Ctrl+Home): the closing `}`.
    }

    let mark = ide.mark();
    ide.key("alt+Return");
    ide.wait_for_event(mark, "the intentions menu to open", |e| {
        e["ev"] == "dialog_shown" && e["name"] == "intentions_menu"
    });
    ide.key("Down"); // A freshly-opened QMenu highlights nothing on its own.
    ide.key("Return"); // The stub offers exactly one action at this line.
    ide.wait_for_event(mark, "the intentions menu to accept", |e| {
        e["ev"] == "dialog_closed" && e["name"] == "intentions_menu" && e["accepted"] == true
    });

    let rows = ide.wait_for_ev(mark, "preview_rows");
    assert_eq!(
        rows["files"].as_u64(),
        Some(2),
        "the preview should list the created file and the edited one"
    );
    ide.key("Return");
    let applied = ide.wait_for_ev(mark, "workspace_edit_applied");
    assert_eq!(applied["documents"].as_u64(), Some(2));
    ide.focus_main();

    ide.wait_for_event(mark, "the open file's buffer to be spliced", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == true
    });
    ide.key("ctrl+s");
    ide.wait_for_event(mark, "the tab to go clean after saving", |e| {
        e["ev"] == "tab_dirty" && e["tab_id"].as_u64() == Some(tab_id) && e["dirty"] == false
    });
    ide.sync(&mcp);
    assert!(
        buffer(&mcp, tab_id).ends_with("moved\n") || buffer(&mcp, tab_id).contains("moved"),
        "the original file's edit was not applied"
    );

    e2e::wait_for("the created file to appear on disk", || {
        ide.read_project_file("src/extracted.rs")
            .contains("moved here")
            .then_some(())
    });

    assert_eq!(ide.quit(), 0);
}

/// The centre of a tab's label on screen, from its `tab_added` marker.
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
