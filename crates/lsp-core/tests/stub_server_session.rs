//! End-to-end tests of `LspManager` against the X2 stub server.
//!
//! The stub's path comes from `CARGO_BIN_EXE_stub_server`, which Cargo sets
//! for integration tests, so nothing here guesses at the target directory.

use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use lsp_core::catalog::ServerConfig;
use lsp_core::manager::{LspError, LspEvent, LspManager};
use serde_json::json;

const STUB: &str = env!("CARGO_BIN_EXE_stub_server");
const LANG: &str = "stub";

fn config(command: &str, args: &[&str]) -> ServerConfig {
    ServerConfig {
        language_id: LANG.into(),
        name: "stub".into(),
        command: command.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        enabled: true,
    }
}

fn stub_config() -> ServerConfig {
    config(STUB, &[])
}

/// The stub dies mid-session on the first `didOpen`. The env var is passed
/// through `env(1)` so the test process' own environment stays untouched —
/// integration tests share one process.
fn dying_stub_config() -> ServerConfig {
    config("env", &["STUB_LSP_DIE_ON_DIDOPEN=1", STUB])
}

/// Drain events until one matches, or fail. Non-matching events are skipped:
/// a server may legitimately emit log notifications we don't care about.
fn wait_for<T>(
    rx: &Receiver<LspEvent>,
    what: &str,
    mut pick: impl FnMut(&LspEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = rx
            .recv_timeout(remaining)
            .unwrap_or_else(|e| panic!("waiting for {what}: {e}"));
        if let Some(value) = pick(&event) {
            return value;
        }
    }
}

#[test]
fn initialize_and_shutdown_lifecycle() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    assert!(manager.is_running(LANG));

    let restarts = wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { restarts, .. } => Some(*restarts),
        _ => None,
    });
    assert_eq!(restarts, 0, "a first launch is not a restart");

    manager.stop(LANG);
    assert!(!manager.is_running(LANG));
    // The server is gone, so requests fail instead of hanging.
    assert!(matches!(
        manager.request(LANG, "stub/echo", json!({})),
        Err(LspError::NoServer(_))
    ));
}

#[test]
fn a_missing_executable_fails_the_start() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    let err = manager
        .start(&config("definitely-not-a-language-server", &[]))
        .expect_err("a missing binary cannot start");
    assert!(matches!(err, LspError::Spawn { .. }), "got {err:?}");
    assert!(!manager.is_running(LANG));
}

#[test]
fn did_open_publishes_diagnostics_as_an_event() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    manager
        .did_open("file:///workspace/a.stub", LANG, "hello\n")
        .expect("didOpen is sent");
    assert_eq!(
        manager.document_version("file:///workspace/a.stub"),
        Some(1)
    );

    let (uri, version, message) = wait_for(&rx, "diagnostics", |e| match e {
        LspEvent::Diagnostics {
            uri,
            version,
            diagnostics,
            ..
        } => Some((uri.clone(), *version, diagnostics[0].message.clone())),
        _ => None,
    });
    assert_eq!(uri, "file:///workspace/a.stub");
    assert_eq!(version, Some(1));
    assert_eq!(message, "canned diagnostic");

    // The manager owns the version counter, not the caller.
    assert_eq!(
        manager
            .did_change("file:///workspace/a.stub", "bye\n")
            .unwrap(),
        2
    );
    assert_eq!(
        manager.document_version("file:///workspace/a.stub"),
        Some(2)
    );
    manager.did_close("file:///workspace/a.stub").unwrap();
    assert_eq!(manager.document_version("file:///workspace/a.stub"), None);
}

#[test]
fn responses_are_matched_to_their_own_requests() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    // The slow request is issued first and answered last; each caller must
    // still get its own payload back.
    std::thread::scope(|scope| {
        let slow = scope
            .spawn(|| manager.request(LANG, "stub/echo", json!({"tag": "slow", "delay_ms": 400})));
        std::thread::sleep(Duration::from_millis(50));
        let fast = manager
            .request(LANG, "stub/echo", json!({"tag": "fast"}))
            .expect("fast echo answers");
        assert_eq!(fast["tag"], "fast");
        let slow = slow.join().unwrap().expect("slow echo answers");
        assert_eq!(slow["tag"], "slow");
    });
}

#[test]
fn an_unimplemented_request_returns_the_servers_error() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    // Deliberately a method the stub has no arm for at all — `rename` and
    // `codeAction` are implemented now (RF3), so they no longer test this.
    let err = manager
        .request(LANG, "textDocument/formatting", json!({}))
        .expect_err("the stub implements no formatting");
    assert!(
        matches!(err, LspError::Response { code: -32601, .. }),
        "got {err:?}"
    );
}

#[test]
fn a_slow_request_times_out_and_is_cancelled() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    let err = manager
        .request_with_timeout(
            LANG,
            "stub/echo",
            json!({"delay_ms": 5000}),
            Duration::from_millis(100),
        )
        .expect_err("the response arrives far too late");
    assert!(matches!(err, LspError::Timeout { .. }), "got {err:?}");

    // The late response must not be mistaken for the next request's.
    let next = manager
        .request(LANG, "stub/echo", json!({"tag": "next"}))
        .expect("the session survives a cancelled request");
    assert_eq!(next["tag"], "next");
}

#[test]
fn a_server_to_client_request_is_answered_rather_than_ignored() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    let answer = manager
        .request(LANG, "stub/askClient", json!({}))
        .expect("the stub's own request does not deadlock us");
    assert_eq!(answer, "asked");
}

#[test]
fn a_server_that_dies_mid_session_is_respawned() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&dying_stub_config()).expect("stub starts");
    wait_for(&rx, "first ServerReady", |e| match e {
        LspEvent::ServerReady { restarts: 0, .. } => Some(()),
        _ => None,
    });

    manager
        .did_open("file:///workspace/a.stub", LANG, "boom\n")
        .expect("didOpen is sent");

    // It publishes its diagnostic, then exits(1) without an `exit` request.
    wait_for(&rx, "diagnostics", |e| match e {
        LspEvent::Diagnostics { .. } => Some(()),
        _ => None,
    });
    let retry_in = wait_for(&rx, "ServerExited", |e| match e {
        LspEvent::ServerExited {
            restarts: 1,
            retry_in,
            ..
        } => Some(*retry_in),
        _ => None,
    });
    assert!(
        retry_in >= Duration::from_millis(200),
        "backoff starts at 200ms"
    );

    wait_for(&rx, "ServerReady after respawn", |e| match e {
        LspEvent::ServerReady { restarts: 1, .. } => Some(()),
        _ => None,
    });
    assert!(manager.is_running(LANG));

    // The respawned server is a working session, not just a live process.
    let echoed = manager
        .request(LANG, "stub/echo", json!({"tag": "after-respawn"}))
        .expect("the respawned server answers");
    assert_eq!(echoed["tag"], "after-respawn");
}

/// The whole L2 path minus Qt: a real child server publishes diagnostics,
/// the event lands in the store the adapter keeps, and the store yields the
/// rows the Problems panel renders.
#[test]
fn published_diagnostics_become_problem_rows() {
    use lsp_core::diagnostics::{DiagnosticCounts, DiagnosticStore, Severity};
    use lsp_core::uri_from_path;

    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    let uri = uri_from_path("/workspace/a b.stub");
    manager.did_open(&uri, LANG, "hello\n").expect("didOpen");

    let mut store = DiagnosticStore::new();
    let (published_uri, diagnostics) = wait_for(&rx, "diagnostics", |e| match e {
        LspEvent::Diagnostics {
            uri, diagnostics, ..
        } => Some((uri.clone(), diagnostics.clone())),
        _ => None,
    });
    store.replace(&published_uri, diagnostics);

    let rows = store.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, "/workspace/a b.stub");
    assert_eq!(rows[0].line, 1);
    assert_eq!(rows[0].column, 0);
    assert_eq!(rows[0].severity, Severity::Error);
    assert_eq!(rows[0].message, "canned diagnostic");
    assert_eq!(rows[0].source, "stub_server");
    assert_eq!(
        store.counts(),
        DiagnosticCounts {
            errors: 1,
            ..DiagnosticCounts::default()
        }
    );

    // Closing the document is what drops its rows from the panel.
    manager.did_close(&uri).expect("didClose");
    store.remove(&uri);
    assert!(store.rows().is_empty());
}

/// L3: the manager reduces every hover shape the stub can send to one text.
#[test]
fn hover_is_parsed_from_every_response_shape() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    let uri = "file:///workspace/main.rs";
    manager
        .did_open(uri, LANG, "fn main() {}")
        .expect("didOpen");

    // Line 0: MarkupContent markdown, rendered for a Qt tooltip.
    let markup = manager
        .hover(uri, 0, 0)
        .expect("hover")
        .expect("some hover");
    assert!(markup.markdown);
    assert_eq!(
        lsp_core::to_tooltip_html(&markup),
        "<pre>fn main()</pre>The <b>entry</b> point."
    );

    // Line 1: the deprecated {language, value} MarkedString.
    let marked = manager
        .hover(uri, 1, 0)
        .expect("hover")
        .expect("some hover");
    assert_eq!(marked.value, "```rust\nfn main()\n```");

    // Line 2: an array of MarkedStrings.
    let array = manager
        .hover(uri, 2, 0)
        .expect("hover")
        .expect("some hover");
    assert!(array.value.starts_with("plain hover"));

    // Anywhere else: a null result is "nothing here", not an error.
    assert_eq!(manager.hover(uri, 7, 0).expect("hover"), None);

    manager.stop(LANG);
}

/// L4: definitions arrive as a Location, a Location array or a LocationLink
/// array, and the precedence rule sends everything else to the index.
#[test]
fn definitions_are_parsed_and_fall_back_to_the_index() {
    use lsp_core::{definition_outcome, DefinitionOutcome};

    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    let uri = "file:///workspace/main.rs";
    manager
        .did_open(uri, LANG, "fn main() {}")
        .expect("didOpen");

    let single = manager.definition(uri, 0, 0).expect("definition");
    assert_eq!(single.len(), 1);
    assert_eq!((single[0].line, single[0].column), (4, 4));

    let many = manager.definition(uri, 0, 1).expect("definition");
    assert_eq!(
        many.len(),
        2,
        "ambiguity is the servers answer, not an error"
    );

    let link = manager.definition(uri, 0, 2).expect("definition");
    assert_eq!((link[0].line, link[0].column), (4, 4));

    // A server that knows nothing hands the gesture to ADR-0011's index.
    let nothing = manager.definition(uri, 0, 9).expect("definition");
    assert_eq!(
        definition_outcome(Some(Ok(nothing))),
        DefinitionOutcome::Index
    );
    assert_eq!(
        definition_outcome(Some(Ok(single.clone()))),
        DefinitionOutcome::Lsp(single)
    );

    manager.stop(LANG);

    // The server is gone: the index answers rather than the gesture failing.
    assert_eq!(
        definition_outcome(Some(manager.definition(uri, 0, 0))),
        DefinitionOutcome::Index
    );
}

/// L5: both response shapes, the insertion precedence, and the server's
/// ordering and filtering, driven through a real child process.
#[test]
fn completions_are_parsed_ordered_and_filtered() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    let uri = "file:///workspace/main.rs";
    manager
        .did_open(uri, LANG, "fn main() {}")
        .expect("didOpen");

    // The trigger characters reach the UI through ServerReady, not a getter.
    let triggers = wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady {
            trigger_characters, ..
        } => Some(trigger_characters.clone()),
        _ => None,
    });
    assert_eq!(triggers, [".", ":"]);
    assert!(lsp_core::should_request(
        &triggers,
        "self.",
        false,
        &lsp_core::CompletionTracker::default(),
    ));

    // Line 0: a bare CompletionItem[], ordered by sortText, not by label.
    let plain = manager.completion(uri, 0, 0).expect("completion");
    assert!(!plain.is_incomplete);
    let ordered: Vec<String> = lsp_core::filter_completions(&plain.items, "p")
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert_eq!(ordered, ["pop", "push"], "sortText wins over the label");
    // filterText, not the label, decides what a prefix matches.
    let filtered = lsp_core::filter_completions(&plain.items, "allo");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].label, "#[allow]");
    assert_eq!(filtered[0].insert, "#[allow(dead_code)]", "insertText");

    // Line 1: a CompletionList, incomplete, with a snippet and a textEdit.
    let list = manager.completion(uri, 1, 0).expect("completion");
    assert!(list.is_incomplete, "ask again as the word grows");
    let snippet = &list.items[0];
    assert_eq!(
        snippet.insert, "map(f)",
        "no placeholders left in the buffer"
    );
    let edit = &list.items[1];
    assert_eq!(edit.insert, "max()");
    assert_eq!(
        edit.range
            .expect("a textEdit names its range")
            .start_character,
        2
    );

    // Anywhere else: an empty list, which is not an error.
    assert!(manager
        .completion(uri, 7, 0)
        .expect("completion")
        .items
        .is_empty());

    manager.stop(LANG);
}

/// The refactoring requests RF3 taught the stub. These go through the
/// generic `request` because the typed `LspManager` methods land with RF6 —
/// what is being proven here is that the fixture answers each shape, so the
/// parsers built on top of it can be tested offline.
#[test]
fn code_actions_are_answered_in_every_shape_the_parser_must_handle() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    let request = |line: u64| {
        manager
            .request(
                LANG,
                "textDocument/codeAction",
                json!({
                    "textDocument": {"uri": "file:///workspace/main.rs"},
                    "range": {"start": {"line": line, "character": 0},
                              "end": {"line": line, "character": 4}},
                    "context": {"diagnostics": []},
                }),
            )
            .expect("the stub answers")
    };

    let with_edit = request(0);
    assert_eq!(with_edit[0]["kind"], "refactor.extract.function");
    assert!(with_edit[0]["edit"]["documentChanges"].is_array());

    let with_command = request(1);
    assert_eq!(
        with_command[0]["command"], "stub.plain",
        "a bare Command has its command as a string",
    );
    assert_eq!(
        with_command[1]["command"]["command"], "stub.applyEdit",
        "a CodeAction has its command as an object",
    );

    assert!(
        request(2)[0]["edit"].is_null(),
        "an unresolved item carries neither an edit nor a command",
    );
    assert_eq!(
        request(3)[0]["disabled"]["reason"],
        "selection is not an expression"
    );
    assert!(request(9).is_null(), "no actions here");
}

#[test]
fn an_unresolved_code_action_gains_its_edit_from_resolve() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    let unresolved = json!({"title": "Needs resolving", "kind": "refactor.inline",
                            "data": {"token": 42}});
    let resolved = manager
        .request(LANG, "codeAction/resolve", unresolved)
        .expect("the stub resolves it");

    assert_eq!(
        resolved["title"], "Needs resolving",
        "the item comes back whole"
    );
    assert!(resolved["edit"]["changes"].is_object());
}

#[test]
fn prepare_rename_and_rename_answer_in_every_shape() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    let at = |method: &'static str, line: u64, extra: serde_json::Value| {
        let mut params = json!({
            "textDocument": {"uri": "file:///workspace/main.rs"},
            "position": {"line": line, "character": 5},
        });
        if let Some(object) = extra.as_object() {
            for (key, value) in object {
                params[key] = value.clone();
            }
        }
        manager.request(LANG, method, params)
    };

    assert!(
        at("textDocument/prepareRename", 0, json!({})).unwrap()["start"].is_object(),
        "line 0 answers with a bare Range",
    );
    assert_eq!(
        at("textDocument/prepareRename", 1, json!({})).unwrap()["placeholder"],
        "old_name",
    );
    assert_eq!(
        at("textDocument/prepareRename", 2, json!({})).unwrap()["defaultBehavior"],
        true,
    );
    assert!(at("textDocument/prepareRename", 9, json!({}))
        .unwrap()
        .is_null());

    let rename = json!({"newName": "renamed"});
    let versioned = at("textDocument/rename", 0, rename.clone()).unwrap();
    assert_eq!(
        versioned["documentChanges"][0]["textDocument"]["version"],
        1
    );
    assert_eq!(
        versioned["documentChanges"][0]["edits"][0]["newText"],
        "renamed",
    );

    let legacy = at("textDocument/rename", 1, rename.clone()).unwrap();
    assert!(legacy["changes"]["file:///workspace/main.rs"].is_array());

    assert!(at("textDocument/rename", 2, rename.clone())
        .unwrap()
        .is_null());
    assert!(
        at("textDocument/rename", 9, rename).is_err(),
        "an un-renameable element answers with an error, not a null",
    );
}

/// The command-driven refactoring shape: the server asks *us* to apply an
/// edit in the middle of an `executeCommand` and blocks on the answer.
///
/// With no refactoring in flight the request is refused outright — a server
/// rewriting the user's files unprompted is what that rule exists to stop —
/// and, crucially, refused without stalling anything.
#[test]
fn an_unsolicited_apply_edit_is_refused_without_troubling_the_editor() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    assert!(!manager.refactor_active());

    let answer = manager
        .request(
            LANG,
            "workspace/executeCommand",
            json!({"command": "stub.applyEdit",
                   "arguments": ["file:///workspace/main.rs"]}),
        )
        .expect("the command completes without deadlocking");

    assert_eq!(
        answer["clientApplied"], false,
        "nobody asked for this edit, so the server is told it was not applied",
    );
    assert!(
        !matches!(rx.try_recv(), Ok(LspEvent::ApplyEdit { .. })),
        "an unsolicited edit must never reach the UI",
    );

    let echo = manager
        .request(LANG, "stub/echo", json!({"tag": "after"}))
        .expect("the session survives the round trip");
    assert_eq!(echo["tag"], "after");
}

/// The whole handshake, end to end: a refactoring is in flight, the server
/// asks for an edit mid-command, the editor claims it, and the command
/// completes with the server told it was applied.
#[test]
fn an_edit_asked_for_during_a_refactoring_reaches_the_editor_and_is_applied() {
    let (manager, rx) = LspManager::new("file:///workspace");
    let manager = Arc::new(manager);
    manager.start(&stub_config()).expect("stub starts");

    // The guard is what makes the server's request legitimate. It is held
    // across the command, exactly as a refactoring gesture would hold it.
    let session = manager.begin_refactor();
    assert!(manager.refactor_active());

    let commanded = Arc::clone(&manager);
    let command = thread::spawn(move || {
        commanded.request(
            LANG,
            "workspace/executeCommand",
            json!({"command": "stub.applyEdit",
                   "arguments": ["file:///workspace/main.rs"]}),
        )
    });

    // The editor's side: the edit arrives as an event, with the server still
    // blocked on the gate.
    let (label, edit, gate) = wait_for(&rx, "the applyEdit request", |event| match event {
        LspEvent::ApplyEdit {
            label, edit, gate, ..
        } => Some((label.clone(), edit.clone(), gate.clone())),
        _ => None,
    });
    assert_eq!(label.as_deref(), Some("Extract class"));
    assert_eq!(
        edit["documentChanges"][0]["textDocument"]["uri"], "file:///workspace/main.rs",
        "the raw WorkspaceEdit is handed over for the UI side to parse",
    );
    // The invariant this whole design exists for: the server's read thread
    // is not blocked while the edit waits, so everything else keeps working.
    // A `didOpen` sent now must still come back with its diagnostics.
    manager
        .did_open("file:///workspace/other.rs", LANG, "fn other() {}\n")
        .expect("the session is still usable mid-handshake");
    wait_for(
        &rx,
        "diagnostics while an edit is pending",
        |event| match event {
            LspEvent::Diagnostics { uri, .. } if uri.ends_with("other.rs") => Some(()),
            _ => None,
        },
    );

    assert!(gate.claim(), "the editor takes the edit");

    let answer = command.join().unwrap().expect("the command completes");
    assert_eq!(
        answer["clientApplied"], true,
        "the server hears that its edit was applied",
    );
    drop(session);
    assert!(!manager.refactor_active());
}

/// The gate is bounded: an editor that never answers must not park the
/// server's thread for ever, and must not then be allowed to apply anything.
#[test]
fn the_gate_refuses_a_late_claim_so_the_answer_is_never_a_lie() {
    let (gate, gate_rx) = lsp_core::ApplyEditGate::new();
    assert!(gate.close(), "the wait gave up first");
    assert!(
        !gate.claim(),
        "a UI arriving after the timeout must not apply text the server was told about",
    );
    drop(gate_rx);
}

/// RF6: the typed refactoring requests, driven the way the UI will drive
/// them. The stub varies its answers by position, so one test walks the
/// shapes each parser has to survive.
#[test]
fn the_typed_rename_requests_reach_the_server_and_parse() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    let uri = "file:///workspace/main.rs";
    manager
        .did_open(uri, LANG, "fn old_name() {}\n")
        .expect("didOpen");

    let prepared = manager
        .prepare_rename(uri, 1, 5)
        .expect("the stub answers")
        .expect("line 1 is renameable");
    assert_eq!(prepared.placeholder.as_deref(), Some("old_name"));
    assert!(
        manager.prepare_rename(uri, 9, 0).unwrap().is_none(),
        "line 9 cannot be renamed",
    );

    let documents = manager
        .rename(uri, 0, 5, "new_name")
        .expect("rename answers");
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].version, Some(1));
    assert_eq!(documents[0].edits[0].new_text, "new_name");

    // The legacy `changes` shape parses to the same thing, minus the version.
    let legacy = manager
        .rename(uri, 1, 5, "new_name")
        .expect("rename answers");
    assert_eq!(legacy[0].version, None);
    assert_eq!(legacy[0].path, "/workspace/main.rs");

    assert!(
        manager.rename(uri, 2, 5, "new_name").unwrap().is_empty(),
        "a null answer is no edits, which the caller resolves to the index",
    );
}

#[test]
fn the_typed_code_action_requests_reach_the_server_and_parse() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    let uri = "file:///workspace/main.rs";
    manager
        .did_open(uri, LANG, "fn main() {}\n")
        .expect("didOpen");

    let extract = manager
        .code_action(uri, (0, 0), (0, 4), &["refactor.extract"])
        .expect("the stub answers");
    assert_eq!(extract.len(), 1);
    assert_eq!(
        extract[0].kind.as_deref(),
        Some("refactor.extract.function")
    );
    assert!(!extract[0].needs_resolve());

    let unresolved = manager
        .code_action(uri, (2, 0), (2, 4), &[])
        .expect("the stub answers");
    assert!(unresolved[0].needs_resolve());
    let resolved = manager
        .resolve_code_action(LANG, &unresolved[0])
        .expect("resolve answers");
    assert!(
        !resolved[0].needs_resolve(),
        "resolving fills in the edit the server withheld",
    );

    assert!(
        manager
            .code_action(uri, (9, 0), (9, 4), &[])
            .unwrap()
            .is_empty(),
        "no actions here",
    );
}

/// The capabilities are what a server reads to decide whether to offer these
/// features at all, so they are asserted where they actually land: in the
/// `initialize` the stub received.
#[test]
fn the_refactoring_capabilities_are_advertised() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    let capabilities = manager
        .request(LANG, "stub/clientCapabilities", json!({}))
        .expect("the stub hands back what we sent");

    assert_eq!(capabilities["workspace"]["applyEdit"], true);
    assert_eq!(
        capabilities["workspace"]["workspaceEdit"]["documentChanges"], true,
        "versions are what let a stale edit be caught before it is applied",
    );
    assert_eq!(
        capabilities["workspace"]["workspaceEdit"]["resourceOperations"],
        json!(["create", "rename", "delete"]),
        "advertised because app_core::apply_file_ops performs them (ADR-0026); \
         without these, every extract-to-new-file refactoring is refused whole",
    );
    assert_eq!(
        capabilities["workspace"]["workspaceEdit"]["failureHandling"],
        "abort",
    );
    assert!(capabilities["workspace"]["executeCommand"].is_object());
    assert_eq!(
        capabilities["textDocument"]["rename"]["prepareSupport"],
        true
    );
    assert_eq!(
        capabilities["textDocument"]["codeAction"]["resolveSupport"]["properties"],
        json!(["edit"]),
    );
    assert_eq!(
        capabilities["textDocument"]["codeAction"]["disabledSupport"], true,
        "a disabled action is shown greyed, so servers may send them",
    );
    assert!(
        capabilities["textDocument"]["codeAction"]["codeActionLiteralSupport"]["codeActionKind"]
            ["valueSet"]
            .as_array()
            .expect("a kind list")
            .contains(&json!("refactor.extract"))
    );
}

// ---------------------------------------------------------------------------
// F1: reformat. The stub picks its behaviour from the requested tab size, so
// all four replies a real server might give can be exercised without four
// documents — and without waiting for a real formatter to disagree with us.
// ---------------------------------------------------------------------------

fn formatting_options(tab_size: u32) -> lsp_core::formatting::FormattingOptions {
    lsp_core::formatting::FormattingOptions {
        tab_size,
        ..lsp_core::formatting::FormattingOptions::default()
    }
}

#[test]
fn formatting_returns_the_servers_edits() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).unwrap();
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });
    manager
        .did_open("file:///workspace/a.stub", LANG, "text")
        .unwrap();

    let outcome = manager
        .format("file:///workspace/a.stub", &formatting_options(2))
        .unwrap();
    match outcome {
        lsp_core::formatting::FormattingOutcome::Edits(edits) => {
            assert_eq!(edits.len(), 1);
            assert_eq!(edits[0].new_text, "  ");
        }
        other => panic!("expected edits, got {other:?}"),
    }
    manager.stop_all();
}

// An empty list and a null both mean "already formatted", which is a
// different message to the user than "this language has no formatter".
#[test]
fn an_empty_or_null_reply_is_already_formatted_not_unsupported() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).unwrap();
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });
    manager
        .did_open("file:///workspace/a.stub", LANG, "text")
        .unwrap();

    for tab_size in [4, 8] {
        assert_eq!(
            manager
                .format("file:///workspace/a.stub", &formatting_options(tab_size))
                .unwrap(),
            lsp_core::formatting::FormattingOutcome::AlreadyFormatted,
            "tab size {tab_size}"
        );
    }
    manager.stop_all();
}

// A server that does not implement formatting answers MethodNotFound rather
// than an empty list. That must surface as Unsupported, not as an error the
// user sees as a failure.
#[test]
fn method_not_found_becomes_unsupported_rather_than_an_error() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).unwrap();
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });
    manager
        .did_open("file:///workspace/a.stub", LANG, "text")
        .unwrap();

    assert_eq!(
        manager
            .format("file:///workspace/a.stub", &formatting_options(99))
            .unwrap(),
        lsp_core::formatting::FormattingOutcome::Unsupported
    );
    manager.stop_all();
}

// The stub never implements rangeFormatting, so this exercises the fall back
// to whole-document formatting: reformatting more than was asked beats
// reformatting nothing, and the preview shows what changed either way.
#[test]
fn range_formatting_falls_back_to_whole_document() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).unwrap();
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });
    manager
        .did_open("file:///workspace/a.stub", LANG, "text")
        .unwrap();

    // tabSize 2 makes the whole-document path return edits, so seeing them
    // here proves the fall back happened.
    let outcome = manager
        .format_range(
            "file:///workspace/a.stub",
            (0, 0),
            (0, 4),
            &formatting_options(2),
        )
        .unwrap();
    match outcome {
        lsp_core::formatting::FormattingOutcome::Edits(edits) => {
            assert_eq!(edits.len(), 1)
        }
        other => panic!("expected the whole-document fall back, got {other:?}"),
    }
    manager.stop_all();
}
