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
        settings_section: None,
        settings: serde_json::Value::Null,
        source: lsp_core::catalog::ServerSource::Builtin,
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

/// C6: a stub configured with a `workspace/configuration` section and
/// starting settings, the way the `csharp` plugin's `ServerConfig` is.
fn stub_config_with_settings() -> ServerConfig {
    ServerConfig {
        settings_section: Some("csharp".into()),
        settings: json!({"analyzersEnabled": true}),
        ..stub_config()
    }
}

/// C7: the stub advertises `completionProvider.resolveProvider: true`, the
/// way csharp-ls does.
fn stub_config_with_completion_resolve() -> ServerConfig {
    config("env", &["STUB_LSP_COMPLETION_RESOLVE=1", STUB])
}

/// C9: the stub advertises `semanticTokensProvider` statically in
/// `initialize`'s result, the way rust-analyzer does.
fn stub_config_with_semantic_tokens_static() -> ServerConfig {
    config("env", &["STUB_LSP_SEMANTIC_TOKENS_STATIC=1", STUB])
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
    // F2-9's signature-help triggers arrive on the same event, read from the
    // same `initialize` result.
    let (triggers, signature_triggers) = wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady {
            trigger_characters,
            signature_triggers,
            ..
        } => Some((trigger_characters.clone(), signature_triggers.clone())),
        _ => None,
    });
    assert_eq!(triggers, [".", ":"]);
    assert!(lsp_core::should_request(
        &triggers,
        "self.",
        false,
        &lsp_core::CompletionTracker::default(),
    ));
    assert!(signature_triggers.supported);
    assert_eq!(signature_triggers.trigger, ["(", ","]);
    assert_eq!(signature_triggers.retrigger, [")"]);

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
        "advertised because app_core::apply_file_ops performs them (F2); \
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

// -- F2: the Alt+Enter surfaces -------------------------------------------

/// A started stub with one open document, which is what every F2 request
/// needs before it can resolve the document's language.
fn session_with_open_document() -> (LspManager, Receiver<LspEvent>, &'static str) {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    let uri = "file:///workspace/main.rs";
    manager
        .did_open(uri, LANG, "fn main() {}")
        .expect("didOpen");
    (manager, rx, uri)
}

#[test]
fn signature_help_is_parsed_from_every_response_shape() {
    let (manager, _rx, uri) = session_with_open_document();

    // Line 0: `[start, end]` parameter labels.
    let offsets = manager
        .signature_help(uri, 0, 0)
        .expect("signatureHelp")
        .expect("a signature");
    assert_eq!(
        offsets.resolved_signature().unwrap().parameters[1].label,
        "value: T"
    );
    assert_eq!(offsets.resolved_parameter(), Some(1));

    // Line 1: substring parameter labels, resolved to the same shape.
    let substrings = manager
        .signature_help(uri, 1, 0)
        .expect("signatureHelp")
        .expect("a signature");
    assert_eq!(
        substrings.resolved_signature().unwrap().parameters[0].range,
        Some((10, 22)),
        "a substring label must reach the view as offsets like any other",
    );

    // Line 2: an overload set whose signature-level index disagrees with the
    // response-level one.
    let overloads = manager
        .signature_help(uri, 2, 0)
        .expect("signatureHelp")
        .expect("a signature");
    assert_eq!(overloads.resolved_signature().unwrap().label, "f(a)");
    assert_eq!(overloads.resolved_parameter(), Some(0));

    // Line 3: a malformed reply — `signatures` is an object. No panic, no
    // hint.
    assert!(manager
        .signature_help(uri, 3, 0)
        .expect("signatureHelp")
        .is_none());

    // Line 4: an active parameter past the signature's own arity.
    let past_end = manager
        .signature_help(uri, 4, 0)
        .expect("signatureHelp")
        .expect("a signature");
    assert_eq!(past_end.resolved_parameter(), None);

    // Anywhere else: null is "no hint here", not an error.
    assert!(manager
        .signature_help(uri, 9, 0)
        .expect("signatureHelp")
        .is_none());

    manager.stop(LANG);
}

#[test]
fn document_highlights_keep_the_kind_of_each_occurrence() {
    use lsp_core::HighlightKind;
    let (manager, _rx, uri) = session_with_open_document();

    let highlights = manager.document_highlights(uri, 0, 0).expect("highlights");
    assert_eq!(
        highlights.iter().map(|h| h.kind).collect::<Vec<_>>(),
        vec![
            HighlightKind::Text,
            HighlightKind::Read,
            HighlightKind::Write
        ],
        "a write and a read want different emphasis, so the kind survives",
    );

    assert_eq!(
        manager.document_highlights(uri, 1, 0).expect("highlights")[0].kind,
        HighlightKind::Text,
        "a kind we do not know still gets painted, neutrally",
    );
    assert!(
        manager
            .document_highlights(uri, 3, 0)
            .expect("highlights")
            .is_empty(),
        "a malformed entry is dropped, not panicked on",
    );
    assert!(manager
        .document_highlights(uri, 9, 0)
        .expect("highlights")
        .is_empty());

    manager.stop(LANG);
}

#[test]
fn inlay_hints_are_asked_for_a_viewport_and_not_for_the_whole_file() {
    use lsp_core::InlayHintKind;
    let (manager, _rx, uri) = session_with_open_document();

    // The stub echoes the range it was given, so the request's own line
    // range is observable from the reply.
    let hints = manager.inlay_hints(uri, 120, 168).expect("inlayHint");
    assert_eq!(hints.len(), 2);
    assert_eq!(hints[0].line, 120);
    assert_eq!(hints[0].label, ": i32");
    assert_eq!(hints[0].kind, InlayHintKind::Type);
    assert!(hints[0].padding_left);
    assert_eq!(
        hints[1].line, 168,
        "the last visible line is included and no line beyond it is asked for",
    );
    assert_eq!(hints[1].label, "value:", "label parts arrive concatenated");
    assert_eq!(hints[1].kind, InlayHintKind::Parameter);

    assert!(
        manager
            .inlay_hints(uri, 900, 950)
            .expect("inlayHint")
            .is_empty(),
        "a hint whose label is the wrong type is dropped, not panicked on",
    );

    manager.stop(LANG);
}

#[test]
fn intentions_merge_both_requests_into_one_grouped_list() {
    use lsp_core::IntentionGroup;
    let (manager, _rx, uri) = session_with_open_document();

    let diagnostic = json!({
        "range": {"start": {"line": 4, "character": 0},
                  "end": {"line": 4, "character": 7}},
        "severity": 1,
        "code": "E0433",
        "message": "failed to resolve: use of undeclared type `HashMap`",
    });
    let intentions = manager
        .intentions(uri, (4, 0), (4, 0), std::slice::from_ref(&diagnostic))
        .expect("intentions");

    assert_eq!(
        intentions.iter().map(|i| i.title()).collect::<Vec<_>>(),
        vec![
            "Import `HashMap`",
            "Extract into function",
            "Organize imports"
        ],
        "quick fix, then refactoring, then source — and each exactly once",
    );
    assert_eq!(intentions[0].group, IntentionGroup::QuickFix);
    assert!(intentions[0].preferred);
    assert_eq!(intentions[1].group, IntentionGroup::Refactor);
    assert_eq!(
        intentions[1].item.raw["data"]["scope"], "diagnostic",
        "the diagnostic-scoped copy of a duplicate is the one kept",
    );
    assert_eq!(intentions[2].group, IntentionGroup::Source);

    assert!(
        lsp_core::suggests_organize_imports(&diagnostic),
        "an unresolved-symbol diagnostic is what makes Organize Imports a quick fix",
    );

    manager.stop(LANG);
}

#[test]
fn organize_imports_survives_a_server_that_ignores_the_only_filter() {
    let (manager, _rx, uri) = session_with_open_document();

    // The stub answers nothing at all to a filtered request, which is the
    // behaviour `needs_unfiltered_retry` exists for.
    let action = manager
        .organize_imports(uri, 42)
        .expect("organizeImports")
        .expect("an action");
    assert_eq!(action.title, "Organize imports");
    assert_eq!(action.kind.as_deref(), Some("source.organizeImports"));
    assert!(
        action.edit.is_some(),
        "the retried answer is the real action, edit and all",
    );

    manager.stop(LANG);
}

#[test]
fn a_request_the_server_never_answers_times_out_without_wedging_the_session() {
    let (manager, _rx, _uri) = session_with_open_document();

    let err = manager
        .request_with_timeout(LANG, "stub/silence", json!({}), Duration::from_millis(200))
        .expect_err("a request nobody answers cannot succeed");
    assert!(matches!(err, LspError::Timeout { .. }), "got {err:?}");

    assert_eq!(
        manager
            .request(LANG, "stub/echo", json!({"still": "here"}))
            .expect("the connection survived")["still"],
        "here",
    );

    manager.stop(LANG);
}

#[test]
fn a_reply_that_arrives_after_its_request_was_answered_goes_nowhere() {
    let (manager, _rx, _uri) = session_with_open_document();

    // The stub answers this id twice: once now, once after the caller has
    // already been given the first answer and moved on.
    assert_eq!(
        manager
            .request(LANG, "stub/lateDuplicate", json!({"delay_ms": 100}))
            .expect("the first answer"),
        "first",
    );
    thread::sleep(Duration::from_millis(250));

    // The superseded reply must not be handed to the next request.
    assert_eq!(
        manager
            .request(LANG, "stub/echo", json!({"fresh": true}))
            .expect("the next request gets its own answer")["fresh"],
        true,
    );

    manager.stop(LANG);
}

#[test]
fn a_framed_message_that_is_not_a_response_is_skipped() {
    let (manager, _rx, _uri) = session_with_open_document();

    assert_eq!(
        manager
            .request(LANG, "stub/garbage", json!({}))
            .expect("the response after the garbage still arrives"),
        "still alive",
    );

    manager.stop(LANG);
}

#[test]
fn the_f2_capabilities_are_advertised_because_they_are_implemented() {
    let (manager, _rx, _uri) = session_with_open_document();

    let capabilities = manager
        .request(LANG, "stub/clientCapabilities", json!({}))
        .expect("the stub hands back what we sent");
    let text_document = &capabilities["textDocument"];

    assert_eq!(
        text_document["signatureHelp"]["signatureInformation"]["parameterInformation"]
            ["labelOffsetSupport"],
        true,
        "offsets are unambiguous where a substring label is not",
    );
    assert_eq!(
        text_document["signatureHelp"]["signatureInformation"]["activeParameterSupport"], true,
        "the only way an overload can say it takes fewer arguments",
    );
    assert!(text_document["documentHighlight"].is_object());
    assert!(text_document["inlayHint"].is_object());
    assert!(
        text_document["codeAction"]["codeActionLiteralSupport"]["codeActionKind"]["valueSet"]
            .as_array()
            .expect("a kind list")
            .contains(&json!("source.organizeImports")),
    );

    manager.stop(LANG);
}

// ---------------------------------------------------------------------------
// F0-16: `$/progress`
// ---------------------------------------------------------------------------

/// The defect the F0-15 conformance run found: `ServerReady` fires the moment
/// `initialize` returns, but rust-analyzer answers everything with nothing
/// until it has indexed. `$/progress` is the protocol's way of saying so, and
/// the stub sends the same sequence on demand.
#[test]
fn a_progress_run_drives_the_server_from_indexing_to_idle() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    // The stub answers this only after every notification of the run is
    // written, so nothing below races the reader thread.
    let outcome = manager
        .request(LANG, "stub/indexingRun", json!({}))
        .expect("the stub runs its progress sequence");
    assert_eq!(
        outcome["created"], true,
        "a client that refuses window/workDoneProgress/create is what makes a \
         real server stop reporting progress at all",
    );

    let states: Vec<Option<lsp_core::ServerActivity>> = (0..3)
        .map(|_| {
            wait_for(&rx, "ServerBusy", |e| match e {
                LspEvent::ServerBusy { activity, .. } => Some(activity.clone()),
                _ => None,
            })
        })
        .collect();

    assert_eq!(
        states,
        vec![
            Some(lsp_core::ServerActivity {
                title: "Indexing".into(),
                percentage: Some(0),
            }),
            Some(lsp_core::ServerActivity {
                title: "Indexing".into(),
                percentage: Some(60),
            }),
            // `end` closes the last open token: the server is idle, and its
            // empty answers now mean "no answer exists" rather than "not yet".
            None,
        ],
    );

    manager.stop(LANG);
}

/// Most servers never send `$/progress` at all. The state is advisory and
/// nothing waits on it, so such a server is usable from `ServerReady` onwards
/// and simply never reports being busy — the alternative, treating "no
/// progress yet" as "not ready", would hang the IDE on every one of them.
#[test]
fn a_server_that_never_reports_progress_is_usable_immediately() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    // Answered, not deferred: readiness is not gated on progress.
    let answer = manager
        .request(LANG, "stub/echo", json!({"value": "hello"}))
        .expect("a request is served without any progress having been reported");
    assert_eq!(answer["value"], "hello");

    // And no busy state was ever claimed for it.
    let deadline = Instant::now() + Duration::from_millis(300);
    while let Ok(event) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        assert!(
            !matches!(event, LspEvent::ServerBusy { .. }),
            "a silent server must never be reported busy: {event:?}",
        );
    }

    manager.stop(LANG);
}

/// A server that dies mid-index can never end the token it opened, so the
/// manager ends it on the server's behalf. Without this the status bar would
/// sit on a dead server's last percentage forever.
#[test]
fn a_server_that_dies_mid_progress_stops_being_busy() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });
    // A run that never ends its token, the way a server killed mid-index
    // leaves it.
    manager
        .request(LANG, "stub/indexingRun", json!({"finish": false}))
        .expect("the stub begins work and leaves it open");
    wait_for(&rx, "ServerBusy", |e| match e {
        LspEvent::ServerBusy { activity, .. } if activity.is_some() => Some(()),
        _ => None,
    });

    manager.stop(LANG);

    // Nothing more is emitted after a stop (the supervisor returns), so the
    // clear has to arrive before the connection is torn down — assert on the
    // events already queued.
    let cleared = rx
        .try_iter()
        .any(|e| matches!(e, LspEvent::ServerBusy { activity: None, .. }));
    assert!(cleared, "a dead server's open work is closed on its behalf");
}

/// F0-16: without this capability a server has no permission to open a
/// progress token, and rust-analyzer stays silent for exactly the seconds in
/// which it cannot answer anything.
#[test]
fn work_done_progress_is_advertised() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");

    let capabilities = manager
        .request(LANG, "stub/clientCapabilities", json!({}))
        .expect("the stub hands back what we sent");
    assert_eq!(capabilities["window"]["workDoneProgress"], true);

    manager.stop(LANG);
}

// ---------------------------------------------------------------------------
// C4: `client/registerCapability` / `client/unregisterCapability`
// ---------------------------------------------------------------------------

/// csharp-ls declares most of its capabilities via dynamic registration
/// rather than up front, so a client that cannot answer
/// `client/registerCapability` never sees them. This drives the stub through
/// the real register-then-unregister sequence on the actual reader thread
/// and checks the registry on the other side of it, not just the unit-level
/// `Registrations` struct.
#[test]
fn register_capability_then_unregister_round_trips_through_the_reader_thread() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    assert!(
        !manager.method_registered(LANG, "workspace/didChangeWatchedFiles"),
        "nothing has registered yet",
    );

    // The stub pauses between its register and unregister requests, wide
    // enough that the test thread can observe the registration actually
    // land — through the real reader thread, not a direct call into
    // `Registrations` — before it is taken away again.
    let outcome = thread::scope(|scope| {
        let run = scope.spawn(|| {
            manager
                .request(LANG, "stub/registerCapabilityRun", json!({"pause_ms": 200}))
                .expect("the stub runs its register/unregister sequence")
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && !manager.method_registered(LANG, "workspace/didChangeWatchedFiles")
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            manager.method_registered(LANG, "workspace/didChangeWatchedFiles"),
            "a client that answers client/registerCapability with \"not \
             implemented\" is what makes csharp-ls never see its own \
             capabilities",
        );

        run.join().expect("stub run thread did not panic")
    });

    assert_eq!(outcome["registered"], true);
    assert_eq!(outcome["unregistered"], true);
    assert!(
        !manager.method_registered(LANG, "workspace/didChangeWatchedFiles"),
        "client/unregisterCapability must remove the registration again",
    );

    manager.stop(LANG);
}

/// C5: while the stub's `**/*.rs` watcher (`stub/registerCapabilityRun`) is
/// live, `did_change_watched_files` must filter out the non-matching path,
/// batch the matching ones into one notification, and send nothing at all
/// once the registration is gone again.
#[test]
fn did_change_watched_files_filters_and_batches_through_the_real_registration() {
    use lsp_core::watched_files::FileChangeKind;
    use std::path::PathBuf;

    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    thread::scope(|scope| {
        let run = scope.spawn(|| {
            manager
                .request(LANG, "stub/registerCapabilityRun", json!({"pause_ms": 300}))
                .expect("the stub runs its register/unregister sequence")
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && !manager.method_registered(LANG, "workspace/didChangeWatchedFiles")
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(manager.method_registered(LANG, "workspace/didChangeWatchedFiles"));

        manager
            .did_change_watched_files(
                LANG,
                &[
                    (
                        PathBuf::from("/workspace/src/a.rs"),
                        FileChangeKind::Changed,
                    ),
                    (
                        PathBuf::from("/workspace/README.md"),
                        FileChangeKind::Changed,
                    ),
                ],
            )
            .expect("filtering and sending must not error");

        run.join().expect("stub run thread did not panic");
    });

    let sent = manager
        .request(LANG, "stub/lastWatchedFilesChange", json!({}))
        .expect("stub answers");
    let changes = sent["changes"].as_array().expect("changes array");
    assert_eq!(changes.len(), 1, "only the .rs path matches **/*.rs");
    assert_eq!(changes[0]["uri"], "file:///workspace/src/a.rs");
    assert_eq!(changes[0]["type"], FileChangeKind::Changed as u8);

    // The registration is gone again after the stub's unregister — sending
    // must now be a no-op rather than waking a server that asked for
    // nothing.
    assert!(!manager.method_registered(LANG, "workspace/didChangeWatchedFiles"));
    manager
        .did_change_watched_files(
            LANG,
            &[(
                PathBuf::from("/workspace/src/b.rs"),
                FileChangeKind::Created,
            )],
        )
        .expect("no-op send must not error");
    let after_unregister = manager
        .request(LANG, "stub/lastWatchedFilesChange", json!({}))
        .expect("stub answers");
    assert_eq!(
        after_unregister, sent,
        "no new notification should have been sent once nothing is registered",
    );

    manager.stop(LANG);
}

/// C6: `workspace/configuration` answers the pulled section with the
/// server's configured settings, and any other section with `null` — the
/// full round trip through the real reader thread and `dispatch`, not just
/// `configuration::resolve` in isolation.
#[test]
fn workspace_configuration_answers_the_configured_section_and_nulls_the_rest() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager
        .start(&stub_config_with_settings())
        .expect("stub starts");

    let answer = manager
        .request(LANG, "stub/configurationRun", json!({}))
        .expect("the stub's configuration pull does not deadlock us");
    let items = answer.as_array().expect("array reply, one per item");
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0],
        json!({"analyzersEnabled": true}),
        "the configured section"
    );
    assert_eq!(
        items[1],
        serde_json::Value::Null,
        "a section this client has no opinion on"
    );

    manager.stop(LANG);
}

/// C6: `update_settings` replaces the stored settings and sends
/// `workspace/didChangeConfiguration` with `{"settings": null}` — telling a
/// pull-based server to re-fetch rather than pushing the value inline — so a
/// pull issued afterwards sees the new settings.
#[test]
fn update_settings_changes_what_the_next_pull_answers() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    manager
        .start(&stub_config_with_settings())
        .expect("stub starts");

    manager
        .update_settings(LANG, json!({"analyzersEnabled": false}))
        .expect("the server is running");

    let answer = manager
        .request(LANG, "stub/configurationRun", json!({}))
        .expect("pull after the settings change");
    assert_eq!(answer[0], json!({"analyzersEnabled": false}));

    manager.stop(LANG);
}

/// C6: a language with no running server is reported the same way every
/// other per-language method reports it, not a silent no-op.
#[test]
fn update_settings_on_a_server_that_is_not_running_is_an_error() {
    let (manager, _rx) = LspManager::new("file:///workspace");
    assert!(matches!(
        manager.update_settings(LANG, json!({})),
        Err(LspError::NoServer(_))
    ));
}

// ---------------------------------------------------------------------------
// C7: completionItem/resolve
// ---------------------------------------------------------------------------

/// A server that never advertises `completionProvider.resolveProvider`
/// reports `completion_resolve_supported: false` on `ServerReady` — the flag
/// that gates every caller from spending the round trip on a server that
/// cannot answer it.
#[test]
fn server_ready_reports_no_resolve_support_when_not_advertised() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    let supported = wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady {
            completion_resolve_supported,
            ..
        } => Some(*completion_resolve_supported),
        _ => None,
    });
    assert!(!supported);
    manager.stop(LANG);
}

/// The same event reports `true` for a server that advertises
/// `completionProvider.resolveProvider: true`, the way csharp-ls does.
#[test]
fn server_ready_reports_resolve_support_when_advertised() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager
        .start(&stub_config_with_completion_resolve())
        .expect("stub starts");
    let supported = wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady {
            completion_resolve_supported,
            ..
        } => Some(*completion_resolve_supported),
        _ => None,
    });
    assert!(supported);
    manager.stop(LANG);
}

/// `resolve_completion_item` round-trips the item through the reader thread
/// and returns whatever the server added — here, the `additionalTextEdits`
/// that simulate a `using` insertion.
#[test]
fn resolve_completion_item_returns_the_resolved_item() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager
        .start(&stub_config_with_completion_resolve())
        .expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    let item = json!({"label": "List", "kind": 7});
    let resolved = manager
        .resolve_completion_item(LANG, &item)
        .expect("the stub answers completionItem/resolve");
    assert_eq!(resolved["label"], "List");
    assert_eq!(
        resolved["additionalTextEdits"][0]["newText"],
        "using System.Collections.Generic;\n"
    );

    manager.stop(LANG);
}

/// A `completionItem/resolve` that never answers is bounded by the same
/// timeout/`$/cancelRequest` path every other request uses — proven here
/// through `request_with_timeout` directly (with a short deadline, rather
/// than `resolve_completion_item`'s own [`lsp_core::DEFAULT_REQUEST_TIMEOUT`],
/// so the test does not have to wait ten seconds) since `resolve_completion_item`
/// is a thin wrapper with no logic of its own to test separately — see its
/// source in `manager.rs`. The accept-path fallback this unblocks — apply the
/// unresolved item's own edit rather than hang — is exercised at the
/// `ui-shell` layer, in `crates/ui-shell/src/bridge/language/mod.rs`.
#[test]
fn a_resolve_that_never_answers_times_out_and_is_cancelled() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager
        .start(&stub_config_with_completion_resolve())
        .expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    let item = json!({"label": "stub/silence"});
    let result = manager.request_with_timeout(
        LANG,
        "completionItem/resolve",
        item,
        Duration::from_millis(200),
    );
    assert!(matches!(
        result,
        Err(LspError::Timeout { method }) if method == "completionItem/resolve"
    ));

    // The server is still alive and answering: the request was cancelled,
    // not the connection torn down.
    let echoed = manager
        .request(LANG, "stub/echo", json!({"tag": "after-resolve-timeout"}))
        .expect("the server is still usable after a cancelled resolve");
    assert_eq!(echoed["tag"], "after-resolve-timeout");

    manager.stop(LANG);
}

/// C9: `semantic_tokens_legend` reads the legend from `initialize`'s static
/// `semanticTokensProvider`, and `semantic_tokens` decodes the canned
/// response into the tokens `crates/lsp-core/src/bin/stub_server.rs`
/// documents — through the real reader thread and `dispatch`, not just
/// `lsp_core::semantic_tokens` in isolation.
#[test]
fn semantic_tokens_are_read_from_the_static_initialize_capability() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager
        .start(&stub_config_with_semantic_tokens_static())
        .expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    let legend = manager
        .semantic_tokens_legend(LANG)
        .expect("the static capability was read at connect time");
    assert_eq!(legend.token_types[1], "type");
    assert_eq!(legend.token_types[12], "function");
    assert!(legend.full);

    let result = manager
        .semantic_tokens(LANG, "file:///stub/main.rs")
        .expect("the stub answers textDocument/semanticTokens/full");
    let (result_id, tokens) =
        lsp_core::parse_semantic_tokens_full(&result).expect("the stub sent data");
    assert_eq!(result_id, Some("1".to_string()));
    assert_eq!(tokens.len(), 3);
    // Same-line, multiple tokens: the second token's column is relative to
    // the first token's own start (0 + 4 = 4), not to column 0.
    assert_eq!(tokens[0].line, 0);
    assert_eq!(tokens[0].start_char, 0);
    assert_eq!(tokens[1].line, 0);
    assert_eq!(tokens[1].start_char, 4);
    // Line advance: the third token resets its column to its own delta.
    assert_eq!(tokens[2].line, 1);
    assert_eq!(tokens[2].start_char, 0);

    let scope = lsp_core::semantic_token_scope(&legend, &tokens[1])
        .expect("\"function\" with defaultLibrary resolves");
    assert_eq!(scope.name(), "function.builtin");

    manager.stop(LANG);
}

/// C9: a server with nothing to say for a document (the stub's
/// `uri.ends_with("empty.rs")` case) answers `null`, which
/// `parse_semantic_tokens_full` reports as `None` rather than an empty
/// token list mistaken for "the server said nothing was highlighted".
#[test]
fn semantic_tokens_null_response_is_not_an_error() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager
        .start(&stub_config_with_semantic_tokens_static())
        .expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    let result = manager
        .semantic_tokens(LANG, "file:///stub/empty.rs")
        .expect("the stub answers even with nothing to say");
    assert!(lsp_core::parse_semantic_tokens_full(&result).is_none());

    manager.stop(LANG);
}

/// C9: a server with no semantic-tokens capability at all — the plain
/// `stub_config()`, which advertises neither the static capability nor
/// runs the dynamic registration — must never be asked: `semantic_tokens_legend`
/// answers `None`, which is this client's generic gate on both paths.
#[test]
fn semantic_tokens_legend_is_none_for_a_server_that_never_advertised_it() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    assert!(manager.semantic_tokens_legend(LANG).is_none());

    manager.stop(LANG);
}

/// C9: csharp-ls's suspected path — declaring `textDocument/semanticTokens`
/// via `client/registerCapability` rather than `initialize`'s static
/// result. `semantic_tokens_legend` must pick it up exactly as it does the
/// static path, through the real reader thread's `client/registerCapability`
/// handling, not a direct call into `crate::semantic_tokens`.
#[test]
fn semantic_tokens_legend_is_read_from_a_dynamic_registration() {
    let (manager, rx) = LspManager::new("file:///workspace");
    manager.start(&stub_config()).expect("stub starts");
    wait_for(&rx, "ServerReady", |e| match e {
        LspEvent::ServerReady { .. } => Some(()),
        _ => None,
    });

    assert!(
        manager.semantic_tokens_legend(LANG).is_none(),
        "nothing has registered yet"
    );

    let outcome = manager
        .request(LANG, "stub/semanticTokensRegisterRun", json!({}))
        .expect("the stub runs its registration sequence");
    assert_eq!(outcome["registered"], true);

    let legend = manager
        .semantic_tokens_legend(LANG)
        .expect("the dynamic registration's legend was captured");
    assert_eq!(legend.token_types[15], "keyword");
    assert!(legend.full);

    manager.stop(LANG);
}
