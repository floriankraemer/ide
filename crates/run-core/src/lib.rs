//! Run configurations and console (F4): the run configuration model,
//! project detection of launchable targets, process supervision, output
//! batching for a run console, and `file:line` link resolution in console
//! output.
//!
//! Qt-free by design (see `docs/architecture/layering.md`'s `run-core`
//! row) — `crates/ui-shell`'s future `RunService` is the only thing that
//! may call this crate from behind the FFI seam (ADR-0003).

pub mod batching;
pub mod config;
pub mod detect;
pub mod error;
pub mod links;
pub mod supervisor;

pub use batching::{BatchedOutput, OutputBatcher};
pub use config::{ConsoleKind, LaunchSpec, RunConfig, RunConfigExt};
pub use detect::{detect, merge_detected};
pub use error::RunError;
pub use links::{resolve_link, ResolvedLink};
pub use supervisor::{ConsoleId, Supervisor};
