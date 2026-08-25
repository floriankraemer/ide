//! Why a `run-core` operation failed.
//!
//! Mirrors `vcs_core::VcsError`'s shape (ADR-0003): a typed variant per
//! failure kind, a stable numeric code in the 800-899 range the plan
//! document reserves for this crate, and a `Display` message meant to be
//! shown to the user verbatim — never a bare `QString` sentinel.

use std::fmt;

use pty_core::PtyError;

/// Why a `run-core` operation failed.
#[derive(Debug)]
pub enum RunError {
    /// A `RunConfig` is missing something it needs to launch, e.g. an empty
    /// `program`. Caught before ever touching `pty-core`.
    InvalidConfig(String),
    /// The configured working directory does not exist. Checked before
    /// spawning, so the failure names the actual problem instead of
    /// surfacing as an opaque spawn error.
    CwdNotFound(String),
    /// `pty-core` could not spawn the process (the program does not exist,
    /// is not executable, is a directory, ...). `pty-core`'s own message —
    /// `program`'s libc/Win32 error text — is kept, since it already says
    /// exactly what went wrong.
    Spawn(String),
    /// A read/write against a running console's PTY failed.
    Io(String),
    /// `stop`/`resolveLink`-shaped calls given a console id nobody issued
    /// (already closed, or never started).
    UnknownConsole,
    /// `kill_tree` itself failed to signal the process (distinct from
    /// [`pty_core::KillOutcome::Escaped`], which is a successful signal that
    /// honestly reports an escaped grandchild — that is not an error, see
    /// `Supervisor::stop`).
    KillTree(String),
}

impl RunError {
    pub const CODE_INVALID_CONFIG: i32 = 800;
    pub const CODE_CWD_NOT_FOUND: i32 = 801;
    pub const CODE_SPAWN: i32 = 802;
    pub const CODE_IO: i32 = 803;
    pub const CODE_UNKNOWN_CONSOLE: i32 = 804;
    pub const CODE_KILL_TREE: i32 = 805;

    /// The variant's stable numeric code. Append-only once this crosses an
    /// FFI seam (ADR-0003): existing numbers must never be renumbered.
    pub fn code(&self) -> i32 {
        match self {
            RunError::InvalidConfig(_) => Self::CODE_INVALID_CONFIG,
            RunError::CwdNotFound(_) => Self::CODE_CWD_NOT_FOUND,
            RunError::Spawn(_) => Self::CODE_SPAWN,
            RunError::Io(_) => Self::CODE_IO,
            RunError::UnknownConsole => Self::CODE_UNKNOWN_CONSOLE,
            RunError::KillTree(_) => Self::CODE_KILL_TREE,
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::InvalidConfig(msg) => write!(f, "invalid run configuration: {msg}"),
            RunError::CwdNotFound(path) => {
                write!(f, "working directory does not exist: {path}")
            }
            RunError::Spawn(msg) => write!(f, "could not start the run: {msg}"),
            RunError::Io(msg) => write!(f, "run console I/O error: {msg}"),
            RunError::UnknownConsole => write!(f, "no such console"),
            RunError::KillTree(msg) => write!(f, "could not stop the run: {msg}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<PtyError> for RunError {
    fn from(err: PtyError) -> Self {
        match err {
            PtyError::Spawn(msg) => RunError::Spawn(msg),
            PtyError::Io(msg) => RunError::Io(msg),
            PtyError::Resize(msg) => RunError::Io(msg),
            PtyError::Wait(msg) => RunError::KillTree(msg),
        }
    }
}
