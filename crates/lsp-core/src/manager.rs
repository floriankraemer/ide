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
use std::path::PathBuf;
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
use crate::code_action::{
    filter_by_kind, needs_unfiltered_retry, parse_code_actions, CodeActionItem, CommandRef,
};
use crate::completion::{parse_completion, parse_trigger_characters, CompletionList};
use crate::configuration;
use crate::document_highlight::{parse_document_highlights, DocumentHighlight};
use crate::formatting::{parse_formatting, FormattingOptions, FormattingOutcome};
use crate::framing::{read_message, write_message};
use crate::hover::{parse_hover, HoverText};
use crate::inlay_hint::{line_range, parse_inlay_hints, InlayHint};
use crate::intentions::{assemble, Intention, ORGANIZE_IMPORTS};
use crate::navigation::{parse_definition, DefinitionTarget};
use crate::progress::{ProgressTracker, ServerActivity};
use crate::registration::{Registration, Registrations};
use crate::rename::{parse_prepare_rename, PrepareRename};
use crate::signature_help::{
    parse_signature_help, parse_signature_triggers, SignatureHelp, SignatureTriggers,
};
use crate::watched_files::{FileChangeKind, WatchedFiles};
use crate::workspace_edit::{parse_workspace_edit, DocumentEdits};

/// How long a request waits for its response before it is cancelled.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Hover is speculative and mouse-driven: an answer that arrives seconds
/// after the dwell is not worth showing, so it gets its own short deadline.
pub const HOVER_TIMEOUT: Duration = Duration::from_secs(2);
/// Completion is asked for per keystroke and the popup is only useful while
/// the word it describes is still being typed, so it gets the shortest
/// deadline of all: a list that lands later is discarded anyway.
pub const COMPLETION_TIMEOUT: Duration = Duration::from_secs(3);
/// Refactoring — code actions, rename, and the commands they run. Generous
/// next to the others because a server may have to analyse a whole project
/// to answer, and an Extract on a large file legitimately takes seconds; the
/// user asked for this one and is waiting on it, unlike a hover they may
/// already have moved past.
pub const REFACTOR_TIMEOUT: Duration = Duration::from_secs(30);
/// Go to Definition is an explicit gesture and the user is waiting for it,
/// but a jump that lands half a minute later is a bug, not a jump.
pub const DEFINITION_TIMEOUT: Duration = Duration::from_secs(5);

/// Reformatting a whole file is real work — rustfmt on a large file, or a
/// formatter that shells out — so this is generous compared with hover or
/// completion. It is still bounded: an editor that hangs on Ctrl+Alt+L is
/// worse than one that says the formatter took too long.
pub const FORMATTING_TIMEOUT: Duration = Duration::from_secs(15);

/// JSON-RPC's "method not found". A server answers this for a request it
/// does not implement, which is how an unsupported capability is discovered
/// without having read every field of its `initialize` result.
const METHOD_NOT_FOUND: i64 = -32601;
/// Signature help is retriggered on `(` and every `,` while an argument
/// list is being typed, so it is as speculative as hover and gets the same
/// deadline: a tip for an argument the user has finished typing is noise.
pub const SIGNATURE_HELP_TIMEOUT: Duration = Duration::from_secs(2);
/// Document highlights fire on every caret move and are purely decorative.
/// One second is deliberately the shortest deadline in this file: an answer
/// slower than that describes a caret position the user has already left,
/// and painting it would highlight the wrong word.
pub const DOCUMENT_HIGHLIGHT_TIMEOUT: Duration = Duration::from_secs(1);
/// Inlay hints cost the server a real inference pass over the viewport, and
/// unlike the caret-driven requests their answer does not go stale — a hint
/// is anchored to a line, so it is still correct when it lands late. Hence
/// longer than hover, and still far short of a refactoring.
pub const INLAY_HINT_TIMEOUT: Duration = Duration::from_secs(5);
/// Alt+Enter, which opens a popup that is not drawn until the list arrives.
/// The user is waiting with nothing on screen, so this cannot be
/// [`REFACTOR_TIMEOUT`]; *applying* the action they then choose still is,
/// because by then they have committed to waiting.
pub const INTENTION_TIMEOUT: Duration = Duration::from_secs(5);
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
        /// F2-9: whether this server offers signature help at all, and which
        /// characters trigger and retrigger it — from the same `initialize`
        /// result, read once for the same reason `trigger_characters` is.
        signature_triggers: SignatureTriggers,
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
    /// F0-16: what the server is working on, from its `$/progress`
    /// notifications. `activity` is `None` when it has no work open, i.e.
    /// it is idle and its answers can be trusted.
    ///
    /// Emitted only when the visible activity changes, and only by servers
    /// that report progress at all. A server that never sends `$/progress`
    /// never sends this event and is idle from [`LspEvent::ServerReady`]
    /// onwards — nothing waits on it, the state is advisory.
    ServerBusy {
        language_id: String,
        activity: Option<ServerActivity>,
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

impl LspError {
    pub const CODE_NO_SERVER: i32 = 600;
    pub const CODE_NOT_RUNNING: i32 = 601;
    pub const CODE_SPAWN: i32 = 602;
    pub const CODE_IO: i32 = 603;
    pub const CODE_RESPONSE: i32 = 604;
    pub const CODE_TIMEOUT: i32 = 605;
    pub const CODE_DISCONNECTED: i32 = 606;
    pub const CODE_PROTOCOL: i32 = 607;

    /// The variant's stable numeric code (ADR-0003 §4: 600–699 is
    /// `lsp-core`'s range, shared with [`crate::workspace_edit::EditError`]).
    /// Append-only.
    ///
    /// Note that [`LspError::Response`] carries the *server's* JSON-RPC
    /// code, which is a different numbering entirely and stays in the
    /// message; this code says only "the server answered with an error".
    pub fn code(&self) -> i32 {
        match self {
            LspError::NoServer(_) => Self::CODE_NO_SERVER,
            LspError::NotRunning(_) => Self::CODE_NOT_RUNNING,
            LspError::Spawn { .. } => Self::CODE_SPAWN,
            LspError::Io(_) => Self::CODE_IO,
            LspError::Response { .. } => Self::CODE_RESPONSE,
            LspError::Timeout { .. } => Self::CODE_TIMEOUT,
            LspError::Disconnected { .. } => Self::CODE_DISCONNECTED,
            LspError::Protocol(_) => Self::CODE_PROTOCOL,
        }
    }
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
    /// F0-16: the `$/progress` work this server currently has open. Owned
    /// per server because a token is only unique within one server, and
    /// touched only from that server's reader thread and its supervisor.
    progress: Mutex<ProgressTracker>,
    /// Whether the editor currently has a refactoring in flight — read by
    /// `dispatch` to decide whether an inbound `workspace/applyEdit` was
    /// asked for. Shared with the manager, not owned here.
    sessions: Arc<RefactorSessions>,
    /// C4: what this server has asked us to watch for it via
    /// `client/registerCapability`, keyed by registration id like the
    /// protocol keys it. Per server, like `progress`, because a
    /// registration id is only unique within one server's session.
    registrations: Registrations,
    /// C6: the `workspace/configuration` section this server pulls its
    /// settings from, from the `ServerConfig` it was launched with. Fixed
    /// for the server's lifetime — changing it means relaunching with a
    /// different config, same as `command`/`args`.
    settings_section: Option<String>,
    /// C6: the settings blob answered for `settings_section`, mutable via
    /// [`LspManager::update_settings`] without a relaunch.
    settings: Mutex<Value>,
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
            progress: Mutex::new(ProgressTracker::default()),
            sessions: Arc::clone(&self.sessions),
            registrations: Registrations::default(),
            settings_section: cfg.settings_section.clone(),
            settings: Mutex::new(cfg.settings.clone()),
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

    /// Tell a server about filesystem changes it asked to watch
    /// (`client/registerCapability` → `workspace/didChangeWatchedFiles`,
    /// C4/C5), off `project_model::ProjectWatcher` rather than a second
    /// watcher of this client's own. `changes` is filtered down to what
    /// this server's current registrations actually cover and sent as one
    /// batched notification — LSP's `changes` param is already an array,
    /// so this is one notification per call, not one per file. A server
    /// with no matching registration, or no server running for
    /// `language_id` at all, gets nothing: this never wakes a server that
    /// asked for no watches.
    pub fn did_change_watched_files(
        &self,
        language_id: &str,
        changes: &[(PathBuf, FileChangeKind)],
    ) -> Result<(), LspError> {
        let Some(server) = self.server(language_id) else {
            return Ok(());
        };
        // Registration is rare (once per server session, typically), so
        // recompiling on every call — rather than caching the compiled
        // `GlobSet` on `Server` and invalidating it on register/unregister
        // — is the simpler correct choice here.
        let watched = WatchedFiles::compile(&server.registrations.watchers());
        if watched.is_empty() {
            return Ok(());
        }
        let interesting: Vec<Value> = changes
            .iter()
            .filter(|(path, kind)| watched.interested(path, *kind))
            .map(|(path, kind)| {
                json!({
                    "uri": crate::diagnostics::uri_from_path(&path.to_string_lossy()),
                    "type": *kind as u8,
                })
            })
            .collect();
        if interesting.is_empty() {
            return Ok(());
        }
        server.notify(
            "workspace/didChangeWatchedFiles",
            json!({"changes": interesting}),
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

    /// `textDocument/codeAction` for a range of an open document.
    ///
    /// `only` narrows the request to a kind family (`refactor.extract`), or
    /// is empty for "everything you have". It is only ever a hint — see
    /// [`crate::code_action::needs_unfiltered_retry`] for what an empty
    /// answer to a filtered request does and does not prove.
    pub fn code_action(
        &self,
        uri: &str,
        start: (u32, u32),
        end: (u32, u32),
        only: &[&str],
    ) -> Result<Vec<CodeActionItem>, LspError> {
        self.code_action_scoped(uri, start, end, only, &[], REFACTOR_TIMEOUT)
    }

    /// One `textDocument/codeAction` request, with both of the things that
    /// scope it: a kind filter and the diagnostics the answer should address.
    ///
    /// Handing the diagnostics back in `context.diagnostics` is not optional
    /// decoration — several servers return their quick fixes *only* for
    /// diagnostics they were given, because a fix is computed from the
    /// diagnostic's own `data`.
    fn code_action_scoped(
        &self,
        uri: &str,
        start: (u32, u32),
        end: (u32, u32),
        only: &[&str],
        diagnostics: &[Value],
        timeout: Duration,
    ) -> Result<Vec<CodeActionItem>, LspError> {
        let language_id = self.language_of(uri)?;
        let mut context = json!({"diagnostics": diagnostics});
        if !only.is_empty() {
            context["only"] = json!(only);
        }
        let result = self.request_with_timeout(
            &language_id,
            "textDocument/codeAction",
            json!({
                "textDocument": {"uri": uri},
                "range": {
                    "start": {"line": start.0, "character": start.1},
                    "end": {"line": end.0, "character": end.1},
                },
                "context": context,
            }),
            timeout,
        )?;
        Ok(parse_code_actions(&result))
    }

    /// Everything that can be done at the caret: the list Alt+Enter shows.
    ///
    /// Two requests, merged by [`crate::intentions::assemble`] — one scoped
    /// to `diagnostics` (the diagnostics under the caret, handed back
    /// verbatim as the protocol requires) and one to the range alone. A
    /// quick fix for the error under the cursor and a refactoring available
    /// at that position are both "things I can do here", and the user should
    /// not have to choose which kind they meant before asking.
    ///
    /// Neither request carries an `only` filter, so there is nothing for
    /// [`crate::code_action::needs_unfiltered_retry`] to retry: an empty
    /// answer to an unfiltered request really does mean the server has
    /// nothing. The retry exists for [`Self::organize_imports`], which does
    /// filter.
    ///
    /// A failure of the diagnostic-scoped request is not a failure of the
    /// whole list — a server that rejects a diagnostic payload it does not
    /// recognise should still get to offer its refactorings — so only the
    /// range-scoped request can fail this method.
    pub fn intentions(
        &self,
        uri: &str,
        start: (u32, u32),
        end: (u32, u32),
        diagnostics: &[Value],
    ) -> Result<Vec<Intention>, LspError> {
        let diagnostic_scoped = if diagnostics.is_empty() {
            Vec::new()
        } else {
            self.code_action_scoped(uri, start, end, &[], diagnostics, INTENTION_TIMEOUT)
                .unwrap_or_default()
        };
        let range_scoped = self.code_action_scoped(uri, start, end, &[], &[], INTENTION_TIMEOUT)?;
        Ok(assemble(&diagnostic_scoped, &range_scoped))
    }

    /// The `source.organizeImports` action for a whole document, if the
    /// server has one.
    ///
    /// Asked with an `only` filter first, because a whole-document code
    /// action request is expensive and there is exactly one action wanted.
    /// An empty answer to that filtered request proves nothing — servers
    /// disagree about whether `only` is a filter or a hint, and some answer
    /// nothing at all for a kind they do not recognise — so
    /// [`crate::code_action::needs_unfiltered_retry`] sends it again
    /// unfiltered and the taxonomy is applied here, where it is understood.
    pub fn organize_imports(
        &self,
        uri: &str,
        last_line: u32,
    ) -> Result<Option<CodeActionItem>, LspError> {
        let (start, end) = ((0, 0), (last_line, u32::MAX));
        let filtered =
            self.code_action_scoped(uri, start, end, &[ORGANIZE_IMPORTS], &[], REFACTOR_TIMEOUT)?;
        let items = if needs_unfiltered_retry(&filtered) {
            let all = self.code_action_scoped(uri, start, end, &[], &[], REFACTOR_TIMEOUT)?;
            filter_by_kind(&all, ORGANIZE_IMPORTS)
        } else {
            filter_by_kind(&filtered, ORGANIZE_IMPORTS)
        };
        Ok(items.into_iter().next())
    }

    /// `codeAction/resolve` for an item the server sent without an edit.
    ///
    /// The item goes back exactly as it arrived — its `data` is the server's
    /// own bookkeeping, and editing it would break the round trip.
    pub fn resolve_code_action(
        &self,
        language_id: &str,
        item: &CodeActionItem,
    ) -> Result<Vec<CodeActionItem>, LspError> {
        let result = self.request_with_timeout(
            language_id,
            "codeAction/resolve",
            item.raw.clone(),
            REFACTOR_TIMEOUT,
        )?;
        Ok(parse_code_actions(&json!([result])))
    }

    /// `workspace/executeCommand`: ask the server to carry out a command a
    /// code action named.
    ///
    /// This is the request during which a server may ask us to apply an edit
    /// (`crate::apply_edit`), so the caller must be holding a
    /// [`LspManager::begin_refactor`] guard — without one the edit that
    /// results is refused as unsolicited, and the refactoring quietly does
    /// nothing.
    pub fn execute_command(
        &self,
        language_id: &str,
        command: &CommandRef,
    ) -> Result<Value, LspError> {
        self.request_with_timeout(
            language_id,
            "workspace/executeCommand",
            json!({"command": command.command, "arguments": command.arguments}),
            REFACTOR_TIMEOUT,
        )
    }

    /// `textDocument/prepareRename`: may the symbol at this position be
    /// renamed, and what should the input be prefilled with?
    ///
    /// `Ok(None)` is the server saying "not this element" — a refusal — while
    /// an `Err` is it saying nothing at all, which most servers do because
    /// they do not implement the request. [`crate::rename::prepare_outcome`]
    /// is what tells those two apart.
    pub fn prepare_rename(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<PrepareRename>, LspError> {
        let language_id = self.language_of(uri)?;
        let result = self.request_with_timeout(
            &language_id,
            "textDocument/prepareRename",
            position_params(uri, line, character),
            REFACTOR_TIMEOUT,
        )?;
        Ok(parse_prepare_rename(&result))
    }

    /// `textDocument/rename`, parsed into the documents it changes.
    ///
    /// An empty result means the server had no answer; whether that falls
    /// back to the name-based index is
    /// [`crate::rename::rename_outcome`]'s decision, not this method's.
    pub fn rename(
        &self,
        uri: &str,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Result<Vec<DocumentEdits>, LspError> {
        let language_id = self.language_of(uri)?;
        let mut params = position_params(uri, line, character);
        params["newName"] = Value::String(new_name.to_string());
        let result = self.request_with_timeout(
            &language_id,
            "textDocument/rename",
            params,
            REFACTOR_TIMEOUT,
        )?;
        parse_workspace_edit(&result).map_err(|e| LspError::Protocol(e.to_string()))
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

    /// `textDocument/formatting` for a whole open document.
    ///
    /// A server that does not implement formatting answers with
    /// `MethodNotFound` rather than an empty list, so that is mapped to
    /// [`FormattingOutcome::Unsupported`] here: from the user's side
    /// "there is no formatter for this language" and "the file is already
    /// formatted" are different messages, and only one of them is a
    /// disappointment worth explaining.
    pub fn format(
        &self,
        uri: &str,
        options: &FormattingOptions,
    ) -> Result<FormattingOutcome, LspError> {
        let language_id = self.language_of(uri)?;
        let params = json!({
            "textDocument": {"uri": uri},
            "options": options.to_json(),
        });
        match self.request_with_timeout(
            &language_id,
            "textDocument/formatting",
            params,
            FORMATTING_TIMEOUT,
        ) {
            Ok(result) => Ok(parse_formatting(&result)),
            Err(LspError::Response { code, .. }) if code == METHOD_NOT_FOUND => {
                Ok(FormattingOutcome::Unsupported)
            }
            Err(err) => Err(err),
        }
    }

    /// `textDocument/rangeFormatting` for a selection.
    ///
    /// Servers commonly implement one of the two and not the other, so this
    /// falls back to whole-document formatting when the range variant is
    /// unsupported — reformatting more than was asked is a better answer
    /// than reformatting nothing, and the preview shows what changed either
    /// way.
    pub fn format_range(
        &self,
        uri: &str,
        start: (u32, u32),
        end: (u32, u32),
        options: &FormattingOptions,
    ) -> Result<FormattingOutcome, LspError> {
        let language_id = self.language_of(uri)?;
        let params = json!({
            "textDocument": {"uri": uri},
            "range": {
                "start": {"line": start.0, "character": start.1},
                "end": {"line": end.0, "character": end.1},
            },
            "options": options.to_json(),
        });
        match self.request_with_timeout(
            &language_id,
            "textDocument/rangeFormatting",
            params,
            FORMATTING_TIMEOUT,
        ) {
            Ok(result) => match parse_formatting(&result) {
                FormattingOutcome::Unsupported => self.format(uri, options),
                outcome => Ok(outcome),
            },
            Err(LspError::Response { code, .. }) if code == METHOD_NOT_FOUND => {
                self.format(uri, options)
            }
            Err(err) => Err(err),
        }
    }

    /// `textDocument/signatureHelp` for a position in an open document.
    /// `Ok(None)` means the caret is not in a call the server recognises.
    ///
    /// The server's `activeParameter` describes the position it was asked
    /// about; by the time the answer lands the caret may have moved, which
    /// is what [`crate::signature_help::call_site_at`] exists to correct.
    pub fn signature_help(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<SignatureHelp>, LspError> {
        let language_id = self.language_of(uri)?;
        let result = self.request_with_timeout(
            &language_id,
            "textDocument/signatureHelp",
            position_params(uri, line, character),
            SIGNATURE_HELP_TIMEOUT,
        )?;
        Ok(parse_signature_help(&result))
    }

    /// `textDocument/documentHighlight`: the occurrences of the symbol under
    /// the caret in this file, each with what it does to the symbol. An
    /// empty vector means the caret is not on a symbol, which is not an
    /// error.
    pub fn document_highlights(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<DocumentHighlight>, LspError> {
        let language_id = self.language_of(uri)?;
        let result = self.request_with_timeout(
            &language_id,
            "textDocument/documentHighlight",
            position_params(uri, line, character),
            DOCUMENT_HIGHLIGHT_TIMEOUT,
        )?;
        Ok(parse_document_highlights(&result))
    }

    /// `textDocument/inlayHint` for the visible lines, inclusive.
    ///
    /// The line range is the request's whole point (see
    /// [`crate::inlay_hint`]): there is no whole-document form of this
    /// method, because a 10,000-line file must not be asked for 10,000
    /// hints to paint fifty.
    pub fn inlay_hints(
        &self,
        uri: &str,
        first_line: u32,
        last_line: u32,
    ) -> Result<Vec<InlayHint>, LspError> {
        let language_id = self.language_of(uri)?;
        let result = self.request_with_timeout(
            &language_id,
            "textDocument/inlayHint",
            json!({"textDocument": {"uri": uri}, "range": line_range(first_line, last_line)}),
            INLAY_HINT_TIMEOUT,
        )?;
        Ok(parse_inlay_hints(&result))
    }

    /// The version last sent for a document, if it is open.
    pub fn document_version(&self, uri: &str) -> Option<i32> {
        self.documents.lock().unwrap().get(uri).map(|d| d.version)
    }

    /// Whether the running server has dynamically registered `method` via
    /// `client/registerCapability`. `false` for a server that is not
    /// running at all, same as "it never registered anything".
    pub fn method_registered(&self, language_id: &str, method: &str) -> bool {
        self.server(language_id)
            .is_some_and(|server| server.registrations.method_registered(method))
    }

    /// C6: update the settings a running server pulls via
    /// `workspace/configuration` and tell it to re-pull them.
    ///
    /// The notification's `settings` is deliberately `null`, not `settings`
    /// itself — that is what tells a client-supports-pull server (csharp-ls
    /// included) to re-issue `workspace/configuration` rather than treat the
    /// notification as the new value pushed inline.
    pub fn update_settings(&self, language_id: &str, settings: Value) -> Result<(), LspError> {
        let server = self
            .server(language_id)
            .ok_or_else(|| LspError::NoServer(language_id.to_string()))?;
        *server.settings.lock().unwrap() = settings;
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings": Value::Null}),
        )
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
                Ok((stdout, trigger_characters, signature_triggers)) => {
                    if let Some(tx) = ready.take() {
                        let _ = tx.send(Ok(()));
                    }
                    let _ = events.send(LspEvent::ServerReady {
                        language_id: cfg.language_id.clone(),
                        restarts,
                        trigger_characters,
                        signature_triggers,
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
            // Work a dead server left open can never end, and a status bar
            // stuck on its last percentage would outlive the server itself.
            if server.progress.lock().unwrap().clear() {
                let _ = events.send(LspEvent::ServerBusy {
                    language_id: cfg.language_id.clone(),
                    activity: None,
                });
            }

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
) -> Result<
    (
        BufReader<std::process::ChildStdout>,
        Vec<String>,
        SignatureTriggers,
    ),
    LspError,
> {
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

    let (trigger_characters, signature_triggers) = loop {
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
            let result = message.get("result").unwrap_or(&Value::Null);
            break (
                parse_trigger_characters(result),
                parse_signature_triggers(result),
            );
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
    Ok((stdout, trigger_characters, signature_triggers))
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
        // F0-16: the server asking permission to open a progress token.
        // There is nothing to decide — the client advertised
        // `window.workDoneProgress`, so the answer is always yes, and the
        // token itself arrives with the `$/progress` that follows. Answered
        // here rather than falling through to "not implemented" below,
        // which is what would otherwise make a server stop reporting.
        (Some("window/workDoneProgress/create"), Some(id)) => {
            let _ = server.send(&json!({"jsonrpc": "2.0", "id": id, "result": Value::Null}));
        }
        // C4: csharp-ls (and any server that leans on dynamic registration
        // rather than declaring capabilities in `initialize`) sends this
        // right after `initialized`. There is nothing to decide — this
        // client always accepts a registration, same reasoning as
        // `window/workDoneProgress/create` above — and no blocking work, so
        // it is answered inline here rather than dispatched elsewhere.
        // Malformed params (missing/wrong-shaped fields) are treated as an
        // empty registration list rather than crashing this reader thread;
        // refusing a registration the server would only retry is worse than
        // ignoring one this client could not parse.
        (Some("client/registerCapability"), Some(id)) => {
            let registrations = message
                .get("params")
                .and_then(|p| p.get("registrations"))
                .cloned()
                .and_then(|v| serde_json::from_value::<Vec<Registration>>(v).ok())
                .unwrap_or_default();
            server.registrations.register(registrations);
            let _ = server.send(&json!({"jsonrpc": "2.0", "id": id, "result": Value::Null}));
        }
        // The spec really does spell this "unregisterations".
        (Some("client/unregisterCapability"), Some(id)) => {
            let ids: Vec<String> = message
                .get("params")
                .and_then(|p| p.get("unregisterations"))
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|e| e.get("id").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            server.registrations.unregister(&ids);
            let _ = server.send(&json!({"jsonrpc": "2.0", "id": id, "result": Value::Null}));
        }
        // C6: csharp-ls pulls its settings rather than taking them pushed,
        // so it sends this right after `initialized` (and again after
        // `workspace/didChangeConfiguration`). A pure lookup against this
        // server's own configured section — nothing to decide, nothing to
        // block on — so it is answered inline here, same reasoning as
        // `window/workDoneProgress/create` and `client/registerCapability`
        // above. Answers are returned in request order, `null` for any
        // section this client has no opinion on (ADR-0016: single-root, so
        // `scopeUri` is ignorable).
        (Some("workspace/configuration"), Some(id)) => {
            let items = message
                .get("params")
                .and_then(|p| p.get("items"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let settings = server.settings.lock().unwrap().clone();
            let result: Vec<Value> = items
                .iter()
                .map(|item| {
                    let section = item.get("section").and_then(Value::as_str).unwrap_or("");
                    configuration::resolve(server.settings_section.as_deref(), &settings, section)
                })
                .collect();
            let _ = server.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
        }
        // Every other server-to-client request. The server blocks until it
        // gets an answer, so answer honestly rather than not at all.
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
            } else if method == "$/progress" {
                // Handled on the reader thread like any other notification:
                // the tracker is a `Mutex` around a `Vec`, so this costs
                // nothing and adds no thread. `apply` says whether anything
                // visible changed, which is what keeps a server reporting
                // every percent from flooding the channel with no-ops.
                let mut progress = server.progress.lock().unwrap();
                if !progress.apply(&params) {
                    return;
                }
                Some(LspEvent::ServerBusy {
                    language_id: language_id.to_string(),
                    activity: progress.current(),
                })
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
            // RF6: code actions as literals rather than bare commands, so an
            // action can carry its own edit; `resolveSupport` names `edit`
            // only, because that is the one field we ask a server to fill in
            // later. The kind list is the families the UI offers — servers
            // may answer with any kind, and `code_action::kind_matches`
            // classifies what arrives, so this list narrows requests without
            // limiting what can come back.
            "codeAction": {
                "codeActionLiteralSupport": {"codeActionKind": {"valueSet": [
                    "", "quickfix", "refactor", "refactor.extract",
                    "refactor.inline", "refactor.rewrite", "source",
                    // F2: Organize Imports is offered in its own right and
                    // as a quick fix for an unresolved symbol, so the kind
                    // is named rather than left to the `source` family.
                    "source.organizeImports",
                ]}},
                "resolveSupport": {"properties": ["edit"]},
                "dataSupport": true,
                "isPreferredSupport": true,
                "disabledSupport": true,
            },
            "rename": {"prepareSupport": true},
            // F1: advertised because reformat is implemented. `dynamicRegistration`
            // is false throughout this client — a server that wants to register
            // capabilities later has nowhere to send them.
            "formatting": {"dynamicRegistration": false},
            "rangeFormatting": {"dynamicRegistration": false},
            // F2: parameter hints. `labelOffsetSupport` says we prefer the
            // unambiguous `[start, end]` parameter label — a substring has
            // to be searched for in the signature and can match the wrong
            // occurrence — but both shapes are handled either way
            // (`signature_help::parse_signature_help`).
            // `activeParameterSupport` opts into the per-signature index,
            // which is the only way an overload set can say that *this*
            // overload takes fewer arguments.
            "signatureHelp": {
                "signatureInformation": {
                    "documentationFormat": ["plaintext", "markdown"],
                    "parameterInformation": {"labelOffsetSupport": true},
                    "activeParameterSupport": true,
                },
                "contextSupport": false,
            },
            "documentHighlight": {"dynamicRegistration": false},
            // No `resolveSupport`: hints are requested for a viewport and
            // painted whole, so there is no second round trip to opt into.
            // The `InlayHintLabelPart[]` label form needs no capability and
            // is parsed regardless.
            "inlayHint": {"dynamicRegistration": false},
        },
        "workspace": {
            // RF5: we answer `workspace/applyEdit`, which is how the
            // command-driven refactorings reach us at all.
            "applyEdit": true,
            "executeCommand": {"dynamicRegistration": false},
            "workspaceEdit": {
                // Versions let a stale edit be caught before it is applied.
                "documentChanges": true,
                // F2: create, rename and delete are performed by
                // `app_core::AppSession::apply_file_ops` (F2). Without
                // these advertised, rust-analyzer's "move to submodule" and
                // every extract-to-new-file refactoring is refused whole —
                // the user sees "unsupported" for a correct edit.
                "resourceOperations": ["create", "rename", "delete"],
                // We apply all of an edit or none of it.
                "failureHandling": "abort",
                "normalizesLineEndings": false,
            },
            // C4: the one capability this client dynamically registers for
            // — csharp-ls and others declare their watched-file globs this
            // way rather than up front. `relativePatternSupport: false`
            // because `Registrations::watchers` hands `globPattern` on
            // untouched to C5, which does not yet resolve a `RelativePattern`
            // against a base URI.
            "didChangeWatchedFiles": {
                "dynamicRegistration": true,
                "relativePatternSupport": false,
            },
            // C6: we answer `workspace/configuration`, which is how
            // csharp-ls (and any server that pulls rather than takes pushed
            // settings) gets its config at all.
            "configuration": true,
        },
        // F0-16: without this a server has no permission to open a progress
        // token, and rust-analyzer stays silent while it indexes — which is
        // exactly the window in which it answers every request with nothing.
        "window": {"workDoneProgress": true},
        "general": {"positionEncodings": ["utf-16"]},
    })
}
