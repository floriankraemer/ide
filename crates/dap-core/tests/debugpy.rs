//! What this client and a real adapter agree on (D3-9).
//!
//! Every other test in this crate drives a stand-in written to behave; this
//! one drives debugpy, which behaves however debugpy behaves. That is the
//! difference that matters: the first failure of this feature in practice
//! was a *claim* this client made to a third party (`runInTerminal`), which
//! no test against a stand-in could have caught.
//!
//! Skipped when debugpy is not installed, so a developer without it is not
//! blocked; the builder image has it, so CI always runs it.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dap_core::catalog::Adapter;
use dap_core::{DapSession, SessionListener};
use serde_json::{json, Value};

fn debugpy_installed() -> bool {
    std::process::Command::new("python3")
        .args(["-c", "import debugpy"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn adapter() -> Adapter {
    dap_core::catalog::resolve("debugpy", &[]).expect("debugpy is in the shipped catalog")
}

struct Events {
    sender: Sender<(String, Value)>,
}

impl SessionListener for Events {
    fn event(&mut self, event: &str, body: &Value) {
        let _ = self.sender.send((event.to_string(), body.clone()));
    }
    fn reverse_request(&mut self, _command: &str, _arguments: &Value) -> Option<Value> {
        None
    }
    fn disconnected(&mut self) {
        let _ = self.sender.send(("disconnected".into(), Value::Null));
    }
}

fn wait_for(events: &Receiver<(String, Value)>, name: &str) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        match events.recv_timeout(Duration::from_secs(30)) {
            Ok((event, body)) if event == name => return body,
            Ok((event, _)) if event == "disconnected" => {
                panic!("the adapter exited before {name} arrived")
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    panic!("no {name} event arrived");
}

/// A script with a line worth stopping on, in a directory of its own.
fn script() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("main.py");
    std::fs::write(&path, "answer = 42\nprint(answer)\n").expect("write script");
    (dir, path)
}

#[test]
fn debugpy_stops_at_a_breakpoint_and_reports_the_variable() {
    if !debugpy_installed() {
        eprintln!("skipping: debugpy is not installed");
        return;
    }

    let (dir, path) = script();
    let (sender, events) = channel();
    let session = DapSession::start(&adapter(), Some(dir.path()), Box::new(Events { sender }))
        .expect("debugpy starts");

    let capabilities = session.initialize().expect("initialize");
    assert!(
        capabilities.supports_configuration_done_request,
        "debugpy has always supported configurationDone; if this fails the \
         capability parsing is wrong, not debugpy"
    );

    let spec = run_core::LaunchSpec {
        program: "python3".into(),
        args: vec![path.display().to_string()],
        cwd: Some(dir.path().to_path_buf()),
        env: Vec::new(),
        console: run_core::ConsoleKind::Pipes,
    };
    // The order DAP actually requires: launch is sent without waiting,
    // breakpoints go in after the adapter says it is ready, and
    // `configurationDone` is what releases the debuggee — and, for debugpy,
    // what releases the launch response too.
    session
        .launch(dap_core::launch::arguments("debugpy", &spec))
        .expect("launch");
    session
        .wait_for_initialized(Duration::from_secs(10))
        .expect("initialized");

    session
        .request(
            "setBreakpoints",
            json!({
                "source": { "path": path.display().to_string() },
                "breakpoints": [{ "line": 2 }],
            }),
        )
        .expect("setBreakpoints");
    session.configuration_done().expect("configurationDone");

    let stopped = wait_for(&events, "stopped");
    assert_eq!(stopped["reason"], "breakpoint");
    let thread_id = stopped["threadId"].as_i64().expect("threadId");

    let frames = session
        .request("stackTrace", json!({ "threadId": thread_id }))
        .map(|body| dap_core::protocol::stack_frames(&body))
        .expect("stackTrace");
    assert_eq!(frames[0].line, 2, "stopped on the breakpoint's line");

    let scopes = session
        .request("scopes", json!({ "frameId": frames[0].id }))
        .map(|body| dap_core::protocol::scopes(&body))
        .expect("scopes");
    let variables = session
        .request(
            "variables",
            json!({ "variablesReference": scopes[0].variables_reference }),
        )
        .map(|body| dap_core::protocol::variables(&body))
        .expect("variables");
    assert!(
        variables
            .iter()
            .any(|v| v.name == "answer" && v.value == "42"),
        "the local we set is not in the first scope: {variables:?}"
    );

    session.shutdown();
}

#[test]
fn a_missing_adapter_reports_its_install_hint_rather_than_an_os_error() {
    let missing = Adapter {
        program: "definitely-not-debugpy".into(),
        ..adapter()
    };
    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let _ = events;
    let (sender, _events) = channel();
    let err = match DapSession::start(&missing, None, Box::new(Events { sender })) {
        Err(err) => err,
        Ok(_) => panic!("an adapter that is not installed must not start"),
    };
    assert!(err.to_string().contains("pip install debugpy"), "{err}");
}
