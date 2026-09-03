//! Building the project (B1-6): `BuildService`, the QObject that runs a
//! build and publishes what it said.
//!
//! # Threading
//!
//! One thread per build, not one worker for all of them — a build is a
//! single long process read to completion, so there is no queue of short
//! operations to serialize the way `RunService` has. Running the steps and
//! reading them is `build_core::runner`'s, which blocks on whatever thread
//! it is given; this adapter gives it one and turns its callbacks into
//! signals. The before-launch path (B2) runs the same function on the run
//! worker's thread, which is why it lives there and not here.
//!
//! Translation only, per `docs/architecture/layering.md`: which steps a
//! build runs and what its output means are `build-core`'s (ADR-0040).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;

use build_core::{BuildDiagnostic, BuildKind, BuildSpec, Severity};
use cxx_qt::{CxxQtThread, Threading};
use cxx_qt_lib::QString;

use crate::bridge::errors;
use crate::bridge::ffi;

/// Rust side of the `BuildService` QObject.
#[derive(Default)]
pub struct BuildServiceRust {
    next_id: Cell<u64>,
    builds: RefCell<HashMap<u64, build_core::BuildHandle>>,
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
        let handle = build_core::BuildHandle::new();
        self.builds.borrow_mut().insert(build_id, handle.clone());

        let qt_thread = self.as_mut().qt_thread();
        let command = QString::from(build_core::runner::command_line(&steps[0]).as_str());
        self.as_mut().build_started(build_id, command);
        self.as_mut().diagnostics_changed();

        let source = toolchain.as_str().to_string();
        std::thread::spawn(move || {
            let mut sink = QtSink {
                build_id,
                source,
                qt_thread: qt_thread.clone(),
            };
            let outcome = build_core::runner::run(&handle, &steps, toolchain, &root, &mut sink);
            let _ = qt_thread.queue(move |mut service: Pin<&mut ffi::BuildService>| {
                service.builds.borrow_mut().remove(&build_id);
                service.as_mut().build_finished(build_id, outcome.exit_code);
            });
        });
        ffi::FfiResult::default()
    }

    pub fn stop(self: Pin<&mut Self>, build_id: u64) {
        // Cloned out of the map before stopping: `BuildHandle::stop` takes a
        // lock the build's own thread also uses, and holding the `RefCell`
        // borrow across it would keep the map borrowed while that thread may
        // be queueing a removal onto this one.
        let handle = self.builds.borrow().get(&build_id).cloned();
        if let Some(handle) = handle {
            handle.stop();
        }
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

/// The `BuildSink` that turns a running build's chunks into Qt signals.
/// Every callback is queued rather than called: this runs on the build's own
/// thread.
struct QtSink {
    build_id: u64,
    source: String,
    qt_thread: CxxQtThread<ffi::BuildService>,
}

impl build_core::BuildSink for QtSink {
    fn output(&mut self, text: &str) {
        let build_id = self.build_id;
        let text = text.to_string();
        let _ = self
            .qt_thread
            .queue(move |mut service: Pin<&mut ffi::BuildService>| {
                service
                    .as_mut()
                    .build_output(build_id, QString::from(text.as_str()));
            });
    }

    fn diagnostics(&mut self, diagnostics: Vec<BuildDiagnostic>) {
        let source = self.source.clone();
        let _ = self
            .qt_thread
            .queue(move |mut service: Pin<&mut ffi::BuildService>| {
                service.diagnostics.borrow_mut().extend(diagnostics);
                *service.source.borrow_mut() = source;
                service.as_mut().diagnostics_changed();
            });
    }
}
