//! Run configurations and console (F4): the run configuration model, the
//! toolchain table, project detection of launchable targets, process
//! supervision, output batching for a run console, and `file:line` link
//! resolution in console output.
//!
//! [`toolchain`] is the repo's single source of truth for which build tool a
//! project uses; `build-core` and `dap-core` read it rather than detecting
//! again (R1-1).
//!
//! Qt-free by design (see `docs/architecture/layering.md`'s `run-core`
//! row) — `crates/ui-shell`'s future `RunService` is the only thing that
//! may call this crate from behind the FFI seam (ADR-0003).

pub mod ansi;
pub mod batching;
pub mod before_launch;
pub mod config;
pub mod context;
pub mod detect;
pub mod error;
pub mod links;
pub mod macros;
pub mod supervisor;
pub mod toolchain;

pub use ansi::{AnsiResolver, AnsiStripper, StyledRun, StyledText, TextStyle};
pub use batching::{BatchedOutput, OutputBatcher};
pub use before_launch::{BeforeLaunchError, BeforeLaunchTask};
pub use config::{ConsoleKind, LaunchSpec, RunConfig, RunConfigExt};
pub use context::{config_for_file, remember_temporary, TEMPORARY_CAP};
pub use detect::{detect, merge_detected};
pub use error::RunError;
pub use links::{resolve_link, ResolvedLink};
pub use macros::{expand as expand_macros, MacroContext};
pub use supervisor::{ConsoleId, Supervisor};
pub use toolchain::{detect_toolchains, ToolCommand, ToolchainId};
