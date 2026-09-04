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
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::mpsc::Sender;

use cxx_qt::{CxxQtThread, Threading};
use cxx_qt_lib::QString;
use run_core::{AnsiStripper, RunConfigExt};

use crate::bridge::errors;
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

fn no_project() -> ffi::FfiResult {
    ffi::FfiResult {
        code: errors::CODE_NO_PROJECT,
        message: QString::from("no project is open"),
    }
}

fn unknown_run_config(message: &str) -> ffi::FfiResult {
    ffi::FfiResult {
        code: errors::CODE_UNKNOWN_RUN_CONFIG,
        message: QString::from(message),
    }
}

/// The before-launch list as the dialog shows and accepts it: one task per
/// line. Parsing it back is `tasks_from_string`; both live here rather than
/// in `run-core` because the *text* is a dialog affordance, not a rule.
fn tasks_to_string(config: &run_core::RunConfig) -> String {
    run_core::before_launch::tasks_of(config)
        .iter()
        .map(|task| match task {
            run_core::BeforeLaunchTask::Build => "build".to_string(),
            run_core::BeforeLaunchTask::RunConfiguration(id) => format!("run {id}"),
            run_core::BeforeLaunchTask::ExternalTool { program, args } => {
                let mut line = format!("tool {program}");
                for arg in args {
                    line.push(' ');
                    line.push_str(arg);
                }
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The inverse. A line this version cannot read is dropped rather than
/// guessed at, the same rule the persisted form follows.
fn tasks_from_string(text: &str) -> Vec<app_config::BeforeLaunchSetting> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let task = match (words.next()?, words) {
                ("build", _) => run_core::BeforeLaunchTask::Build,
                ("run", mut rest) => {
                    run_core::BeforeLaunchTask::RunConfiguration(rest.next()?.to_string())
                }
                ("tool", mut rest) => run_core::BeforeLaunchTask::ExternalTool {
                    program: rest.next()?.to_string(),
                    args: rest.map(str::to_string).collect(),
                },
                _ => return None,
            };
            Some(task.to_setting())
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
        toolchain: QString::from(config.toolchain.clone().unwrap_or_default().as_str()),
        target: QString::from(config.target.clone().unwrap_or_default().as_str()),
        temporary: config.temporary,
        allow_parallel: config.allow_parallel,
        before_launch: QString::from(tasks_to_string(config).as_str()),
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
/// Run one configuration's before-launch tasks, in order, stopping at the
/// first failure. Returns whether the launch may proceed.
///
/// Every task is a process run to completion, which is exactly what
/// `build_core::runner` does — including a Build task, whose steps come from
/// the same `BuildSpec` the Build dock uses. There is no second way to run a
/// build in this codebase (ADR-0040).
fn run_before_launch(
    tasks: &[run_core::BeforeLaunchTask],
    config_id: &str,
    root: &Path,
    configs: &[run_core::RunConfig],
    qt_thread: &CxxQtThread<ffi::RunService>,
) -> bool {
    for task in tasks {
        let (label, steps, toolchain) = match resolve_task(task, root, configs) {
            Ok(resolved) => resolved,
            Err(message) => {
                fail_before_launch(qt_thread, config_id, &message);
                return false;
            }
        };

        let started_id = config_id.to_string();
        let started_label = label.clone();
        let started_thread = qt_thread.clone();
        let _ = started_thread.queue(move |mut service: Pin<&mut ffi::RunService>| {
            service.as_mut().before_launch_started(
                QString::from(started_id.as_str()),
                QString::from(started_label.as_str()),
            );
        });

        let mut sink = BeforeLaunchSink {
            config_id: config_id.to_string(),
            qt_thread: qt_thread.clone(),
        };
        let handle = build_core::BuildHandle::new();
        let outcome = build_core::runner::run(&handle, &steps, toolchain, root, &mut sink);
        if !outcome.succeeded() {
            fail_before_launch(
                qt_thread,
                config_id,
                &format!("{label} failed (exit {})", outcome.exit_code),
            );
            return false;
        }
    }
    true
}

/// What one task actually runs: a label for the dock, the steps, and the
/// toolchain whose diagnostics the output is read as.
type ResolvedTask = (String, Vec<run_core::LaunchSpec>, run_core::ToolchainId);

fn resolve_task(
    task: &run_core::BeforeLaunchTask,
    root: &Path,
    configs: &[run_core::RunConfig],
) -> Result<ResolvedTask, String> {
    match task {
        run_core::BeforeLaunchTask::Build => {
            let toolchain = build_core::buildable_toolchain(root).map_err(|err| err.to_string())?;
            let steps = build_core::BuildSpec::new(toolchain, build_core::BuildKind::Build, root)
                .steps()
                .map_err(|err| err.to_string())?;
            Ok(("Build".to_string(), steps, toolchain))
        }
        run_core::BeforeLaunchTask::RunConfiguration(id) => {
            let config = configs
                .iter()
                .find(|c| &c.id == id)
                .ok_or_else(|| format!("unknown run configuration \"{id}\""))?;
            let mut spec = config.to_launch_spec(root);
            spec.cwd = Some(spec.cwd.clone().unwrap_or_else(|| root.to_path_buf()));
            Ok((config.name.clone(), vec![spec], tool_for_output(root)))
        }
        run_core::BeforeLaunchTask::ExternalTool { program, args } => {
            let spec = run_core::LaunchSpec {
                program: program.clone(),
                args: args.clone(),
                cwd: Some(root.to_path_buf()),
                env: Vec::new(),
                console: run_core::ConsoleKind::Pty,
            };
            Ok((program.clone(), vec![spec], tool_for_output(root)))
        }
    }
}

/// Which parser a non-build task's output is read with. The project's own
/// toolchain, so a script that runs the compiler still produces recognisable
/// diagnostics; `Make` (the text table) when the project has no build tool,
/// because guessing Cargo's JSON for arbitrary output would find nothing.
fn tool_for_output(root: &Path) -> run_core::ToolchainId {
    build_core::buildable_toolchain(root).unwrap_or(run_core::ToolchainId::Make)
}

fn fail_before_launch(qt_thread: &CxxQtThread<ffi::RunService>, config_id: &str, message: &str) {
    let config_id = config_id.to_string();
    let message = message.to_string();
    let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::RunService>| {
        service.as_mut().before_launch_failed(
            QString::from(config_id.as_str()),
            ffi::FfiResult {
                code: errors::CODE_BEFORE_LAUNCH,
                message: QString::from(message.as_str()),
            },
        );
    });
}

/// Streams a before-launch task's output to the Build dock. Diagnostics are
/// deliberately dropped: the Problems dock is fed by `BuildService`, and a
/// second writer to it from here would leave rows nothing clears. The
/// errors themselves are still visible — they are in the output.
struct BeforeLaunchSink {
    config_id: String,
    qt_thread: CxxQtThread<ffi::RunService>,
}

impl build_core::BuildSink for BeforeLaunchSink {
    fn output(&mut self, text: &str) {
        let config_id = self.config_id.clone();
        let text = text.to_string();
        let _ = self
            .qt_thread
            .queue(move |mut service: Pin<&mut ffi::RunService>| {
                service.as_mut().before_launch_output(
                    QString::from(config_id.as_str()),
                    QString::from(text.as_str()),
                );
            });
    }

    fn diagnostics(&mut self, _diagnostics: Vec<build_core::BuildDiagnostic>) {}
}

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
            return no_project();
        };
        let configs = app_config::project_settings::load(&root)
            .unwrap_or_default()
            .run_configs
            .unwrap_or_default();
        let Some(config) = configs.into_iter().find(|c| c.id == config_id) else {
            return unknown_run_config("unknown run configuration");
        };

        let context = run_core::MacroContext::for_project(&root);
        self.as_mut().launch(config, &root, &context)
    }

    pub fn can_run_file(&self, path: &QString) -> bool {
        let Some(root) = current_project_root() else {
            return false;
        };
        run_core::config_for_file(&root, Path::new(&path.to_string())).is_some()
    }

    pub fn run_context(mut self: Pin<&mut Self>, path: &QString) -> ffi::FfiResult {
        let Some(root) = current_project_root() else {
            return no_project();
        };
        let file = PathBuf::from(path.to_string());
        let Some(config) = run_core::config_for_file(&root, &file) else {
            return unknown_run_config("this file has no run target");
        };

        // Persist before launching: the toolbar's combo, the console tab and
        // a later rerun all key on a configuration the settings file knows
        // about, so a temporary one that never reached disk would run once
        // and then be unrerunnable.
        let remembered = {
            let config = config.clone();
            app_config::project_settings::update(&root, move |settings| {
                let mut configs = settings.run_configs.clone().unwrap_or_default();
                run_core::remember_temporary(&mut configs, config.clone());
                settings.run_configs = Some(configs);
            })
        };
        if remembered.is_ok() {
            self.as_mut().configurations_changed();
        }

        let context = run_core::MacroContext::for_file(&root, &file);
        self.as_mut().launch(config, &root, &context)
    }

    /// Launch `config`, honouring its parallel-run policy: unless the
    /// configuration allows parallel runs, its still-running consoles are
    /// stopped first, which is IntelliJ's default ("Allow multiple
    /// instances" off) and the reason a second Run does not quietly leave
    /// two servers holding the same port.
    fn launch(
        mut self: Pin<&mut Self>,
        config: run_core::RunConfig,
        root: &Path,
        context: &run_core::MacroContext,
    ) -> ffi::FfiResult {
        let config_id = config.id.clone();
        if !config.allow_parallel {
            let running: Vec<u64> = self
                .consoles
                .borrow()
                .iter()
                .filter(|(_, state)| !state.finished && state.config_id == config_id)
                .map(|(id, _)| *id)
                .collect();
            for console_id in running {
                self.as_mut().stop(console_id);
            }
        }

        let root = root.to_path_buf();
        let mut spec = config.to_launch_spec_in(context);
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

        // Before-launch tasks are validated here, on the Qt thread, before
        // anything runs: a cycle discovered halfway through would already
        // have started processes the user then has to kill one at a time
        // (B2-3).
        let configs = app_config::project_settings::load(&root)
            .unwrap_or_default()
            .run_configs
            .unwrap_or_default();
        if let Err(err) = run_core::before_launch::validate(&config_id, &configs) {
            return ffi::FfiResult {
                code: errors::CODE_BEFORE_LAUNCH,
                message: QString::from(err.to_string().as_str()),
            };
        }
        let tasks = run_core::before_launch::tasks_of(&config);

        let tx = self.as_mut().ensure_worker();
        let qt_thread = self.qt_thread();
        let launch_config_id = config_id.clone();
        let task_root = root.clone();
        let _ = tx.send(Box::new(move |worker: &mut RunWorker| {
            // Sequential and fail-fast, on the worker thread: a run whose
            // build failed must not start, and the user finds out in the
            // Build dock rather than by reading a program's output for
            // errors that are not its own (B2-2).
            if !run_before_launch(&tasks, &launch_config_id, &task_root, &configs, &qt_thread) {
                return;
            }
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
                    code: errors::CODE_UNKNOWN_CONSOLE,
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
