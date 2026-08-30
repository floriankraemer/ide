//! Run configurations and console (F4-9): `RunService`, the `Threading`
//! QObject that owns one `run_core::Supervisor`.
//!
//! # Threading
//!
//! One worker thread owns the `Supervisor` — a launch, a `kill_tree()`, and
//! reading a batcher's pending bytes must not run on the UI thread, the same
//! reason `VcsService` owns its `Repository` on a worker thread
//! (`crate::bridge::vcs`). That alone would only get one console read at a
//! time, though: `Supervisor::read_output` needs a chunk the caller has
//! already read, and `PtySession::read` blocks until bytes arrive, so a
//! second running console would sit unread for as long as the first one's
//! shell stays quiet.
//!
//! So each launch also spawns a **dedicated reader thread for that console**
//! (`docs/architecture/next-five-features-plan.md`'s "the same \[shape\],
//! plus one reader thread per console") that owns the raw PTY read half —
//! obtained via `Supervisor::take_reader`, exactly `PtySession::take_reader`
//! passed one level up — and does nothing but block on `read()` in a loop.
//! Each chunk it reads is sent as a new job into the **same** channel the Qt
//! thread's `run`/`stop`/`detectConfigurations` calls already use, so every
//! operation against the `Supervisor` — including feeding it a chunk through
//! `read_output` — is still serialized through the one worker thread that
//! owns it exclusively. `run_core::Supervisor`'s own doc comment asks for
//! exactly this: "single-threaded... expects its caller to serialize
//! access". Concurrency lives in *how many threads produce jobs*, never in
//! shared mutable access to the `Supervisor` itself.
//!
//! `pty_core::PtySession`'s fields are all `Send` trait objects, so
//! `run_core::Supervisor` (a `BTreeMap` of PTY sessions, batchers and
//! strings) is `Send` as a whole — confirmed by the compiler accepting the
//! `move` into the worker thread's closure below, not asserted separately.

use std::cell::RefCell;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::mpsc::Sender;

use cxx_qt::{CxxQtThread, Threading};
use cxx_qt_lib::QString;
use run_core::RunConfigExt;

use crate::bridge::ffi;

mod editor;
pub use editor::RunConfigEditorRust;

/// One unit of work for the worker thread that owns the `Supervisor`.
/// Mirrors `VcsJob` (`crate::bridge::vcs`): every call against the
/// `Supervisor` — a launch, a stop, or a console's reader thread handing
/// over a freshly read chunk — is one closure through this queue, so nothing
/// ever touches the `Supervisor` from two threads at once.
type RunJob = Box<dyn FnOnce(&mut RunWorker) + Send>;

/// What the worker thread owns.
struct RunWorker {
    supervisor: run_core::Supervisor,
    /// Kept so a job (the launch job, specifically) can hand a clone to a
    /// new console's reader thread — the one thing a reader thread needs to
    /// feed bytes back into this same queue.
    jobs_tx: Sender<RunJob>,
}

/// What the Qt thread remembers about one active console: enough to answer
/// `resolveLink` with no worker round trip, and to know which configuration
/// `rerun` should relaunch. `output` mirrors what `consoleOutput` has
/// already emitted (not a second copy of `run-core`'s own ring buffer,
/// which is private to `Supervisor`) and is capped at the same
/// `run_core::batching::MAX_RING_BYTES` bound for the same reason.
struct ConsoleState {
    config_id: String,
    cwd: PathBuf,
    output: String,
    /// F4-11 v1: console text is displayed and cached with ANSI/VT escapes
    /// removed rather than rendered as color (a `QPlainTextEdit` has no SGR
    /// support of its own, and the plan's `FfiStyledRun` design — SGR parsed
    /// into styled runs in Rust, one `QTextCharFormat` per run in C++ — is
    /// real work this branch's time budget didn't reach). Stripping here,
    /// once, keeps `output` (what `resolveLink` byte-offsets index into) and
    /// what `consoleOutput` sends the view in sync — both are the same
    /// visible text — so link resolution stays correct even though the
    /// stream it is computed from is not the raw one `run-core` batched.
    /// Follow-up: replace this field and `strip_ansi_stateful` with real SGR
    /// parsing and an `FfiStyledRun` signal.
    ansi: AnsiStripper,
    /// Set once, by whichever of `finish_console`/`stop` first observes this
    /// console has exited, so the other does not also emit `consoleFinished`
    /// (see both call sites). The entry itself is kept around afterwards,
    /// not removed: `resolveLink` must still answer for a finished console's
    /// scrollback — F4-11's dock deliberately leaves a finished console's
    /// tab open for exactly that review — so the cache it reads from has to
    /// outlive the process it was collected from.
    finished: bool,
}

/// Removes ANSI/VT escape sequences (SGR color codes, cursor moves, OSC
/// hyperlinks/titles) from streamed process output, byte-by-byte and
/// statefully: a batch boundary can split a sequence mid-way — the same
/// concern `run_core::batching`'s "ansi state survives a batch boundary"
/// test covers for `resolve_link` — so an unterminated sequence's state
/// carries over to the next `feed` call instead of leaking stray bytes into
/// the visible text or corrupting the next chunk's parse.
///
/// Scans byte-by-byte rather than char-by-char, but this never splits a
/// multi-byte UTF-8 character: every byte this parser treats specially
/// (ESC, `[`, `]`, BEL, `\`, and the CSI final-byte range `0x40..=0x7E`) is
/// ASCII (< 0x80), and UTF-8 continuation bytes are always >= 0x80, so they
/// are never mistaken for one of these markers and always fall through
/// untouched in `Normal` state.
#[derive(Default)]
struct AnsiStripper {
    state: AnsiState,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum AnsiState {
    #[default]
    Normal,
    /// Saw ESC; the next byte says what kind of sequence follows.
    Escape,
    /// Inside a CSI (`ESC [ ... final`) sequence, awaiting the final byte.
    Csi,
    /// Inside an OSC (`ESC ] ... terminator`) sequence, awaiting BEL or ST
    /// (`ESC \`).
    Osc,
    /// Inside an OSC sequence, just saw ESC — one more byte says whether
    /// this is ST (`\`) or an unrelated ESC that keeps the OSC going.
    OscEscape,
}

impl AnsiStripper {
    fn feed(&mut self, text: &str) -> String {
        let mut out = Vec::with_capacity(text.len());
        for &byte in text.as_bytes() {
            self.state = match (self.state, byte) {
                (AnsiState::Normal, 0x1B) => AnsiState::Escape,
                (AnsiState::Normal, _) => {
                    out.push(byte);
                    AnsiState::Normal
                }
                (AnsiState::Escape, b'[') => AnsiState::Csi,
                (AnsiState::Escape, b']') => AnsiState::Osc,
                // Any other byte after a lone ESC is a two-byte sequence
                // (e.g. `ESC M`) — fully consumed by this one byte.
                (AnsiState::Escape, _) => AnsiState::Normal,
                (AnsiState::Csi, 0x40..=0x7E) => AnsiState::Normal,
                (AnsiState::Csi, _) => AnsiState::Csi,
                (AnsiState::Osc, 0x07) => AnsiState::Normal,
                (AnsiState::Osc, 0x1B) => AnsiState::OscEscape,
                (AnsiState::Osc, _) => AnsiState::Osc,
                (AnsiState::OscEscape, b'\\') => AnsiState::Normal,
                (AnsiState::OscEscape, _) => AnsiState::Osc,
            };
        }
        // Safe: every dropped byte belonged to an escape sequence, and every
        // sequence marker is ASCII (see the struct's doc comment), so `out`
        // is a concatenation of untouched, already-valid UTF-8 spans.
        String::from_utf8(out).unwrap_or_default()
    }
}

#[cfg(test)]
mod ansi_strip_tests {
    use super::AnsiStripper;

    #[test]
    fn passes_plain_text_through_unchanged() {
        assert_eq!(AnsiStripper::default().feed("hello\nworld"), "hello\nworld");
    }

    #[test]
    fn strips_an_sgr_color_sequence() {
        assert_eq!(
            AnsiStripper::default().feed("\x1b[31merror\x1b[0m: oops"),
            "error: oops"
        );
    }

    #[test]
    fn strips_a_two_byte_escape() {
        assert_eq!(AnsiStripper::default().feed("a\x1bMb"), "ab");
    }

    #[test]
    fn strips_an_osc_hyperlink_terminated_by_bel() {
        assert_eq!(
            AnsiStripper::default().feed("\x1b]8;;file:///x\x07link\x1b]8;;\x07"),
            "link"
        );
    }

    #[test]
    fn strips_an_osc_sequence_terminated_by_string_terminator() {
        assert_eq!(
            AnsiStripper::default().feed("\x1b]0;title\x1b\\visible"),
            "visible"
        );
    }

    #[test]
    fn a_sequence_split_across_two_batches_is_still_stripped() {
        let mut stripper = AnsiStripper::default();
        let mut visible = stripper.feed("before\x1b[3");
        visible.push_str(&stripper.feed("1mafter"));
        assert_eq!(visible, "beforeafter");
    }

    #[test]
    fn preserves_non_ascii_text_around_a_stripped_sequence() {
        assert_eq!(
            AnsiStripper::default().feed("caf\u{e9} \x1b[1mbold\x1b[0m \u{1F600}"),
            "caf\u{e9} bold \u{1F600}"
        );
    }
}

/// Rust side of the `RunService` QObject.
#[derive(Default)]
pub struct RunServiceRust {
    jobs: RefCell<Option<Sender<RunJob>>>,
    consoles: RefCell<std::collections::HashMap<u64, ConsoleState>>,
}

/// The project root `AppSession` currently has open, or `None` with no
/// project open. `RunService` needs no `openProject` of its own the way
/// `VcsService` does: `Supervisor` has no per-project state to reset (a
/// launched console is just a process, indifferent to which project is
/// open), so the root is read fresh from the one shared session on every
/// call that needs it — the same handle `ProjectTreeModel`/`DocumentManager`
/// already share (`crate::bridge::registry`).
fn current_project_root() -> Option<PathBuf> {
    crate::bridge::convert::current_project_root()
}

/// `err`, as the typed code + message that crosses the FFI seam (ADR-0003).
/// `run-core`'s own codes (800-899, `error.rs`) are already stable, so this
/// is a field copy, not a translation.
fn to_ffi_result(err: &run_core::RunError) -> ffi::FfiResult {
    ffi::FfiResult {
        code: err.code(),
        message: QString::from(err.to_string().as_str()),
    }
}

fn env_to_string(env: &[(String, String)]) -> String {
    env.iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The inverse of [`env_to_string`]. A line with no `=` is dropped rather
/// than guessed at — better an env var silently missing from the draft than
/// one invented with an empty value from a stray line.
fn env_from_string(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            line.split_once('=')
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn to_ffi_run_config(config: &run_core::RunConfig) -> ffi::FfiRunConfig {
    ffi::FfiRunConfig {
        id: QString::from(config.id.as_str()),
        name: QString::from(config.name.as_str()),
        program: QString::from(config.program.as_str()),
        args: QString::from(config.args.join(" ").as_str()),
        cwd: QString::from(config.cwd.clone().unwrap_or_default().as_str()),
        env: QString::from(env_to_string(&config.env).as_str()),
    }
}

/// Trim `output` down to `max_bytes` from the front, on a UTF-8 char
/// boundary — never a raw byte cut, which could split a multi-byte
/// character and produce invalid UTF-8 in a `String`.
fn cap_cached_output(output: &mut String, max_bytes: usize) {
    if output.len() <= max_bytes {
        return;
    }
    let mut boundary = output.len() - max_bytes;
    while boundary < output.len() && !output.is_char_boundary(boundary) {
        boundary += 1;
    }
    output.drain(..boundary);
}

/// Queue every event a batch produced onto the Qt thread: text as
/// `consoleOutput` (appended to this console's cached output for
/// `resolveLink`), a dropped-history notice as `consoleTruncated`.
fn emit_events(
    console_id: u64,
    events: Vec<run_core::BatchedOutput>,
    qt_thread: &CxxQtThread<ffi::RunService>,
) {
    for event in events {
        match event {
            run_core::BatchedOutput::Output(text) => {
                let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::RunService>| {
                    let visible = {
                        let mut consoles = service.consoles.borrow_mut();
                        let Some(state) = consoles.get_mut(&console_id) else {
                            return;
                        };
                        let visible = state.ansi.feed(&text);
                        state.output.push_str(&visible);
                        cap_cached_output(&mut state.output, run_core::batching::MAX_RING_BYTES);
                        visible
                    };
                    service
                        .as_mut()
                        .console_output(console_id, QString::from(visible.as_str()));
                });
            }
            run_core::BatchedOutput::Truncated => {
                let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::RunService>| {
                    service.as_mut().console_truncated(console_id);
                });
            }
        }
    }
}

/// Spawn the dedicated reader thread for a freshly launched console: block
/// on `read()` in a loop, feeding every chunk back into the job queue as a
/// `read_output` call, until EOF (the process exited) or the channel is
/// gone (the app is shutting down).
fn spawn_reader_thread(
    console_id: u64,
    mut reader: Box<dyn std::io::Read + Send>,
    jobs_tx: Sender<RunJob>,
    qt_thread: CxxQtThread<ffi::RunService>,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let qt_thread = qt_thread.clone();
                    let _ = jobs_tx.send(Box::new(move |worker: &mut RunWorker| {
                        finish_console(worker, run_core::ConsoleId(console_id), &qt_thread);
                    }));
                    break;
                }
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    let qt_thread = qt_thread.clone();
                    let sent = jobs_tx.send(Box::new(move |worker: &mut RunWorker| {
                        let now = std::time::Instant::now();
                        if let Ok(events) = worker.supervisor.read_output(
                            run_core::ConsoleId(console_id),
                            &chunk,
                            now,
                        ) {
                            emit_events(console_id, events, &qt_thread);
                        }
                    }));
                    if sent.is_err() {
                        break; // The worker thread is gone (app shutdown).
                    }
                }
                Err(_) => break,
            }
        }
    });
}

/// A console's process exited on its own (its reader thread hit EOF): flush
/// whatever output was still pending, best-effort recover its exit code,
/// reap it via `Supervisor::stop` (a no-op kill on an already-dead process
/// — see `pty_core::platform::kill_tree`'s `ESRCH` case — so this is really
/// just "stop tracking it"), and report it finished.
fn finish_console(
    worker: &mut RunWorker,
    id: run_core::ConsoleId,
    qt_thread: &CxxQtThread<ffi::RunService>,
) {
    let now = std::time::Instant::now();
    if let Ok(events) = worker.supervisor.flush_remaining(id, now) {
        emit_events(id.0, events, qt_thread);
    }

    // The exit code is usually available the instant EOF is seen, but not
    // guaranteed to be — give the OS a brief moment to reap the child.
    let mut exit_code = None;
    for _ in 0..20 {
        match worker.supervisor.exit_code(id) {
            Ok(Some(code)) => {
                exit_code = Some(code);
                break;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
            Err(_) => break,
        }
    }

    let escaped = matches!(
        worker.supervisor.stop(id),
        Ok(pty_core::KillOutcome::Escaped)
    );
    let exit_code = exit_code.map_or(-1, |code| code as i32);
    let console_id = id.0;
    let qt_thread = qt_thread.clone();
    let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::RunService>| {
        // The console may already be marked finished if an explicit
        // `stop()` won the race with this natural-exit path — both call
        // `Supervisor::stop`, and only the first to run here should emit.
        // The entry itself is never removed (see `ConsoleState::finished`'s
        // doc comment): `resolveLink` still has to answer for it.
        if mark_finished(&mut service.consoles.borrow_mut(), console_id) {
            service
                .as_mut()
                .console_finished(console_id, exit_code, escaped);
        }
    });
}

/// Marks `console_id` finished and returns whether this call was the one
/// that did it — `false` if it was already finished (or is unknown), so a
/// caller emits `consoleFinished` at most once per console. Shared by
/// `finish_console` and `RunService::stop`, the two paths that can each
/// observe the same exit.
fn mark_finished(
    consoles: &mut std::collections::HashMap<u64, ConsoleState>,
    console_id: u64,
) -> bool {
    match consoles.get_mut(&console_id) {
        Some(state) if !state.finished => {
            state.finished = true;
            true
        }
        _ => false,
    }
}

impl ffi::RunService {
    /// The `Sender` jobs are pushed through, starting the worker thread on
    /// first use — there is no per-project reset to hang this on (see this
    /// module's doc comment), so lazy start is simpler than an `openProject`
    /// nothing else needs.
    fn ensure_worker(self: Pin<&mut Self>) -> Sender<RunJob> {
        if let Some(tx) = self.jobs.borrow().clone() {
            return tx;
        }
        let (tx, rx) = std::sync::mpsc::channel::<RunJob>();
        let mut worker = RunWorker {
            supervisor: run_core::Supervisor::new(),
            jobs_tx: tx.clone(),
        };
        std::thread::spawn(move || {
            for job in rx {
                job(&mut worker);
            }
        });
        *self.jobs.borrow_mut() = Some(tx.clone());
        tx
    }

    pub fn configurations(&self) -> Vec<ffi::FfiRunConfig> {
        let Some(root) = current_project_root() else {
            return Vec::new();
        };
        app_config::project_settings::load(&root)
            .unwrap_or_default()
            .run_configs
            .unwrap_or_default()
            .iter()
            .map(to_ffi_run_config)
            .collect()
    }

    pub fn detect_configurations(mut self: Pin<&mut Self>) {
        let Some(root) = current_project_root() else {
            return;
        };
        let tx = self.as_mut().ensure_worker();
        let qt_thread = self.as_mut().qt_thread();
        let _ = tx.send(Box::new(move |_worker: &mut RunWorker| {
            let result = app_config::project_settings::update(&root, |settings| {
                let existing = settings.run_configs.clone().unwrap_or_default();
                let detected = run_core::detect(&root);
                settings.run_configs = Some(run_core::merge_detected(&existing, detected));
            });
            if result.is_ok() {
                let _ = qt_thread.queue(|mut service: Pin<&mut ffi::RunService>| {
                    service.as_mut().configurations_changed();
                });
            }
        }));
    }

    pub fn run(mut self: Pin<&mut Self>, config_id: &QString) -> ffi::FfiResult {
        let config_id = config_id.to_string();
        let Some(root) = current_project_root() else {
            return ffi::FfiResult {
                code: 1,
                message: QString::from("no project is open"),
            };
        };
        let configs = app_config::project_settings::load(&root)
            .unwrap_or_default()
            .run_configs
            .unwrap_or_default();
        let Some(config) = configs.into_iter().find(|c| c.id == config_id) else {
            return ffi::FfiResult {
                code: 1,
                message: QString::from("unknown run configuration"),
            };
        };

        let mut spec = config.to_launch_spec(&root);
        let cwd = spec.cwd.clone().unwrap_or_else(|| root.clone());
        // `to_launch_spec` leaves `cwd` as `None` for a configuration with
        // no explicit working directory (`run_core::config::to_launch_spec`
        // makes no project-root assumption of its own — that is this
        // bridge's job, per its own `cwd` field doc comment). Filling it in
        // here, rather than trusting `pty_core`'s spawn to fall back to
        // something sane, is what actually launches the process in the
        // project root instead of wherever the IDE process itself happened
        // to start from.
        spec.cwd = Some(cwd.clone());

        let tx = self.as_mut().ensure_worker();
        let qt_thread = self.qt_thread();
        let launch_config_id = config_id.clone();
        let _ = tx.send(Box::new(move |worker: &mut RunWorker| {
            match worker.supervisor.launch(launch_config_id.clone(), &spec) {
                Ok(id) => {
                    let reader = worker.supervisor.take_reader(id);
                    let console_id = id.0;
                    let started_config_id = launch_config_id.clone();
                    let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::RunService>| {
                        service.consoles.borrow_mut().insert(
                            console_id,
                            ConsoleState {
                                config_id: started_config_id.clone(),
                                cwd,
                                output: String::new(),
                                ansi: AnsiStripper::default(),
                                finished: false,
                            },
                        );
                        service
                            .as_mut()
                            .console_started(console_id, QString::from(started_config_id.as_str()));
                    });
                    if let Ok(reader) = reader {
                        spawn_reader_thread(
                            console_id,
                            reader,
                            worker.jobs_tx.clone(),
                            qt_thread.clone(),
                        );
                    }
                }
                Err(err) => {
                    let result = to_ffi_result(&err);
                    let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::RunService>| {
                        service
                            .as_mut()
                            .run_failed(QString::from(launch_config_id.as_str()), result);
                    });
                }
            }
        }));
        ffi::FfiResult::default()
    }

    pub fn stop(mut self: Pin<&mut Self>, console_id: u64) {
        let Some(tx) = self.jobs.borrow().clone() else {
            return;
        };
        let qt_thread = self.as_mut().qt_thread();
        let _ = tx.send(Box::new(move |worker: &mut RunWorker| {
            let id = run_core::ConsoleId(console_id);
            let now = std::time::Instant::now();
            if let Ok(events) = worker.supervisor.flush_remaining(id, now) {
                emit_events(console_id, events, &qt_thread);
            }
            if let Ok(outcome) = worker.supervisor.stop(id) {
                let escaped = matches!(outcome, pty_core::KillOutcome::Escaped);
                let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::RunService>| {
                    if mark_finished(&mut service.consoles.borrow_mut(), console_id) {
                        service.as_mut().console_finished(console_id, -1, escaped);
                    }
                });
            }
        }));
    }

    pub fn rerun(mut self: Pin<&mut Self>, console_id: u64) -> ffi::FfiResult {
        let config_id = match self.consoles.borrow().get(&console_id) {
            Some(state) => state.config_id.clone(),
            None => {
                return ffi::FfiResult {
                    code: 1,
                    message: QString::from("unknown console"),
                }
            }
        };
        self.as_mut().stop(console_id);
        self.as_mut().run(&QString::from(config_id.as_str()))
    }

    pub fn resolve_link(&self, console_id: u64, byte_offset: u32) -> ffi::FfiResolvedLink {
        let consoles = self.consoles.borrow();
        let Some(state) = consoles.get(&console_id) else {
            return ffi::FfiResolvedLink::default();
        };
        match run_core::resolve_link(&state.output, byte_offset as usize, &state.cwd) {
            Some(link) => ffi::FfiResolvedLink {
                found: true,
                path: QString::from(link.path.display().to_string().as_str()),
                line: link.line,
                has_column: link.col.is_some(),
                column: link.col.unwrap_or(0),
            },
            None => ffi::FfiResolvedLink::default(),
        }
    }
}
