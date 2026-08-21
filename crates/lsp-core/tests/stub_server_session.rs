//! End-to-end tests of `LspManager` against the X2 stub server.
//!
//! The stub's path comes from `CARGO_BIN_EXE_stub_server`, which Cargo sets
//! for integration tests, so nothing here guesses at the target directory.

use std::sync::mpsc::Receiver;
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

    let err = manager
        .request(LANG, "textDocument/hover", json!({}))
        .expect_err("the stub implements no hover");
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
