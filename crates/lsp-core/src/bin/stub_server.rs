//! A minimal stub language server, used as a test fixture so `lsp-core`'s
//! tests run offline with no real language server installed (task X2).
//!
//! It is a `[[bin]]` of this crate rather than an example so that integration
//! tests can locate it with `env!("CARGO_BIN_EXE_stub_server")` — a path Cargo
//! guarantees, instead of guessing at `target/debug` layout, which breaks
//! under a custom `CARGO_TARGET_DIR`, `--release`, or cross-compilation.
//!
//! Supported: framing, `initialize`/`initialized`/`shutdown`/`exit`, a canned
//! diagnostic on `textDocument/didOpen`, and two test affordances —
//! `stub/echo` (with an optional `delay_ms`, answered on its own thread so
//! responses can come back out of order) and the `STUB_LSP_DIE_ON_DIDOPEN`
//! environment variable, which makes the server die mid-session.

use std::io::{self, BufReader, Stdout, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use lsp_core::framing::{read_message, write_message};
use serde_json::{json, Value};

/// Set to `1` to exit(1) right after answering the first `textDocument/didOpen`,
/// so respawn and backoff can be exercised.
const DIE_ON_DIDOPEN: &str = "STUB_LSP_DIE_ON_DIDOPEN";

type Out = Arc<Mutex<Stdout>>;

fn send(out: &Out, message: Value) {
    let payload = serde_json::to_vec(&message).expect("serializable");
    let mut guard = out.lock().expect("stdout lock");
    write_message(&mut *guard, &payload).expect("write to stdout");
}

fn main() {
    let out: Out = Arc::new(Mutex::new(io::stdout()));
    let mut input = BufReader::new(io::stdin());
    let die_on_didopen = std::env::var(DIE_ON_DIDOPEN).is_ok_and(|v| v == "1");

    while let Some(body) = read_message(&mut input).expect("read from stdin") {
        let message: Value = match serde_json::from_slice(&body) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        match (method, id) {
            ("initialize", Some(id)) => send(
                &out,
                json!({"jsonrpc": "2.0", "id": id, "result": {
                    "capabilities": {"textDocumentSync": 1},
                    "serverInfo": {"name": "stub_server", "version": "0.1.0"},
                }}),
            ),
            ("initialized", _) => {}
            ("shutdown", Some(id)) => {
                send(&out, json!({"jsonrpc": "2.0", "id": id, "result": null}))
            }
            ("exit", _) => return,
            ("textDocument/didOpen", _) => {
                let uri = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let version = params.pointer("/textDocument/version").cloned();
                send(
                    &out,
                    json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                           "params": {
                        "uri": uri,
                        "version": version,
                        "diagnostics": [canned_diagnostic()],
                    }}),
                );
                if die_on_didopen {
                    // Die mid-session, without an `exit`: the client must see
                    // EOF and respawn us.
                    std::process::exit(1);
                }
            }
            // Echo the params back, optionally after a delay, on its own
            // thread — so a slow request cannot hold up a later fast one and
            // response correlation is actually tested.
            ("stub/echo", Some(id)) => {
                let out = Arc::clone(&out);
                let delay = params.get("delay_ms").and_then(Value::as_u64).unwrap_or(0);
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(delay));
                    send(&out, json!({"jsonrpc": "2.0", "id": id, "result": params}));
                });
            }
            // Ask the client something it does not implement, to check we
            // answer server-to-client requests instead of hanging.
            ("stub/askClient", Some(id)) => {
                let out = Arc::clone(&out);
                thread::spawn(move || {
                    send(
                        &out,
                        json!({"jsonrpc": "2.0", "id": 9001,
                                      "method": "workspace/configuration", "params": {}}),
                    );
                    send(&out, json!({"jsonrpc": "2.0", "id": id, "result": "asked"}));
                });
            }
            (_, Some(id)) => send(
                &out,
                json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": -32601, "message": format!("{method} is not implemented"),
                }}),
            ),
            (_, None) => {}
        }
        io::stdout().flush().ok();
    }
}

/// The one diagnostic this server ever reports, on line 1.
fn canned_diagnostic() -> Value {
    json!({
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}},
        "severity": 1,
        "source": "stub_server",
        "message": "canned diagnostic",
    })
}
