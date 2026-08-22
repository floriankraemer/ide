//! A minimal stub language server, used as a test fixture so `lsp-core`'s
//! tests run offline with no real language server installed (task X2).
//!
//! It is a `[[bin]]` of this crate rather than an example so that integration
//! tests can locate it with `env!("CARGO_BIN_EXE_stub_server")` — a path Cargo
//! guarantees, instead of guessing at `target/debug` layout, which breaks
//! under a custom `CARGO_TARGET_DIR`, `--release`, or cross-compilation.
//!
//! Supported: framing, `initialize`/`initialized`/`shutdown`/`exit`, a canned
//! diagnostic on `textDocument/didOpen`, canned `textDocument/hover`,
//! `textDocument/definition` and `textDocument/completion` replies that vary by the requested position so
//! every response shape the protocol allows is reachable, and two test
//! affordances —
//! `stub/echo` (with an optional `delay_ms`, answered on its own thread so
//! responses can come back out of order) and the `STUB_LSP_DIE_ON_DIDOPEN`
//! environment variable, which makes the server die mid-session.
//!
//! RF3 added the refactoring half: `textDocument/codeAction`,
//! `codeAction/resolve`, `textDocument/prepareRename`, `textDocument/rename`
//! and `workspace/executeCommand` — the last of which can send a
//! `workspace/applyEdit` request *back* to the client and block on the
//! answer, which is how the command-driven refactorings that jdtls and
//! intelephense use are exercised offline.

use std::collections::HashMap;
use std::io::{self, BufReader, Stdout, Write};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use lsp_core::framing::{read_message, write_message};
use serde_json::{json, Value};

/// Set to `1` to exit(1) right after answering the first `textDocument/didOpen`,
/// so respawn and backoff can be exercised.
const DIE_ON_DIDOPEN: &str = "STUB_LSP_DIE_ON_DIDOPEN";

type Out = Arc<Mutex<Stdout>>;

/// Responses to requests *this server* sent to the client, waiting to be
/// handed back to the thread that is blocked on them.
type Pending = Arc<Mutex<HashMap<i64, Sender<Value>>>>;

/// How long a server-to-client request waits before giving up. Only a
/// deadlocked client ever reaches it, and a test that hangs is worse than a
/// test that fails.
const CLIENT_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

fn send(out: &Out, message: Value) {
    let payload = serde_json::to_vec(&message).expect("serializable");
    let mut guard = out.lock().expect("stdout lock");
    write_message(&mut *guard, &payload).expect("write to stdout");
}

fn main() {
    let out: Out = Arc::new(Mutex::new(io::stdout()));
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
    let mut next_request_id = 9100i64;
    let mut input = BufReader::new(io::stdin());
    let die_on_didopen = std::env::var(DIE_ON_DIDOPEN).is_ok_and(|v| v == "1");

    while let Some(body) = read_message(&mut input).expect("read from stdin") {
        let message: Value = match serde_json::from_slice(&body) {
            Ok(m) => m,
            Err(_) => continue,
        };
        // A message with an id and no method is the client answering
        // something we asked, not a request: hand it to whoever is waiting.
        if message.get("method").is_none() {
            if let Some(id) = message.get("id").and_then(Value::as_i64) {
                if let Some(tx) = pending.lock().expect("pending lock").remove(&id) {
                    let _ = tx.send(message.get("result").cloned().unwrap_or(Value::Null));
                }
            }
            continue;
        }

        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        match (method, id) {
            ("initialize", Some(id)) => send(
                &out,
                json!({"jsonrpc": "2.0", "id": id, "result": {
                    "capabilities": {
                        "textDocumentSync": 1,
                        // L5: the same pair rust-analyzer advertises, so
                        // `.` and (twice over) `::` are covered.
                        "completionProvider": {"triggerCharacters": [".", ":"]},
                    },
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
            // L3: a hover in every shape a real server might pick, chosen by
            // the requested line so one stub covers the parsing matrix:
            // line 0 -> MarkupContent, line 1 -> a {language, value}
            // MarkedString, line 2 -> an array, anything else -> no hover.
            ("textDocument/hover", Some(id)) => {
                let line = params
                    .pointer("/position/line")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let contents = match line {
                    0 => {
                        json!({"kind": "markdown", "value": "```rust\nfn main()\n```\nThe **entry** point."})
                    }
                    1 => json!({"language": "rust", "value": "fn main()"}),
                    2 => json!(["plain hover", {"language": "rust", "value": "fn main()"}]),
                    _ => Value::Null,
                };
                let result = if contents.is_null() {
                    Value::Null
                } else {
                    json!({ "contents": contents })
                };
                send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
            }
            // L4: one target per requested character, so the single-Location,
            // Location-array and LocationLink-array replies are all reachable.
            ("textDocument/definition", Some(id)) => {
                let uri = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let character = params
                    .pointer("/position/character")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let result = match character {
                    0 => location(&uri, 3, 4),
                    1 => json!([location(&uri, 3, 4), location(&uri, 9, 2)]),
                    2 => json!([{
                        "targetUri": uri,
                        "targetRange": {"start": {"line": 3, "character": 0},
                                        "end": {"line": 5, "character": 1}},
                        "targetSelectionRange": {"start": {"line": 3, "character": 4},
                                                 "end": {"line": 3, "character": 8}},
                    }]),
                    _ => Value::Null,
                };
                send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
            }
            // L5: both response shapes, chosen by the requested line —
            // line 0 -> a bare CompletionItem[], line 1 -> a CompletionList
            // that is incomplete and carries a snippet item and a textEdit
            // item, anything else -> null (nothing to complete).
            ("textDocument/completion", Some(id)) => {
                let line = params
                    .pointer("/position/line")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let result = match line {
                    0 => json!([
                        {"label": "push", "kind": 2, "detail": "fn push(&mut self, value: T)",
                         "sortText": "0001"},
                        {"label": "pop", "kind": 2, "sortText": "0000",
                         "documentation": {"kind": "markdown", "value": "Removes the last element."}},
                        {"label": "#[allow]", "kind": 15, "filterText": "allow",
                         "insertText": "#[allow(dead_code)]"},
                    ]),
                    1 => json!({"isIncomplete": true, "items": [
                        {"label": "map", "kind": 3, "insertTextFormat": 2,
                         "insertText": "map(${1:f})$0"},
                        {"label": "max", "kind": 3, "textEdit": {
                            "newText": "max()",
                            "range": {"start": {"line": 1, "character": 2},
                                      "end": {"line": 1, "character": 4}},
                        }},
                    ]}),
                    _ => Value::Null,
                };
                send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
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
            // RF3: code actions, chosen by the requested range's start line so
            // one stub covers the shapes the parser has to handle —
            // line 0 -> a CodeAction carrying its own edit,
            // line 1 -> a bare Command and a CodeAction carrying a command,
            // line 2 -> an unresolved item (no edit, no command, has `data`),
            // line 3 -> a disabled item,
            // anything else -> no actions here.
            ("textDocument/codeAction", Some(id)) => {
                let uri = uri_of(&params);
                let line = params
                    .pointer("/range/start/line")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let result = match line {
                    0 => json!([{
                        "title": "Extract into function",
                        "kind": "refactor.extract.function",
                        "edit": {"documentChanges": [{
                            "textDocument": {"uri": uri, "version": 1},
                            "edits": [{
                                "range": {"start": {"line": 0, "character": 0},
                                          "end": {"line": 0, "character": 3}},
                                "newText": "extracted()",
                            }],
                        }]},
                    }]),
                    1 => json!([
                        {"title": "Run the command directly",
                         "command": "stub.plain", "arguments": [1]},
                        {"title": "Extract class",
                         "kind": "refactor.extract.class",
                         "command": {"title": "Extract class",
                                     "command": "stub.applyEdit", "arguments": [uri]}},
                    ]),
                    2 => json!([{
                        "title": "Needs resolving",
                        "kind": "refactor.inline",
                        "data": {"token": 42},
                    }]),
                    3 => json!([{
                        "title": "Not available here",
                        "kind": "refactor.extract",
                        "disabled": {"reason": "selection is not an expression"},
                    }]),
                    _ => Value::Null,
                };
                send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
            }
            // The unresolved item from line 2 above, with its edit filled in.
            ("codeAction/resolve", Some(id)) => {
                let mut action = params.clone();
                action["edit"] = json!({"changes": {
                    "file:///stub/resolved.rs": [{
                        "range": {"start": {"line": 2, "character": 0},
                                  "end": {"line": 2, "character": 1}},
                        "newText": "resolved",
                    }],
                }});
                send(&out, json!({"jsonrpc": "2.0", "id": id, "result": action}));
            }
            // RF3: prepareRename, one shape per requested line —
            // 0 -> a bare Range, 1 -> {range, placeholder},
            // 2 -> {defaultBehavior}, anything else -> null (cannot rename).
            ("textDocument/prepareRename", Some(id)) => {
                let line = position_line(&params);
                let range = json!({"start": {"line": line, "character": 4},
                                   "end": {"line": line, "character": 8}});
                let result = match line {
                    0 => range,
                    1 => json!({"range": range, "placeholder": "old_name"}),
                    2 => json!({"defaultBehavior": true}),
                    _ => Value::Null,
                };
                send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
            }
            // RF3: rename, again by line — 0 -> a versioned documentChanges
            // edit, 1 -> a legacy `changes` edit, 2 -> null (nothing to do),
            // anything else -> an error response.
            ("textDocument/rename", Some(id)) => {
                let uri = uri_of(&params);
                let new_name = params
                    .get("newName")
                    .and_then(Value::as_str)
                    .unwrap_or("new_name")
                    .to_string();
                let range = json!({"start": {"line": 0, "character": 4},
                                   "end": {"line": 0, "character": 8}});
                match position_line(&params) {
                    0 => send(
                        &out,
                        json!({"jsonrpc": "2.0", "id": id, "result": {"documentChanges": [{
                            "textDocument": {"uri": uri, "version": 1},
                            "edits": [{"range": range, "newText": new_name}],
                        }]}}),
                    ),
                    1 => send(
                        &out,
                        json!({"jsonrpc": "2.0", "id": id, "result": {"changes": {
                            uri: [{"range": range, "newText": new_name}],
                        }}}),
                    ),
                    2 => send(&out, json!({"jsonrpc": "2.0", "id": id, "result": null})),
                    _ => send(
                        &out,
                        json!({"jsonrpc": "2.0", "id": id, "error": {
                            "code": -32602, "message": "cannot rename this element",
                        }}),
                    ),
                }
            }
            // RF3: the command-driven refactoring shape. `stub.applyEdit`
            // asks the client to apply an edit and blocks on the answer
            // before completing, which is exactly what jdtls, omnisharp and
            // intelephense do for Extract — and the reason the client may
            // not answer server requests on its read thread.
            ("workspace/executeCommand", Some(id)) => {
                let command = params
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if command != "stub.applyEdit" {
                    send(&out, json!({"jsonrpc": "2.0", "id": id, "result": null}));
                    io::stdout().flush().ok();
                    continue;
                }
                let uri = params
                    .pointer("/arguments/0")
                    .and_then(Value::as_str)
                    .unwrap_or("file:///stub/commanded.rs")
                    .to_string();
                let request_id = next_request_id;
                next_request_id += 1;

                let (tx, rx) = channel();
                pending.lock().expect("pending lock").insert(request_id, tx);

                let out = Arc::clone(&out);
                let pending = Arc::clone(&pending);
                thread::spawn(move || {
                    send(
                        &out,
                        json!({"jsonrpc": "2.0", "id": request_id,
                               "method": "workspace/applyEdit", "params": {
                            "label": "Extract class",
                            "edit": {"documentChanges": [{
                                "textDocument": {"uri": uri, "version": 1},
                                "edits": [{
                                    "range": {"start": {"line": 0, "character": 0},
                                              "end": {"line": 0, "character": 0}},
                                    "newText": "class Extracted {}\n",
                                }],
                            }]},
                        }}),
                    );
                    let answer = rx.recv_timeout(CLIENT_REPLY_TIMEOUT).unwrap_or(Value::Null);
                    pending.lock().expect("pending lock").remove(&request_id);
                    send(
                        &out,
                        json!({"jsonrpc": "2.0", "id": id, "result": {
                            "clientApplied": answer.get("applied").and_then(Value::as_bool),
                        }}),
                    );
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

/// The `textDocument.uri` of a request, or a stand-in when it has none.
fn uri_of(params: &Value) -> String {
    params
        .pointer("/textDocument/uri")
        .and_then(Value::as_str)
        .unwrap_or("file:///stub/main.rs")
        .to_string()
}

/// The 0-based line of a request's `position`, which is what every canned
/// answer above is keyed by.
fn position_line(params: &Value) -> u64 {
    params
        .pointer("/position/line")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

/// A `Location` in `uri` at a 0-based line/character.
fn location(uri: &str, line: u64, character: u64) -> Value {
    json!({"uri": uri, "range": {
        "start": {"line": line, "character": character},
        "end": {"line": line, "character": character + 4},
    }})
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
