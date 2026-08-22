//! Server lifecycle, request routing, restart policy and document-version
//! tracking — the rules of talking to a language server.
//!
//! Blocking threads only, no async runtime (plan decision 9): one child
//! process per language, one writer behind a mutex, one supervisor thread per
//! server that both reads that server's stdout and owns its restart loop.
//! Everything the UI needs to see arrives on a single `Receiver<LspEvent>`.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::apply_edit::{
    await_verdict, ApplyEditGate, RefactorSession, RefactorSessions, UNSOLICITED_REASON,
};
use crate::catalog::ServerConfig;
use crate::completion::{parse_completion, parse_trigger_characters, CompletionList};
use crate::framing::{read_message, write_message};
use crate::hover::{parse_hover, HoverText};
use crate::navigation::{parse_definition, DefinitionTarget};

/// How long a request waits for its response before it is cancelled.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Hover is speculative and mouse-driven: an answer that arrives seconds
/// after the dwell is not worth showing, so it gets its own short deadline.
pub const HOVER_TIMEOUT: Duration = Duration::from_secs(2);
/// Completion is asked for per keystroke and the popup is only useful while
/// the word it describes is still being typed, so it gets the shortest
/// deadline of all: a list that lands later is discarded anyway.
pub const COMPLETION_TIMEOUT: Duration = Duration::from_secs(3);
/// Go to Definition is an explicit gesture and the user is waiting for it,
/// but a jump that lands half a minute later is a bug, not a jump.
pub const DEFINITION_TIMEOUT: Duration = Duration::from_secs(5);
/// Delay before the first respawn attempt; doubles per consecutive failure.
const RESTART_BACKOFF_INITIAL: Duration = Duration::from_millis(200);
/// Ceiling for the exponential backoff.
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(10);
/// Consecutive crashes tolerated before the server is given up on. A server
/// that dies this often is broken, and respawning forever would just burn CPU.
const MAX_RESTARTS: u32 = 5;
/// How long a stopping server is given to exit on its own before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
/// A session that lasted at least this long counts as healthy, so its exit
/// resets the restart budget and the backoff.
const HEALTHY_SESSION: Duration = Duration::from_secs(30);

/// Something the UI wants to know about. Delivered in arrival order on the
/// manager's single event channel; a dropped receiver silently discards.
#[derive(Debug, Clone)]
pub enum LspEvent {
    /// The server finished `initialize`/`initialized` and accepts requests.
    /// `restarts` is 0 for the first launch and counts respawns after that.
    ServerReady {
        language_id: String,
        restarts: u32,
        /// The characters this server wants completion requested after, from
        /// its `initialize` result — `.` in most languages, `:` in Rust.
        trigger_characters: Vec<String>,
    },
    /// The server's stdout hit EOF or errored, i.e. it died. A respawn follows
    /// after `retry_in` unless the restart budget is used up.
    ServerExited {
        language_id: String,
        restarts: u32,
        retry_in: Duration,
    },
    /// The server could not be launched or handshaken, or crashed past its
    /// restart budget. No further events will arrive for this language.
    ServerFailed {
        language_id: String,
        message: String,
    },
    /// `textDocument/publishDiagnostics`.
    Diagnostics {
        language_id: String,
        uri: String,
        /// The document version the server diagnosed, when it reports one.
        version: Option<i32>,
        diagnostics: Vec<lsp_types::Diagnostic>,
    },
    /// The server is asking the editor to apply a `WorkspaceEdit`
    /// (`workspace/applyEdit`) — the shape command-driven refactorings take.
    ///
    /// The server is blocked until `gate` is answered, so the receiver must
    /// claim or refuse it rather than dropping it on the floor. `edit` is the
    /// raw `WorkspaceEdit`, parsed by `crate::workspace_edit` on the UI side
    /// where the set of open documents is known.
    ApplyEdit {
        language_id: String,
        /// What the server calls this change, for the preview's title.
        label: Option<String>,
        edit: Value,
        gate: ApplyEditGate,
    },
    /// Any other server-to-client notification, unparsed.
    Notification {
        language_id: String,
        method: String,
        params: Value,
    },
}

/// Why an LSP operation failed.
#[derive(Debug)]
pub enum LspError {
    /// No server is configured/started for that language id.
    NoServer(String),
    /// The server is not currently connected (crashed, or still restarting).
    NotRunning(String),
    /// Spawning the child process failed — usually a missing executable.
    Spawn { command: String, source: io::Error },
    /// Writing to or reading from the server's pipes failed.
    Io(io::Error),
    /// The server answered with a JSON-RPC error.
    Response { code: i64, message: String },
    /// No response within the timeout; a `$/cancelRequest` was sent.
    Timeout { method: String },
    /// The server died while the request was in flight.
    Disconnected { method: String },
    /// The server's payload was not the shape the protocol requires.
    Protocol(String),
}

impl fmt::Display for LspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LspError::NoServer(lang) => write!(f, "no language server configured for {lang}"),
            LspError::NotRunning(lang) => write!(f, "the {lang} language server is not running"),
            LspError::Spawn { command, source } => {
                write!(f, "could not start {command}: {source}")
            }
            LspError::Io(e) => write!(f, "language server I/O failed: {e}"),
            LspError::Response { code, message } => {
                write!(f, "language server error {code}: {message}")
            }
            LspError::Timeout { method } => write!(f, "{method} timed out"),
            LspError::Disconnected { method } => {
                write!(f, "the language server exited during {method}")
            }
            LspError::Protocol(what) => write!(f, "malformed language server message: {what}"),
        }
    }
}

impl std::error::Error for LspError {}

impl From<io::Error> for LspError {
    fn from(e: io::Error) -> Self {
        LspError::Io(e)
    }
}

/// The live child process: its stdin (the one writer) and the handle we reap.
struct Conn {
    stdin: ChildStdin,
    child: Child,
}

/// One language's server: its config, its connection, and the requests
/// currently awaiting a response.
struct Server {
    language_id: String,
    conn: Mutex<Option<Conn>>,
    pending: Mutex<HashMap<i64, Sender<Result<Value, LspError>>>>,
    next_id: AtomicI64,
    stopping: AtomicBool,
    /// Whether the editor currently has a refactoring in flight — read by
    /// `dispatch` to decide whether an inbound `workspace/applyEdit` was
    /// asked for. Shared with the manager, not owned here.
    sessions: Arc<RefactorSessions>,
}

impl Server {
    fn send(&self, message: &Value) -> Result<(), LspError> {
        let payload = serde_json::to_vec(message).map_err(io::Error::from)?;
        let mut guard = self.conn.lock().unwrap();
        let conn = guard
            .as_mut()
            .ok_or_else(|| LspError::NotRunning(self.language_id.clone()))?;
        write_message(&mut conn.stdin, &payload)?;
        Ok(())
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, LspError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.pending.lock().unwrap().insert(id, tx);

        let sent = self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }));
        if let Err(e) = sent {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }

        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                // Best-effort: the server may already be gone.
                let _ = self.notify("$/cancelRequest", json!({ "id": id }));
                Err(LspError::Timeout {
                    method: method.to_string(),
                })
            }
            // The sender is dropped when the connection dies.
            Err(RecvTimeoutError::Disconnected) => Err(LspError::Disconnected {
                method: method.to_string(),
            }),
        }
    }

    /// Fail every in-flight request; called when the connection dies so no
    /// caller waits for a response that can never arrive.
    fn drop_pending(&self) {
        self.pending.lock().unwrap().clear();
    }
}

/// What the manager knows about one open document.
struct DocState {
    language_id: String,
    version: i32,
}

/// Owns the running language servers and the rules around them.
///
/// Deliberately not a `bridge.rs` concern: lifecycle, restart backoff, request
/// correlation and document versions are rules, and the adapter is allowed
/// none (`docs/architecture/layering.md`).
pub struct LspManager {
    /// Workspace root as a `file://` URI, sent in `initialize`.
    root_uri: String,
    servers: Mutex<HashMap<String, Arc<Server>>>,
    supervisors: Mutex<HashMap<String, JoinHandle<()>>>,
    documents: Mutex<HashMap<String, DocState>>,
    events: Sender<LspEvent>,
    /// Refactorings the editor has in flight, which is what makes an inbound
    /// `workspace/applyEdit` legitimate (`crate::apply_edit`).
    sessions: Arc<RefactorSessions>,
}

impl LspManager {
    /// Create a manager for a workspace root, plus the channel every event is
    /// delivered on. The caller owns the receiver — typically a listener
    /// thread that forwards onto the UI thread.
    pub fn new(root_uri: impl Into<String>) -> (Self, Receiver<LspEvent>) {
        let (events, rx) = channel();
        let manager = LspManager {
            root_uri: root_uri.into(),
            servers: Mutex::new(HashMap::new()),
            supervisors: Mutex::new(HashMap::new()),
            documents: Mutex::new(HashMap::new()),
            events,
            sessions: Arc::new(RefactorSessions::default()),
        };
        (manager, rx)
    }

    /// Launch a server and complete its `initialize`/`initialized` handshake.
    ///
    /// Blocks until the server is ready, so a bad command surfaces here as an
    /// error rather than as a silent no-op later. Starting a language that is
    /// already running is a no-op.
    pub fn start(&self, cfg: &ServerConfig) -> Result<(), LspError> {
        if self.servers.lock().unwrap().contains_key(&cfg.language_id) {
            return Ok(());
        }
        let server = Arc::new(Server {
            language_id: cfg.language_id.clone(),
            conn: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(1),
            stopping: AtomicBool::new(false),
            sessions: Arc::clone(&self.sessions),
        });

        let (ready_tx, ready_rx) = channel();
        let handle = spawn_supervisor(
            Arc::clone(&server),
            cfg.clone(),
            self.root_uri.clone(),
            self.events.clone(),
            ready_tx,
        );

        // The handshake runs in the supervisor thread (it must own the reader);
        // the first attempt's outcome comes back here.
        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.servers
                    .lock()
                    .unwrap()
                    .insert(cfg.language_id.clone(), server);
                self.supervisors
                    .lock()
                    .unwrap()
                    .insert(cfg.language_id.clone(), handle);
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                Err(LspError::NotRunning(cfg.language_id.clone()))
            }
        }
    }

    /// Is a server currently connected for this language?
    pub fn is_running(&self, language_id: &str) -> bool {
        self.server(language_id)
            .map(|s| s.conn.lock().unwrap().is_some())
            .unwrap_or(false)
    }

    /// Send a request and wait for its response ([`DEFAULT_REQUEST_TIMEOUT`]).
    pub fn request(
        &self,
        language_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, LspError> {
        self.request_with_timeout(language_id, method, params, DEFAULT_REQUEST_TIMEOUT)
    }

    /// Send a request, cancelling it with `$/cancelRequest` if `timeout`
    /// elapses first.
    pub fn request_with_timeout(
        &self,
        language_id: &str,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, LspError> {
        let server = self
            .server(language_id)
            .ok_or_else(|| LspError::NoServer(language_id.to_string()))?;
        server.request(method, params, timeout)
    }

    /// Send a notification (fire and forget by protocol definition).
    pub fn notify(&self, language_id: &str, method: &str, params: Value) -> Result<(), LspError> {
        let server = self
            .server(language_id)
            .ok_or_else(|| LspError::NoServer(language_id.to_string()))?;
        server.notify(method, params)
    }

    /// Tell the server a document is open. The manager owns the version
    /// counter: versions start at 1 and only ever increase, per document.
    pub fn did_open(&self, uri: &str, language_id: &str, text: &str) -> Result<(), LspError> {
        self.documents.lock().unwrap().insert(
            uri.to_string(),
            DocState {
                language_id: language_id.to_string(),
                version: 1,
            },
        );
        self.notify(
            language_id,
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": language_id, "version": 1, "text": text
            }}),
        )
    }

    /// Tell the server a document changed, as a full-text sync. Returns the
    /// new version.
    pub fn did_change(&self, uri: &str, text: &str) -> Result<i32, LspError> {
        let (language_id, version) = {
            let mut docs = self.documents.lock().unwrap();
            let doc = docs
                .get_mut(uri)
                .ok_or_else(|| LspError::Protocol(format!("{uri} was never opened")))?;
            doc.version += 1;
            (doc.language_id.clone(), doc.version)
        };
        self.notify(
            &language_id,
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": uri, "version": version},
                "contentChanges": [{"text": text}],
            }),
        )?;
        Ok(version)
    }

    /// Tell the server a document was saved.
    pub fn did_save(&self, uri: &str) -> Result<(), LspError> {
        let language_id = self.language_of(uri)?;
        self.notify(
            &language_id,
            "textDocument/didSave",
            json!({"textDocument": {"uri": uri}}),
        )
    }

    /// Tell the server a document is closed and forget its version.
    pub fn did_close(&self, uri: &str) -> Result<(), LspError> {
        let language_id = self.language_of(uri)?;
        self.documents.lock().unwrap().remove(uri);
        self.notify(
            &language_id,
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri}}),
        )
    }

    /// `textDocument/hover` for a position in an open document, already
    /// reduced to the one text the tooltip shows. `Ok(None)` means the server
    /// has nothing to say here, which is not an error.
    pub fn hover(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<HoverText>, LspError> {
        let language_id = self.language_of(uri)?;
        let result = self.request_with_timeout(
            &language_id,
            "textDocument/hover",
            position_params(uri, line, character),
            HOVER_TIMEOUT,
        )?;
        Ok(parse_hover(&result))
    }

    /// `textDocument/definition` for a position in an open document. An empty
    /// vector means the server had no answer; whether that falls back to the
    /// index is [`crate::navigation::definition_outcome`]'s decision.
    pub fn definition(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<DefinitionTarget>, LspError> {
        let language_id = self.language_of(uri)?;
        let result = self.request_with_timeout(
            &language_id,
            "textDocument/definition",
            position_params(uri, line, character),
            DEFINITION_TIMEOUT,
        )?;
        Ok(parse_definition(&result))
    }

    /// `textDocument/completion` for a position in an open document, parsed
    /// across both response shapes. Ordering and filtering are the caller's
    /// next step ([`crate::completion::filter`]), not the manager's.
    pub fn completion(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<CompletionList, LspError> {
        let language_id = self.language_of(uri)?;
        let result = self.request_with_timeout(
            &language_id,
            "textDocument/completion",
            position_params(uri, line, character),
            COMPLETION_TIMEOUT,
        )?;
        Ok(parse_completion(&result))
    }

    /// The version last sent for a document, if it is open.
    pub fn document_version(&self, uri: &str) -> Option<i32> {
        self.documents.lock().unwrap().get(uri).map(|d| d.version)
    }

    /// Shut one server down: `shutdown`, `exit`, then kill if it lingers.
    pub fn stop(&self, language_id: &str) {
        let server = self.servers.lock().unwrap().remove(language_id);
        let handle = self.supervisors.lock().unwrap().remove(language_id);
        let Some(server) = server else { return };

        server.stopping.store(true, Ordering::SeqCst);
        let _ = server.request("shutdown", Value::Null, SHUTDOWN_GRACE);
        let _ = server.notify("exit", Value::Null);

        // Give it the grace period to close its stdout on its own; the
        // supervisor takes the connection when it sees EOF.
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        while Instant::now() < deadline {
            if server.conn.lock().unwrap().is_none() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if let Some(conn) = server.conn.lock().unwrap().as_mut() {
            let _ = conn.child.kill();
        }
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    /// Shut every running server down.
    pub fn stop_all(&self) {
        let languages: Vec<String> = self.servers.lock().unwrap().keys().cloned().collect();
        for language_id in languages {
            self.stop(&language_id);
        }
    }

    fn server(&self, language_id: &str) -> Option<Arc<Server>> {
        self.servers.lock().unwrap().get(language_id).cloned()
    }

    /// Mark a refactoring as in flight for as long as the returned guard
    /// lives.
    ///
    /// This is what makes an inbound `workspace/applyEdit` legitimate: a
    /// server may only rewrite the user's files while the user is asking it
    /// to. Callers hold the guard across the whole gesture — including the
    /// `workspace/executeCommand` that provokes the edit — and dropping it
    /// closes the door again.
    pub fn begin_refactor(&self) -> RefactorSession {
        self.sessions.begin()
    }

    /// Whether a refactoring this client started is in flight right now.
    pub fn refactor_active(&self) -> bool {
        self.sessions.active()
    }

    fn language_of(&self, uri: &str) -> Result<String, LspError> {
        self.documents
            .lock()
            .unwrap()
            .get(uri)
            .map(|d| d.language_id.clone())
            .ok_or_else(|| LspError::Protocol(format!("{uri} was never opened")))
    }
}

impl Drop for LspManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// The per-server supervisor: spawn, handshake, read until EOF, respawn with
/// capped exponential backoff.
fn spawn_supervisor(
    server: Arc<Server>,
    cfg: ServerConfig,
    root_uri: String,
    events: Sender<LspEvent>,
    ready: Sender<Result<(), LspError>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut restarts: u32 = 0;
        let mut ready = Some(ready);
        let mut backoff = RESTART_BACKOFF_INITIAL;

        loop {
            match connect(&server, &cfg, &root_uri) {
                Ok((stdout, trigger_characters)) => {
                    if let Some(tx) = ready.take() {
                        let _ = tx.send(Ok(()));
                    }
                    let _ = events.send(LspEvent::ServerReady {
                        language_id: cfg.language_id.clone(),
                        restarts,
                        trigger_characters,
                    });
                    let started = Instant::now();
                    read_loop(&server, &cfg.language_id, stdout, &events);
                    // A session that ran for a while was healthy: its exit
                    // starts a fresh restart budget rather than counting
                    // towards the crash loop the backoff exists to damp.
                    if started.elapsed() >= HEALTHY_SESSION {
                        restarts = 0;
                        backoff = RESTART_BACKOFF_INITIAL;
                    }
                }
                Err(e) => {
                    let message = e.to_string();
                    if let Some(tx) = ready.take() {
                        let _ = tx.send(Err(e));
                        return;
                    }
                    let _ = events.send(LspEvent::ServerFailed {
                        language_id: cfg.language_id.clone(),
                        message,
                    });
                    return;
                }
            }

            // The connection is gone: reap the child and release waiters.
            if let Some(mut conn) = server.conn.lock().unwrap().take() {
                let _ = conn.child.kill();
                let _ = conn.child.wait();
            }
            server.drop_pending();

            if server.stopping.load(Ordering::SeqCst) {
                return;
            }
            restarts += 1;
            if restarts > MAX_RESTARTS {
                let _ = events.send(LspEvent::ServerFailed {
                    language_id: cfg.language_id.clone(),
                    message: format!("gave up after {MAX_RESTARTS} restarts"),
                });
                return;
            }
            let _ = events.send(LspEvent::ServerExited {
                language_id: cfg.language_id.clone(),
                restarts,
                retry_in: backoff,
            });
            thread::sleep(backoff);
            backoff = (backoff * 2).min(RESTART_BACKOFF_MAX);
        }
    })
}

/// Spawn the child and run the `initialize`/`initialized` handshake, leaving
/// the connection published and the reader positioned at the next message.
fn connect(
    server: &Server,
    cfg: &ServerConfig,
    root_uri: &str,
) -> Result<(BufReader<std::process::ChildStdout>, Vec<String>), LspError> {
    let mut child = Command::new(&cfg.command)
        .args(&cfg.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Servers are chatty on stderr and nothing reads it; a full pipe
        // would deadlock the child.
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| LspError::Spawn {
            command: cfg.command.clone(),
            source,
        })?;

    let mut stdin = child.stdin.take().expect("stdin was piped");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout was piped"));

    // The handshake is done inline, before the connection is published, so
    // nothing else can be in flight and no dispatch table is needed yet.
    let init = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": client_capabilities(),
            "workspaceFolders": Value::Null,
        }
    });
    write_message(
        &mut stdin,
        &serde_json::to_vec(&init).map_err(io::Error::from)?,
    )?;

    let triggers = loop {
        let Some(body) = read_message(&mut stdout)? else {
            return Err(LspError::Disconnected {
                method: "initialize".into(),
            });
        };
        let message: Value = serde_json::from_slice(&body).map_err(io::Error::from)?;
        if message.get("id").and_then(Value::as_i64) == Some(0) && message.get("method").is_none() {
            if let Some(error) = message.get("error") {
                return Err(response_error(error));
            }
            // What the server can do is read here, once, and published with
            // `ServerReady` — nothing else ever sees the raw result.
            break parse_trigger_characters(message.get("result").unwrap_or(&Value::Null));
        }
        // Anything else before the response (log messages, server requests)
        // is dropped: the client isn't observable yet.
    };

    let initialized = json!({"jsonrpc": "2.0", "method": "initialized", "params": {}});
    write_message(
        &mut stdin,
        &serde_json::to_vec(&initialized).map_err(io::Error::from)?,
    )?;
    stdin.flush()?;

    *server.conn.lock().unwrap() = Some(Conn { stdin, child });
    Ok((stdout, triggers))
}

/// Read and dispatch until the server's stdout ends (i.e. it died).
fn read_loop(
    server: &Arc<Server>,
    language_id: &str,
    mut stdout: BufReader<std::process::ChildStdout>,
    events: &Sender<LspEvent>,
) {
    loop {
        match read_message(&mut stdout) {
            Ok(Some(body)) => match serde_json::from_slice::<Value>(&body) {
                Ok(message) => dispatch(server, language_id, message, events),
                // A single unparsable message is not worth killing the
                // session over; the framing is still in sync.
                Err(_) => continue,
            },
            Ok(None) | Err(_) => return,
        }
    }
}

fn dispatch(server: &Arc<Server>, language_id: &str, message: Value, events: &Sender<LspEvent>) {
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id").and_then(Value::as_i64);

    match (method, id) {
        // Response to one of our requests.
        (None, Some(id)) => {
            let Some(tx) = server.pending.lock().unwrap().remove(&id) else {
                return;
            };
            let result = match message.get("error") {
                Some(error) => Err(response_error(error)),
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(result);
        }
        // `workspace/applyEdit` is the one server-to-client request we
        // implement, and the only one that cannot be answered here: applying
        // an edit needs the UI thread, and this is the thread that reads
        // every message from this server — blocking it would stall
        // diagnostics and every in-flight response behind one dialog.
        //
        // So the answer is made elsewhere and this arm only routes. An edit
        // nobody asked for is refused immediately, without a thread and
        // without troubling the UI: that is a server rewriting the user's
        // files unprompted. A wanted one is handed to a short-lived thread
        // that publishes it and waits on the gate, which is bounded — see
        // `crate::apply_edit`. There is at most one such thread per
        // refactoring gesture, because a gesture is what makes the request
        // legitimate in the first place.
        (Some("workspace/applyEdit"), Some(id)) => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            if !server.sessions.active() {
                let _ = server.send(&apply_edit_response(id, false, Some(UNSOLICITED_REASON)));
                return;
            }
            let (gate, rx) = ApplyEditGate::new();
            let _ = events.send(LspEvent::ApplyEdit {
                language_id: language_id.to_string(),
                label: params
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                edit: params.get("edit").cloned().unwrap_or(Value::Null),
                gate: gate.clone(),
            });
            let server = Arc::clone(server);
            thread::spawn(move || {
                let verdict = await_verdict(rx, &gate);
                let _ = server.send(&apply_edit_response(
                    id,
                    verdict.applied(),
                    verdict.reason(),
                ));
            });
        }
        // Every other server-to-client request. The server blocks until it
        // gets an answer, so answer honestly rather than not at all.
        // ponytail: a real handler (workspace/configuration, registerCapability)
        // lands with the features that need it.
        (Some(method), Some(id)) => {
            let _ = server.send(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("{method} is not implemented")},
            }));
        }
        // Notification.
        (Some(method), None) => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let event = if method == "textDocument/publishDiagnostics" {
                publish_diagnostics(language_id, &params)
            } else {
                None
            };
            let _ = events.send(event.unwrap_or(LspEvent::Notification {
                language_id: language_id.to_string(),
                method: method.to_string(),
                params,
            }));
        }
        (None, None) => {}
    }
}

/// The `ApplyWorkspaceEditResult` the protocol expects: `applied`, plus a
/// `failureReason` whenever it is false, so a server can tell the user why
/// its refactoring did not happen.
fn apply_edit_response(id: i64, applied: bool, reason: Option<&str>) -> Value {
    let mut result = json!({"applied": applied});
    if let Some(reason) = reason {
        result["failureReason"] = Value::String(reason.to_string());
    }
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn publish_diagnostics(language_id: &str, params: &Value) -> Option<LspEvent> {
    let uri = params.get("uri")?.as_str()?.to_string();
    let diagnostics =
        serde_json::from_value::<Vec<lsp_types::Diagnostic>>(params.get("diagnostics")?.clone())
            .ok()?;
    Some(LspEvent::Diagnostics {
        language_id: language_id.to_string(),
        uri,
        version: params
            .get("version")
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        diagnostics,
    })
}

/// `TextDocumentPositionParams`: `line` and `character` are 0-based and
/// counted in UTF-16 code units, per the encoding `initialize` negotiates.
fn position_params(uri: &str, line: u32, character: u32) -> Value {
    json!({
        "textDocument": {"uri": uri},
        "position": {"line": line, "character": character},
    })
}

fn response_error(error: &Value) -> LspError {
    LspError::Response {
        code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_string(),
    }
}

/// What this client can do. Kept deliberately small — capabilities are added
/// by the feature tasks that implement them (L2-L5), not speculatively.
fn client_capabilities() -> Value {
    json!({
        "textDocument": {
            "synchronization": {"dynamicRegistration": false},
            "publishDiagnostics": {"relatedInformation": true},
            // L3/L4: advertised because they are implemented — `contentFormat`
            // lists markdown first because that is what the tooltip renders,
            // and `linkSupport` opts into the richer `LocationLink` reply.
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "definition": {"linkSupport": true},
            // L5: `snippetSupport: false` is the honest answer — snippet
            // items are inserted as their plain text, with no tabstops (see
            // `completion::strip_snippet`), so a server that would send
            // placeholder-heavy items is told to prefer plain ones.
            "completion": {
                "completionItem": {
                    "snippetSupport": false,
                    "documentationFormat": ["plaintext", "markdown"],
                },
                "contextSupport": false,
            },
        },
        "general": {"positionEncodings": ["utf-16"]},
    })
}
