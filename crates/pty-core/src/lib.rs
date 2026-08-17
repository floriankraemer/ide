//! Cross-platform PTY transport: spawn a shell attached to a pseudo-terminal,
//! read/write its byte stream, resize it, and manage the child process.
//!
//! Qt-free by design (see `docs/architecture/layering.md`) — this crate only
//! moves bytes in and out of a PTY. Grid/VT100 state lives in `terminal-core`
//! (task F2); wiring it into the UI lives in `ui-shell` (task F3).
//!
//! Blocking reads are intentional: the eventual `ui-shell` integration drives
//! this from a dedicated `std::thread` doing blocking reads, the same shape
//! `start_mcp_server` already uses for its background listener thread.

use std::env;
use std::fmt;
use std::io::{Read, Write};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize as NativePtySize};

/// Terminal size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl PtySize {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self { rows, cols }
    }
}

impl From<PtySize> for NativePtySize {
    fn from(size: PtySize) -> Self {
        NativePtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// Typed error crossing this crate's API (ADR-0003's typed-error convention
/// applies once this reaches the FFI seam in a later task).
#[derive(Debug)]
pub enum PtyError {
    Spawn(String),
    Io(String),
    Resize(String),
    Wait(String),
}

impl fmt::Display for PtyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PtyError::Spawn(msg) => write!(f, "failed to spawn shell: {msg}"),
            PtyError::Io(msg) => write!(f, "PTY I/O error: {msg}"),
            PtyError::Resize(msg) => write!(f, "failed to resize PTY: {msg}"),
            PtyError::Wait(msg) => write!(f, "failed to wait on child process: {msg}"),
        }
    }
}

impl std::error::Error for PtyError {}

/// Which Windows shell to launch. Windows offers no single canonical shell,
/// and CI/build environments won't have all of them installed, so the
/// caller picks explicitly instead of this crate probing the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsShellKind {
    PowerShellCore,
    WindowsPowerShell,
    Wsl,
}

impl WindowsShellKind {
    fn program(self) -> &'static str {
        match self {
            WindowsShellKind::PowerShellCore => "pwsh.exe",
            WindowsShellKind::WindowsPowerShell => "powershell.exe",
            WindowsShellKind::Wsl => "wsl.exe",
        }
    }
}

/// The program (and args) to launch as the PTY's child process.
/// Deliberately a plain data struct — no OS probing happens in constructors,
/// so callers (and tests) can inject an explicit shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl ShellSpec {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    /// Resolve `$SHELL`, falling back to `/bin/bash` then `/bin/sh` if unset.
    pub fn unix_default() -> Self {
        let program = env::var("SHELL").unwrap_or_else(|_| {
            if std::path::Path::new("/bin/bash").exists() {
                "/bin/bash".to_string()
            } else {
                "/bin/sh".to_string()
            }
        });
        Self {
            program,
            args: Vec::new(),
        }
    }

    /// A named Windows shell, with no OS probing — the caller (or its own
    /// fallback policy) decides which of `pwsh.exe`/`powershell.exe`/
    /// `wsl.exe` to request.
    pub fn windows(kind: WindowsShellKind) -> Self {
        Self {
            program: kind.program().to_string(),
            args: Vec::new(),
        }
    }
}

/// A running shell attached to a pseudo-terminal.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    reader: Option<Box<dyn Read + Send>>,
    writer: Box<dyn Write + Send>,
}

impl PtySession {
    /// Spawn `shell` attached to a new PTY of the given size.
    pub fn spawn(shell: &ShellSpec, size: PtySize) -> Result<Self, PtyError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size.into())
            .map_err(|e| PtyError::Spawn(e.to_string()))?;

        let mut cmd = CommandBuilder::new(&shell.program);
        for arg in &shell.args {
            cmd.arg(arg);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Spawn(e.to_string()))?;
        // Drop our copy of the slave end after spawning: the child owns its
        // own handle, and holding ours open would keep the PTY's read end
        // from ever reporting EOF once the child exits.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Spawn(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Spawn(e.to_string()))?;

        Ok(Self {
            master: pair.master,
            child,
            reader: Some(reader),
            writer,
        })
    }

    /// Blocking read of whatever output bytes are currently available.
    /// Returns `Ok(0)` on EOF (child exited and closed the PTY). Errors with
    /// `PtyError::Io` if [`take_reader`](Self::take_reader) already moved
    /// the read half out.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, PtyError> {
        match self.reader.as_mut() {
            Some(reader) => reader.read(buf).map_err(|e| PtyError::Io(e.to_string())),
            None => Err(PtyError::Io("read half already taken".to_string())),
        }
    }

    /// Move the read half out for a dedicated background thread to own.
    /// Needed because `read`/`write` both require exclusive (`&mut self`)
    /// access: a caller that put the whole `PtySession` behind one lock so a
    /// background thread could do blocking reads would find that lock held
    /// for the whole blocking `read` call, stalling any `write` from another
    /// thread until the next output byte arrives — for an interactive shell
    /// that's a deadlock (the shell can't echo a keystroke that `write` can
    /// never deliver). Splitting the read half out lets the reader thread
    /// own it exclusively while `write`/`resize`/`kill` stay on the
    /// `PtySession` for the caller's own thread to use, lock-free. Returns
    /// `None` if already taken.
    pub fn take_reader(&mut self) -> Option<Box<dyn Read + Send>> {
        self.reader.take()
    }

    /// Write input bytes (e.g. keystrokes) to the shell.
    pub fn write(&mut self, data: &[u8]) -> Result<(), PtyError> {
        self.writer
            .write_all(data)
            .map_err(|e| PtyError::Io(e.to_string()))
    }

    /// Resize the PTY (e.g. on dock-widget resize).
    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(size.into())
            .map_err(|e| PtyError::Resize(e.to_string()))
    }

    /// Forcibly terminate the child process.
    pub fn kill(&mut self) -> Result<(), PtyError> {
        self.child.kill().map_err(|e| PtyError::Wait(e.to_string()))
    }

    /// Non-blocking check: `Some(exit_code)` if the child has already
    /// exited, `None` if it's still running.
    pub fn try_wait(&mut self) -> Result<Option<u32>, PtyError> {
        self.child
            .try_wait()
            .map_err(|e| PtyError::Wait(e.to_string()))
            .map(|status| status.map(|s| s.exit_code()))
    }

    /// Block until the child exits, returning its exit code.
    pub fn wait(&mut self) -> Result<u32, PtyError> {
        self.child
            .wait()
            .map_err(|e| PtyError::Wait(e.to_string()))
            .map(|status| status.exit_code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_reads_expected_output() {
        let shell = ShellSpec::new("/bin/sh", vec!["-c".into(), "echo hello".into()]);
        let mut session = PtySession::spawn(&shell, PtySize::new(24, 80)).expect("spawn");

        let mut output = Vec::new();
        let mut buf = [0u8; 256];
        // Read until EOF (child exits and closes the PTY) or we've clearly
        // seen the expected text — avoids hanging if the shell keeps the
        // PTY open past printing.
        loop {
            match session.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    output.extend_from_slice(&buf[..n]);
                    if String::from_utf8_lossy(&output).contains("hello") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("hello"), "expected 'hello' in output, got: {text:?}");

        session.wait().expect("wait");
    }

    #[test]
    fn resize_does_not_error() {
        let shell = ShellSpec::new("/bin/sh", vec!["-c".into(), "sleep 1".into()]);
        let mut session = PtySession::spawn(&shell, PtySize::new(24, 80)).expect("spawn");

        session.resize(PtySize::new(40, 120)).expect("resize");

        session.kill().expect("kill");
        session.wait().expect("wait");
    }

    #[test]
    fn kill_stops_the_child() {
        let shell = ShellSpec::new("/bin/sh", vec!["-c".into(), "sleep 30".into()]);
        let mut session = PtySession::spawn(&shell, PtySize::new(24, 80)).expect("spawn");

        assert_eq!(session.try_wait().expect("try_wait before kill"), None);

        session.kill().expect("kill");
        session.wait().expect("wait after kill");

        assert!(session.try_wait().expect("try_wait after kill").is_some());
    }

    #[test]
    fn take_reader_moves_reading_out_and_still_works() {
        let shell = ShellSpec::new("/bin/sh", vec!["-c".into(), "echo hi".into()]);
        let mut session = PtySession::spawn(&shell, PtySize::new(24, 80)).expect("spawn");

        let mut reader = session.take_reader().expect("reader available once");
        assert!(session.take_reader().is_none(), "second take must yield None");

        let mut output = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    output.extend_from_slice(&buf[..n]);
                    if String::from_utf8_lossy(&output).contains("hi") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        assert!(String::from_utf8_lossy(&output).contains("hi"));

        session.wait().expect("wait");
    }

    #[test]
    fn read_after_take_reader_errors() {
        let shell = ShellSpec::new("/bin/sh", vec!["-c".into(), "sleep 1".into()]);
        let mut session = PtySession::spawn(&shell, PtySize::new(24, 80)).expect("spawn");

        session.take_reader();
        let mut buf = [0u8; 16];
        assert!(session.read(&mut buf).is_err());

        session.kill().expect("kill");
        session.wait().expect("wait");
    }

    #[test]
    fn unix_default_resolves_a_shell() {
        let spec = ShellSpec::unix_default();
        assert!(!spec.program.is_empty());
    }

    #[test]
    fn windows_shell_kinds_map_to_expected_programs() {
        assert_eq!(ShellSpec::windows(WindowsShellKind::PowerShellCore).program, "pwsh.exe");
        assert_eq!(
            ShellSpec::windows(WindowsShellKind::WindowsPowerShell).program,
            "powershell.exe"
        );
        assert_eq!(ShellSpec::windows(WindowsShellKind::Wsl).program, "wsl.exe");
    }
}
