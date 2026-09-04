//! Building the project (B1-6): `BuildService`, the QObject that runs a
//! build and publishes what it said.
//!
//! # Threading
//!
//! One thread per build, not one worker for all of them — a build is a
//! single long process read to completion, so there is no queue of short
//! operations to serialize the way `RunService` has. That thread owns the
//! whole sequence: launch step, read to EOF, check the exit status, launch
//! the next step (a rebuild is clean then build), and report.
//!
//! The build's `Supervisor` is shared with the Qt thread as an
//! `Arc<Mutex<..>>` purely so `stop` can reach it. The blocking read happens
//! on the reader taken *out* of the supervisor, never with the lock held —
//! the same arrangement `crate::bridge::run` uses for exactly the same
//! reason.
//!
//! Translation only, per `docs/architecture/layering.md`: which steps a
//! build runs and what its output means are `build-core`'s (ADR-0040).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use build_core::{BuildDiagnostic, BuildKind, BuildSpec, DiagnosticParser, Severity};
use cxx_qt::{CxxQtThread, Threading};
use cxx_qt_lib::QString;
use run_core::toolchain::ToolchainId;
use run_core::LaunchSpec;

use crate::bridge::errors;
use crate::bridge::ffi;

/// What the Qt thread keeps about one running build.
struct BuildHandle {
    /// Shared with the build's own thread so `stop` can kill it. The lock is
    /// only ever held for a supervisor call, never across a read.
    supervisor: Arc<Mutex<run_core::Supervisor>>,
    /// The console the build's current step is running as, so `stop` knows
    /// what to kill. Replaced when a rebuild moves on to its second step.
    console: Arc<Mutex<Option<run_core::ConsoleId>>>,
}

/// Rust side of the `BuildService` QObject.
#[derive(Default)]
pub struct BuildServiceRust {
    next_id: Cell<u64>,
    builds: RefCell<HashMap<u64, BuildHandle>>,
    /// Everything the last build said, in the order it said it. Replaced
    /// wholesale when a new build starts: a diagnostic from a build two
    /// edits ago is worse than no diagnostic, because it looks current.
    diagnostics: RefCell<Vec<BuildDiagnostic>>,
    /// Which tool produced the diagnostics currently held — the `source`
    /// column the Problems dock already shows for a language server's.
    source: RefCell<String>,
}

fn current_project_root() -> Option<PathBuf> {
    crate::bridge::convert::current_project_root()
}

fn no_project() -> ffi::FfiResult {
    ffi::FfiResult {
        code: errors::CODE_NO_PROJECT,
        message: QString::from("no project is open"),
    }
}

fn to_ffi_result(err: &build_core::BuildError) -> ffi::FfiResult {
    ffi::FfiResult {
        code: err.code(),
        message: QString::from(err.to_string().as_str()),
    }
}

fn to_ffi_severity(severity: Severity) -> ffi::FfiSeverity {
    match severity {
        Severity::Error => ffi::FfiSeverity::Error,
        Severity::Warning => ffi::FfiSeverity::Warning,
        Severity::Note => ffi::FfiSeverity::Information,
    }
}

/// A build diagnostic in the shape the Problems dock already renders for a
/// language server's (ADR-0040): same struct, and `source` says which tool
/// produced it so the two can never be confused for each other.
fn to_ffi_diagnostic(diagnostic: &BuildDiagnostic, source: &str) -> ffi::FfiDiagnostic {
    ffi::FfiDiagnostic {
        path: QString::from(diagnostic.path.display().to_string().as_str()),
        line: diagnostic.line,
        column: diagnostic.column,
        // A build reports where a problem starts, never where it ends;
        // repeating the start is what the Problems dock's row needs, and
        // inventing an end would put a made-up range in the editor.
        end_line: diagnostic.line,
        end_column: diagnostic.column,
        severity: to_ffi_severity(diagnostic.severity),
        message: QString::from(diagnostic.message.as_str()),
        source: QString::from(source),
    }
}

/// A human-readable form of what is being run, for the dock's header —
/// `cargo build --message-format=json`, `./gradlew clean`.
fn command_line(spec: &LaunchSpec) -> String {
    let mut line = spec.program.clone();
    for arg in &spec.args {
        line.push(' ');
        line.push_str(arg);
    }
    line
}

impl ffi::BuildService {
    pub fn build(self: Pin<&mut Self>) -> ffi::FfiResult {
        self.start(BuildKind::Build)
    }

    pub fn rebuild(self: Pin<&mut Self>) -> ffi::FfiResult {
        self.start(BuildKind::Rebuild)
    }

    pub fn build_target(self: Pin<&mut Self>, target: &QString) -> ffi::FfiResult {
        self.start(BuildKind::Target(target.to_string()))
    }

    fn start(mut self: Pin<&mut Self>, kind: BuildKind) -> ffi::FfiResult {
        let Some(root) = current_project_root() else {
            return no_project();
        };
        let toolchain = match build_core::buildable_toolchain(&root) {
            Ok(toolchain) => toolchain,
            Err(err) => return to_ffi_result(&err),
        };
        let steps = match BuildSpec::new(toolchain, kind, &root).steps() {
            Ok(steps) => steps,
            Err(err) => return to_ffi_result(&err),
        };

        // A new build's problems replace the last one's: a diagnostic from a
        // build two edits ago looks current and is not.
        self.diagnostics.borrow_mut().clear();
        *self.source.borrow_mut() = toolchain.as_str().to_string();

        let build_id = self.next_id.get() + 1;
        self.next_id.set(build_id);
        let supervisor = Arc::new(Mutex::new(run_core::Supervisor::new()));
        let console = Arc::new(Mutex::new(None));
        self.builds.borrow_mut().insert(
            build_id,
            BuildHandle {
                supervisor: Arc::clone(&supervisor),
                console: Arc::clone(&console),
            },
        );

        let qt_thread = self.as_mut().qt_thread();
        self.as_mut()
            .build_started(build_id, QString::from(command_line(&steps[0]).as_str()));
        self.as_mut().diagnostics_changed();

        std::thread::spawn(move || {
            run_build(BuildRun {
                build_id,
                steps,
                toolchain,
                project_root: root,
                supervisor,
                console,
                qt_thread,
            });
        });
        ffi::FfiResult::default()
    }

    pub fn stop(self: Pin<&mut Self>, build_id: u64) {
        // Cloned out of the map before anything is locked: holding the
        // `RefCell` borrow across a `Mutex` lock would keep the map borrowed
        // while the build's own thread may be queueing a removal onto this
        // one.
        let handle = self
            .builds
            .borrow()
            .get(&build_id)
            .map(|handle| (Arc::clone(&handle.supervisor), Arc::clone(&handle.console)));
        let Some((supervisor, console_slot)) = handle else {
            return;
        };
        let console = match console_slot.lock() {
            Ok(console) => *console,
            Err(_) => return,
        };
        let Some(console) = console else {
            return;
        };
        let Ok(mut supervisor) = supervisor.lock() else {
            return;
        };
        // `Supervisor::stop` kills the whole tree, not just the direct
        // child: `cargo` spawns `rustc` and `gradle` spawns a daemon, so
        // killing one process would leave the build running (ADR-0040).
        let _ = supervisor.stop(console);
    }

    pub fn is_building(&self) -> bool {
        !self.builds.borrow().is_empty()
    }

    pub fn diagnostics(&self) -> Vec<ffi::FfiDiagnostic> {
        let source = self.source.borrow().clone();
        self.diagnostics
            .borrow()
            .iter()
            .map(|diagnostic| to_ffi_diagnostic(diagnostic, &source))
            .collect()
    }
}

/// Everything one build's thread needs. A struct rather than eight
/// parameters, which is what the clippy gate asks for anyway.
struct BuildRun {
    build_id: u64,
    steps: Vec<LaunchSpec>,
    toolchain: ToolchainId,
    project_root: PathBuf,
    supervisor: Arc<Mutex<run_core::Supervisor>>,
    console: Arc<Mutex<Option<run_core::ConsoleId>>>,
    qt_thread: CxxQtThread<ffi::BuildService>,
}

/// The whole build, on its own thread: every step in order, stopping at the
/// first one that fails, then one `buildFinished`.
fn run_build(run: BuildRun) {
    let BuildRun {
        build_id,
        steps,
        toolchain,
        project_root,
        supervisor,
        console,
        qt_thread,
    } = run;

    let mut exit_code = 0i32;
    for (index, step) in steps.iter().enumerate() {
        if index > 0 {
            let line = command_line(step);
            queue_output(&qt_thread, build_id, &format!("\n$ {line}\n"));
        }
        match run_step(RunStep {
            build_id,
            step,
            toolchain,
            project_root: &project_root,
            supervisor: &supervisor,
            console: &console,
            qt_thread: &qt_thread,
        }) {
            Ok(code) => exit_code = code,
            Err(message) => {
                queue_output(&qt_thread, build_id, &format!("{message}\n"));
                exit_code = -1;
            }
        }
        if exit_code != 0 {
            break;
        }
    }

    let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::BuildService>| {
        service.builds.borrow_mut().remove(&build_id);
        service.as_mut().build_finished(build_id, exit_code);
    });
}

struct RunStep<'a> {
    build_id: u64,
    step: &'a LaunchSpec,
    toolchain: ToolchainId,
    project_root: &'a Path,
    supervisor: &'a Arc<Mutex<run_core::Supervisor>>,
    console: &'a Arc<Mutex<Option<run_core::ConsoleId>>>,
    qt_thread: &'a CxxQtThread<ffi::BuildService>,
}

/// One step: launch, read to EOF while parsing, and report its exit code.
fn run_step(step: RunStep<'_>) -> Result<i32, String> {
    let (id, reader) = {
        let mut supervisor = step.supervisor.lock().map_err(|_| "build lock poisoned")?;
        let id = supervisor
            .launch(format!("build-{}", step.build_id), step.step)
            .map_err(|err| err.to_string())?;
        let reader = supervisor.take_reader(id).map_err(|err| err.to_string())?;
        (id, reader)
    };
    *step.console.lock().map_err(|_| "console lock poisoned")? = Some(id);

    read_to_end(&step, reader);

    let mut supervisor = step.supervisor.lock().map_err(|_| "build lock poisoned")?;
    let code = supervisor
        .exit_code(id)
        .map_err(|err| err.to_string())?
        .unwrap_or(0);
    Ok(code as i32)
}

/// Read this step's output until the process closes it, publishing text and
/// diagnostics as they arrive rather than at the end — the Problems dock
/// fills while the build is still running.
fn read_to_end(step: &RunStep<'_>, mut reader: Box<dyn Read + Send>) {
    let mut parser = DiagnosticParser::new(step.toolchain, step.project_root);
    let mut ansi = crate::bridge::run::AnsiStripper::default();
    let mut buffer = [0u8; 8192];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        // Lossy: a build tool writing invalid UTF-8 is not a reason to lose
        // the rest of its output.
        let chunk = String::from_utf8_lossy(&buffer[..read]).into_owned();
        // Stripped before parsing: a build over a PTY colours its
        // diagnostics, and `error:` wrapped in SGR codes matches nothing
        // (ADR-0040).
        let visible = ansi.feed(&chunk);
        publish(step, parser.feed(&visible));
        queue_output(step.qt_thread, step.build_id, &visible);
    }
    publish(step, parser.finish());
}

fn publish(step: &RunStep<'_>, diagnostics: Vec<BuildDiagnostic>) {
    if diagnostics.is_empty() {
        return;
    }
    let _ = step
        .qt_thread
        .queue(move |mut service: Pin<&mut ffi::BuildService>| {
            service.diagnostics.borrow_mut().extend(diagnostics);
            service.as_mut().diagnostics_changed();
        });
}

fn queue_output(qt_thread: &CxxQtThread<ffi::BuildService>, build_id: u64, text: &str) {
    let text = text.to_string();
    let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::BuildService>| {
        service
            .as_mut()
            .build_output(build_id, QString::from(text.as_str()));
    });
}
