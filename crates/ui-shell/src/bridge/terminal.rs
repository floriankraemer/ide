use core::pin::Pin;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cxx_qt::Threading;
use cxx_qt_lib::QString;

use crate::bridge::errors;
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

/// One session's Qt-thread-owned state: a spawned shell plus its VT100/grid
/// state. Same split `TerminalSessionRust` (the single-session predecessor
/// of this type) used — `pty_session` is `Rc<RefCell<..>>` because only
/// Qt-thread invokables ever touch it, `emulator` is `Arc<Mutex<..>>`
/// because the background reader thread's `feed()` calls and the Qt
/// thread's snapshot reads both touch it.
#[derive(Default)]
struct TerminalEntry {
    pty_session: Rc<RefCell<Option<pty_core::PtySession>>>,
    emulator: std::sync::Arc<std::sync::Mutex<Option<terminal_core::TerminalEmulator>>>,
}

impl TerminalEntry {
    /// `kill_tree`, not `kill`: an interactive shell routinely backgrounds
    /// children (`sleep 60 &`, `npm run dev &`) that share its process
    /// group but are not reaped just because the shell itself is signalled.
    /// Closing a tab or quitting the app must not leave those running
    /// detached from anything that could ever read their output again —
    /// the same "no orphan" guarantee `pty_core::PtySession::kill_tree`'s
    /// own doc comment and tests describe.
    fn kill(&self) {
        if let Some(mut session) = self.pty_session.borrow_mut().take() {
            let _ = session.kill_tree();
        }
    }
}

/// Rust side of the `TerminalSupervisor` QObject (Task F4-14a): the owner of
/// every open terminal session, mirroring `RunServiceRust`'s
/// one-QObject-owns-a-map-of-sessions shape (`bridge/run/mod.rs`) rather
/// than the single self-starting session `TerminalSessionRust` used to be.
///
/// # Why one QObject, not N instances of the old one
///
/// The old shape — one `TerminalSessionRust` per dock widget — does not
/// generalize to N terminals: cxx-qt registers a `#[qobject]` type's
/// QMetaObject once at build time (see `bridge/registry.rs` and every other
/// adapter in this file), so C++ can `new` more `QObject`s of a *type* the
/// bridge declares, but there is no mechanism here for the *view* to ask
/// Rust for a fresh, independently-backed instance of `TerminalSession` at
/// runtime — every existing multi-instance QObject in this codebase (dock
/// widgets, dialogs) is a plain `QWidget`/`QDialog`, never a `#[qobject]`
/// cxx-qt bridge type constructed more than once. `RunServiceRust` already
/// solved the actual problem this task has — N independent backend
/// lifecycles behind one adapter — with a `HashMap<u64, ..>` keyed by an
/// id the view carries per tab, so this file follows that precedent instead
/// of exploring new cxx-qt plumbing for something the codebase already has
/// a working answer to.
///
/// No worker thread, unlike `RunServiceRust`: `run_core::Supervisor` is one
/// value shared by every launch, so it needs a single thread serializing
/// access to it. A terminal session's `PtySession` + `TerminalEmulator`
/// pair is independent per session — nothing here is shared *across*
/// sessions — so each session's own reader thread queuing its own
/// `gridUpdated(sessionId)` is enough; there is no shared value that a
/// second thread would need to also serialize.
#[derive(Default)]
pub struct TerminalSupervisorRust {
    sessions: RefCell<HashMap<u64, TerminalEntry>>,
    next_id: std::cell::Cell<u64>,
}

impl Drop for TerminalSupervisorRust {
    /// App shutdown with N sessions open: kill every shell so none is left
    /// running detached from anything that could ever read its output again
    /// (mirrors `pty_core::PtySession::kill_tree`'s "no orphan" guarantee).
    /// A killed PTY's read half returns EOF/an error, which is what lets
    /// each session's reader thread notice and exit on its own — this is
    /// the shutdown order the whole file is built around: kill first, let
    /// readers unwind themselves, never join them from here (a blocking
    /// `read()` a shell never writes to again would otherwise hang `Drop`).
    fn drop(&mut self) {
        for entry in self.sessions.borrow_mut().values() {
            entry.kill();
        }
    }
}

/// A grid column is a cell — one character — while `run_core::links`
/// measures in bytes. These two convert between them; a row of ASCII makes
/// them the identity, which is exactly why the conversion has to be
/// written down rather than assumed.
fn byte_offset_of_column(text: &str, column: usize) -> usize {
    text.char_indices()
        .nth(column)
        .map_or(text.len(), |(offset, _)| offset)
}

fn column_of_byte_offset(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].chars().count()
}

impl ffi::TerminalSupervisor {
    /// Allocate a fresh session id and its (not-yet-started) backing state.
    /// The shell is not spawned here — `start()` is, same as the old
    /// single-session `TerminalSessionRust::start`, called once the new
    /// tab's `TerminalWidget` knows its own pixel size.
    pub fn new_session(self: Pin<&mut Self>) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.sessions
            .borrow_mut()
            .insert(id, TerminalEntry::default());
        id
    }

    /// Close a session: kill its shell (its reader thread then sees EOF and
    /// exits on its own) and forget its state. Safe to call on an id that
    /// was never started or is already gone.
    pub fn close_session(self: Pin<&mut Self>, session_id: u64) {
        if let Some(entry) = self.sessions.borrow_mut().remove(&session_id) {
            entry.kill();
        }
    }

    pub fn start(self: Pin<&mut Self>, session_id: u64, rows: u32, cols: u32) -> FfiResult {
        let Some(entry) = self.handles(session_id) else {
            return FfiResult {
                code: errors::CODE_TERMINAL,
                message: QString::from("unknown terminal session"),
            };
        };

        let shell = resolve_shell();
        let pty_size = pty_core::PtySize::new(rows as u16, cols as u16);
        let mut session = match pty_core::PtySession::spawn(&shell, pty_size) {
            Ok(session) => session,
            Err(err) => {
                return FfiResult {
                    code: errors::CODE_TERMINAL,
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
                code: errors::CODE_TERMINAL,
                message: QString::from("PTY read half unavailable"),
            };
        };

        let grid_size = terminal_core::GridSize::new(rows as usize, cols as usize);
        *entry.emulator.lock().unwrap() = Some(terminal_core::TerminalEmulator::new(grid_size));
        *entry.pty_session.borrow_mut() = Some(session);

        let emulator_slot = std::sync::Arc::clone(&entry.emulator);
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
                        let sent = qt_thread.queue(move |mut supervisor: Pin<&mut Self>| {
                            supervisor.as_mut().grid_updated(session_id);
                        });
                        if sent.is_err() {
                            break; // The supervisor is gone (app shutdown).
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        FfiResult::default()
    }

    /// Clone out the two handles `session_id` owns, dropping the map borrow
    /// immediately — every invokable that reaches into a session's
    /// `PtySession`/emulator needs this rather than holding
    /// `self.sessions.borrow()` across the call, which the borrow checker
    /// rightly refuses (the call's `RefMut`/`MutexGuard` would otherwise be
    /// dropped while the map `Ref` that outlives it is still considered
    /// borrowed).
    fn handles(&self, session_id: u64) -> Option<TerminalEntry> {
        self.sessions
            .borrow()
            .get(&session_id)
            .map(|entry| TerminalEntry {
                pty_session: Rc::clone(&entry.pty_session),
                emulator: std::sync::Arc::clone(&entry.emulator),
            })
    }

    pub fn write(self: Pin<&mut Self>, session_id: u64, input: &QString) {
        let Some(entry) = self.handles(session_id) else {
            return;
        };
        let mut pty_session = entry.pty_session.borrow_mut();
        if let Some(session) = pty_session.as_mut() {
            let _ = session.write(input.to_string().as_bytes());
        }
    }

    pub fn resize(self: Pin<&mut Self>, session_id: u64, rows: u32, cols: u32) {
        let Some(entry) = self.handles(session_id) else {
            return;
        };
        let mut pty_session = entry.pty_session.borrow_mut();
        if let Some(session) = pty_session.as_mut() {
            let _ = session.resize(pty_core::PtySize::new(rows as u16, cols as u16));
        }
        drop(pty_session);
        // Bound to a named local first, same as `pty_session` above: an
        // `if let` matching directly on a method call through a field of a
        // local (`entry.emulator.lock()`) ties that method's temporary to
        // `entry`'s own scope in a way the borrow checker rejects.
        let lock_result = entry.emulator.lock();
        if let Ok(mut guard) = lock_result {
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
    fn snapshot(&self, session_id: u64) -> Option<terminal_core::Grid> {
        let sessions = self.sessions.borrow();
        let entry = sessions.get(&session_id)?;
        let guard = entry.emulator.lock().ok()?;
        guard.as_ref().map(terminal_core::TerminalEmulator::grid)
    }

    pub fn grid_cells(&self, session_id: u64) -> Vec<ffi::FfiTerminalCell> {
        let Some(snapshot) = self.snapshot(session_id) else {
            return Vec::new();
        };
        snapshot
            .rows
            .into_iter()
            .flatten()
            .map(to_ffi_terminal_cell)
            .collect()
    }

    pub fn grid_rows(&self, session_id: u64) -> u32 {
        self.snapshot(session_id).map_or(0, |g| g.rows.len() as u32)
    }

    pub fn grid_cols(&self, session_id: u64) -> u32 {
        self.snapshot(session_id)
            .map_or(0, |g| g.rows.first().map_or(0, Vec::len) as u32)
    }

    pub fn cursor_row(&self, session_id: u64) -> u32 {
        self.snapshot(session_id).map_or(0, |g| g.cursor.row as u32)
    }

    pub fn cursor_col(&self, session_id: u64) -> u32 {
        self.snapshot(session_id).map_or(0, |g| g.cursor.col as u32)
    }

    /// Run `body` against a live session's emulator, if `session_id` is
    /// known and has been started. The selection invokables take `&self`
    /// (not `Pin<&mut Self>`) because the emulator lives behind the
    /// `Arc<Mutex<..>>` the reader thread also holds — the `&mut` they need
    /// comes from the lock, not from the QObject.
    fn with_emulator<T>(
        &self,
        session_id: u64,
        body: impl FnOnce(&mut terminal_core::TerminalEmulator) -> T,
    ) -> Option<T> {
        let sessions = self.sessions.borrow();
        let entry = sessions.get(&session_id)?;
        let mut guard = entry.emulator.lock().ok()?;
        guard.as_mut().map(body)
    }

    pub fn selection_start(
        &self,
        session_id: u64,
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
        self.with_emulator(session_id, |emulator| {
            emulator.selection_start(row as usize, col as usize, right_half, kind)
        });
    }

    pub fn selection_update(&self, session_id: u64, row: u32, col: u32, right_half: bool) {
        self.with_emulator(session_id, |emulator| {
            emulator.selection_update(row as usize, col as usize, right_half)
        });
    }

    pub fn selection_clear(&self, session_id: u64) {
        self.with_emulator(session_id, terminal_core::TerminalEmulator::selection_clear);
    }

    pub fn has_selection(&self, session_id: u64) -> bool {
        self.with_emulator(session_id, |emulator| emulator.has_selection())
            .unwrap_or(false)
    }

    pub fn selection_text(&self, session_id: u64) -> QString {
        let text = self
            .with_emulator(session_id, |emulator| emulator.selection_text())
            .flatten()
            .unwrap_or_default();
        QString::from(text.as_str())
    }

    pub fn paste(self: Pin<&mut Self>, session_id: u64, text: &QString) {
        let Some(payload) = self.with_emulator(session_id, |emulator| {
            emulator.paste_payload(&text.to_string())
        }) else {
            return;
        };
        let Some(entry) = self.handles(session_id) else {
            return;
        };
        let mut pty_session = entry.pty_session.borrow_mut();
        if let Some(session) = pty_session.as_mut() {
            let _ = session.write(payload.as_bytes());
        }
    }

    /// What, if anything, the cell at `row`/`col` links to.
    ///
    /// A `http(s)` URL is the grid's own answer (`terminal_core`'s
    /// `link_at`). Anything else a row might contain is a question about
    /// *text*, and the one place this codebase recognises a `file:line` in
    /// text is `run_core::links` — the same function the run console's
    /// Ctrl+Click goes through, so a compiler error is the same link
    /// wherever it is printed (R2-6). Relative paths resolve against the
    /// project root, which is where a terminal opened in this IDE starts.
    pub fn link_at(&self, session_id: u64, row: u32, col: u32) -> ffi::FfiTerminalLink {
        if let Some(link) = self
            .with_emulator(session_id, |emulator| {
                emulator.link_at(row as usize, col as usize)
            })
            .flatten()
        {
            return ffi::FfiTerminalLink {
                found: true,
                url: QString::from(link.url.as_str()),
                row: link.row as u32,
                start_col: link.start_col as u32,
                end_col: link.end_col as u32,
                ..Default::default()
            };
        }

        let Some(root) = crate::bridge::convert::current_project_root() else {
            return ffi::FfiTerminalLink::default();
        };
        let Some(text) = self
            .with_emulator(session_id, |emulator| emulator.row_text(row as usize))
            .flatten()
        else {
            return ffi::FfiTerminalLink::default();
        };

        let offset = byte_offset_of_column(&text, col as usize);
        let Some(link) = run_core::resolve_link(&text, offset, &root) else {
            return ffi::FfiTerminalLink::default();
        };
        ffi::FfiTerminalLink {
            found: true,
            url: QString::default(),
            row,
            start_col: column_of_byte_offset(&text, link.start) as u32,
            end_col: column_of_byte_offset(&text, link.end) as u32,
            is_file: true,
            path: QString::from(link.path.display().to_string().as_str()),
            line: link.line,
            has_column: link.col.is_some(),
            column: link.col.unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod shutdown_order_tests {
    //! Qt-free: `TerminalSupervisorRust`'s session bookkeeping (the
    //! `HashMap`/`TerminalEntry` pair) needs no QObject to exercise, so the
    //! "no orphaned process on shutdown" guarantee is tested directly here,
    //! reusing the exact idiom `pty-core`'s own `kill_tree_reaches_a_grandchild`
    //! test uses: a shell backgrounds a `sleep`, prints its pid, and the test
    //! watches that specific process rather than the shell itself — the case
    //! that matters is the grandchild a plain `kill` would leave orphaned.
    use super::TerminalEntry;
    use std::time::{Duration, Instant};

    fn wait_until(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if check() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("timed out waiting for {what}");
    }

    /// Mirrors `pty_core`'s own private `alive()` test helper (not `kill(pid,
    /// 0)`, which reports a reparented zombie as alive forever).
    fn alive(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .map(|state| state != "Z")
            .unwrap_or(false)
    }

    /// Spawn a shell that backgrounds a `sleep` and prints its pid, wrap it
    /// in a `TerminalEntry` the way `start()` would, and return the
    /// grandchild's pid once it has reported in.
    fn spawn_entry_with_grandchild() -> (TerminalEntry, u32) {
        let spec = pty_core::ShellSpec::new(
            "/bin/sh",
            vec!["-c".into(), "sleep 300 & echo $!; wait".into()],
        );
        let mut session = pty_core::PtySession::spawn(&spec, pty_core::PtySize::new(24, 80))
            .expect("spawn /bin/sh for test");

        let mut buffer = String::new();
        wait_until("the grandchild to report its pid", || {
            let mut chunk = [0u8; 256];
            match session.read(&mut chunk) {
                Ok(n) if n > 0 => {
                    buffer.push_str(&String::from_utf8_lossy(&chunk[..n]));
                    buffer.contains('\n')
                }
                _ => false,
            }
        });
        let grandchild: u32 = buffer
            .split_whitespace()
            .find_map(|token| token.parse().ok())
            .expect("a pid on stdout");
        assert!(alive(grandchild), "the grandchild should be running");

        let entry = TerminalEntry {
            pty_session: std::rc::Rc::new(std::cell::RefCell::new(Some(session))),
            emulator: Default::default(),
        };
        (entry, grandchild)
    }

    #[test]
    fn killing_an_entry_reaches_its_backgrounded_grandchild() {
        let (entry, grandchild) = spawn_entry_with_grandchild();
        entry.kill();
        wait_until("the grandchild to die", || !alive(grandchild));
    }

    #[test]
    fn dropping_the_supervisor_kills_every_open_session() {
        let supervisor = super::TerminalSupervisorRust::default();
        let (entry_a, grandchild_a) = spawn_entry_with_grandchild();
        let (entry_b, grandchild_b) = spawn_entry_with_grandchild();
        supervisor.sessions.borrow_mut().insert(1, entry_a);
        supervisor.sessions.borrow_mut().insert(2, entry_b);

        drop(supervisor);

        wait_until("session 1's grandchild to die", || !alive(grandchild_a));
        wait_until("session 2's grandchild to die", || !alive(grandchild_b));
    }
}

#[cfg(test)]
mod terminal_link_offset_tests {
    use super::{byte_offset_of_column, column_of_byte_offset};

    #[test]
    fn ascii_columns_and_byte_offsets_agree() {
        let row = "src/main.rs:42:5";
        assert_eq!(byte_offset_of_column(row, 4), 4);
        assert_eq!(column_of_byte_offset(row, 4), 4);
    }

    #[test]
    fn a_wide_character_earlier_in_the_row_shifts_the_offset() {
        // The grid counts cells; `run_core::links` counts bytes. Assuming
        // they are the same underlines the wrong cells the moment anything
        // non-ASCII is printed above the link — which build output does,
        // routinely, with arrows and check marks.
        let row = "\u{2192} src/main.rs:1";
        assert_eq!(byte_offset_of_column(row, 2), "\u{2192} ".len());
        assert_eq!(column_of_byte_offset(row, "\u{2192} ".len()), 2);
    }

    #[test]
    fn a_column_past_the_end_lands_at_the_end() {
        assert_eq!(byte_offset_of_column("abc", 99), 3);
        assert_eq!(column_of_byte_offset("abc", 99), 3);
    }
}
