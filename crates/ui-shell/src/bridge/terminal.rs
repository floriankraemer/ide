use core::pin::Pin;
use std::cell::RefCell;
use std::rc::Rc;

use cxx_qt::Threading;
use cxx_qt_lib::QString;

use crate::bridge::ffi::{self, FfiResult};

/// Resolve which shell to spawn (Task F3). Only Linux is in scope for this
/// task (Windows shell-picker UI is a later task); the `cfg` branch just
/// keeps the Rust side platform-correct rather than Linux-only, reusing
/// `pty-core`'s own per-platform `ShellSpec` constructors instead of
/// re-deciding shell resolution here.
fn resolve_shell() -> pty_core::ShellSpec {
    #[cfg(windows)]
    {
        pty_core::ShellSpec::windows(pty_core::WindowsShellKind::PowerShellCore)
    }
    #[cfg(not(windows))]
    {
        pty_core::ShellSpec::unix_default()
    }
}

fn to_ffi_terminal_cell(cell: terminal_core::RenderCell) -> ffi::FfiTerminalCell {
    ffi::FfiTerminalCell {
        character: QString::from(cell.character.to_string().as_str()),
        fg_r: cell.fg.r,
        fg_g: cell.fg.g,
        fg_b: cell.fg.b,
        bg_r: cell.bg.r,
        bg_g: cell.bg.g,
        bg_b: cell.bg.b,
        bold: cell.attrs.bold,
        italic: cell.attrs.italic,
        underline: cell.attrs.underline,
        inverse: cell.attrs.inverse,
        selected: cell.selected,
    }
}

/// Rust side of the `TerminalSession` QObject (Task F3). `pty_session` is
/// `Rc<RefCell<..>>` (Qt-thread-only, same convention `AppSession`'s handle
/// uses in every other adapter here) because only Qt-thread invokables
/// (`write`/`resize`, plus `start`'s own setup) ever touch it — the
/// background reader thread only ever holds the split-off
/// `pty_core::PtySession::take_reader()` handle, never the session itself.
/// `emulator` is `Arc<Mutex<..>>` because it genuinely is shared: the
/// reader thread's `feed()` calls and the Qt thread's `grid()` reads both
/// touch it, mirroring `SearchModelRust`'s index handle.
#[derive(Default)]
pub struct TerminalSessionRust {
    pty_session: Rc<RefCell<Option<pty_core::PtySession>>>,
    emulator: std::sync::Arc<std::sync::Mutex<Option<terminal_core::TerminalEmulator>>>,
}

impl Drop for TerminalSessionRust {
    /// Kill the shell when the dock widget (and its `TerminalSession`) goes
    /// away, e.g. on app shutdown — otherwise the child process would be
    /// left running detached from anything that could ever read its output
    /// again.
    fn drop(&mut self) {
        if let Some(mut session) = self.pty_session.borrow_mut().take() {
            let _ = session.kill();
        }
    }
}

impl ffi::TerminalSession {
    pub fn start(self: Pin<&mut Self>, rows: u32, cols: u32) -> FfiResult {
        let shell = resolve_shell();
        let pty_size = pty_core::PtySize::new(rows as u16, cols as u16);
        let mut session = match pty_core::PtySession::spawn(&shell, pty_size) {
            Ok(session) => session,
            Err(err) => {
                return FfiResult {
                    code: 1,
                    message: QString::from(err.to_string().as_str()),
                }
            }
        };
        // Split off the read half before storing the session (see
        // `pty_core::PtySession::take_reader`'s doc comment for why: a
        // lock held across a blocking `read` would stall `write`, which
        // deadlocks an interactive shell).
        let Some(mut reader) = session.take_reader() else {
            return FfiResult {
                code: 1,
                message: QString::from("PTY read half unavailable"),
            };
        };

        let grid_size = terminal_core::GridSize::new(rows as usize, cols as usize);
        *self.emulator.lock().unwrap() = Some(terminal_core::TerminalEmulator::new(grid_size));
        *self.pty_session.borrow_mut() = Some(session);

        let emulator_slot = std::sync::Arc::clone(&self.emulator);
        let qt_thread = self.qt_thread();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF: the shell exited.
                    Ok(n) => {
                        let Ok(mut guard) = emulator_slot.lock() else {
                            break;
                        };
                        let Some(emulator) = guard.as_mut() else {
                            break;
                        };
                        emulator.feed(&buf[..n]);
                        drop(guard);
                        let _ = qt_thread.queue(|mut session: Pin<&mut Self>| {
                            session.as_mut().grid_updated();
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        FfiResult::default()
    }

    pub fn write(self: Pin<&mut Self>, input: &QString) {
        if let Some(session) = self.pty_session.borrow_mut().as_mut() {
            let _ = session.write(input.to_string().as_bytes());
        }
    }

    pub fn resize(self: Pin<&mut Self>, rows: u32, cols: u32) {
        if let Some(session) = self.pty_session.borrow_mut().as_mut() {
            let _ = session.resize(pty_core::PtySize::new(rows as u16, cols as u16));
        }
        if let Ok(mut guard) = self.emulator.lock() {
            if let Some(emulator) = guard.as_mut() {
                emulator.resize(terminal_core::GridSize::new(rows as usize, cols as usize));
            }
        }
    }

    /// Shared snapshot fetch behind the four `grid*`/`cursor*` invokables
    /// below — `terminal_core::Grid` isn't itself an FFI type, so there is
    /// no way to expose "the" snapshot as a single call's return value
    /// (see `FfiTerminalCell`'s doc comment); each accessor re-snapshots
    /// instead. All four only ever run on the Qt thread, right after
    /// `gridUpdated`, at repaint frequency — not a hot loop.
    fn snapshot(&self) -> Option<terminal_core::Grid> {
        let guard = self.emulator.lock().ok()?;
        guard.as_ref().map(terminal_core::TerminalEmulator::grid)
    }

    pub fn grid_cells(&self) -> Vec<ffi::FfiTerminalCell> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        snapshot
            .rows
            .into_iter()
            .flatten()
            .map(to_ffi_terminal_cell)
            .collect()
    }

    pub fn grid_rows(&self) -> u32 {
        self.snapshot().map_or(0, |g| g.rows.len() as u32)
    }

    pub fn grid_cols(&self) -> u32 {
        self.snapshot()
            .map_or(0, |g| g.rows.first().map_or(0, Vec::len) as u32)
    }

    pub fn cursor_row(&self) -> u32 {
        self.snapshot().map_or(0, |g| g.cursor.row as u32)
    }

    pub fn cursor_col(&self) -> u32 {
        self.snapshot().map_or(0, |g| g.cursor.col as u32)
    }

    /// Run `body` against the live emulator, if a session has been started.
    /// The selection invokables take `&self` (not `Pin<&mut Self>`) because
    /// the emulator lives behind the `Arc<Mutex<..>>` the reader thread also
    /// holds — the `&mut` they need comes from the lock, not from the
    /// QObject, so C++ is spared a pin dance for what is a read-side gesture.
    fn with_emulator<T>(
        &self,
        body: impl FnOnce(&mut terminal_core::TerminalEmulator) -> T,
    ) -> Option<T> {
        let mut guard = self.emulator.lock().ok()?;
        guard.as_mut().map(body)
    }

    pub fn selection_start(
        &self,
        row: u32,
        col: u32,
        right_half: bool,
        kind: ffi::FfiSelectionKind,
    ) {
        let kind = match kind {
            ffi::FfiSelectionKind::Word => terminal_core::SelectionKind::Word,
            ffi::FfiSelectionKind::Line => terminal_core::SelectionKind::Line,
            // `FfiSelectionKind` is a C++-facing enum, so it is not
            // exhaustively matchable from Rust; Simple is the safe default.
            _ => terminal_core::SelectionKind::Simple,
        };
        self.with_emulator(|emulator| {
            emulator.selection_start(row as usize, col as usize, right_half, kind)
        });
    }

    pub fn selection_update(&self, row: u32, col: u32, right_half: bool) {
        self.with_emulator(|emulator| {
            emulator.selection_update(row as usize, col as usize, right_half)
        });
    }

    pub fn selection_clear(&self) {
        self.with_emulator(terminal_core::TerminalEmulator::selection_clear);
    }

    pub fn has_selection(&self) -> bool {
        self.with_emulator(|emulator| emulator.has_selection())
            .unwrap_or(false)
    }

    pub fn selection_text(&self) -> QString {
        let text = self
            .with_emulator(|emulator| emulator.selection_text())
            .flatten()
            .unwrap_or_default();
        QString::from(text.as_str())
    }

    pub fn paste(self: Pin<&mut Self>, text: &QString) {
        let Some(payload) =
            self.with_emulator(|emulator| emulator.paste_payload(&text.to_string()))
        else {
            return;
        };
        if let Some(session) = self.pty_session.borrow_mut().as_mut() {
            let _ = session.write(payload.as_bytes());
        }
    }

    pub fn link_at(&self, row: u32, col: u32) -> ffi::FfiTerminalLink {
        let link = self
            .with_emulator(|emulator| emulator.link_at(row as usize, col as usize))
            .flatten();
        match link {
            Some(link) => ffi::FfiTerminalLink {
                found: true,
                url: QString::from(link.url.as_str()),
                row: link.row as u32,
                start_col: link.start_col as u32,
                end_col: link.end_col as u32,
            },
            None => ffi::FfiTerminalLink {
                found: false,
                url: QString::default(),
                row: 0,
                start_col: 0,
                end_col: 0,
            },
        }
    }
}
