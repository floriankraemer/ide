# 0007. Embedded terminal: `portable-pty` + `alacritty_terminal` + custom `QPainter` grid widget

## Status

Proposed.
Pending a `devops-expert` verification pass of `portable-pty` (Windows ConPTY path) and
`alacritty_terminal` building clean under the existing MXE cross-build stage, per the plan
doc's "Gate before starting Tracks 2 and 3's native-dependency work".
Do not mark this Accepted until that check comes back clean.

## Context

The language-folding/Class-View/terminal/search plan (task F, tracks F1-F3) calls for an
embedded, dockable, cross-platform shell: PowerShell, WSL2, or a regular Unix shell, running
inside the IDE rather than in an external terminal window.
This needs two things a Qt-only approach can't give cleanly: a PTY transport that behaves the
same on Windows (ConPTY) and Unix (a real PTY), and VT100/escape-sequence interpretation that
can be unit tested without a running Qt event loop.

## Decision

Split the terminal into two new Qt-free crates plus a humble view, mirroring the
`syntax-core`/`SyntaxHighlighter` split already in this codebase:

- `pty-core` (this task, F1): spawns a shell attached to a pseudo-terminal via the
  `portable-pty` crate, which abstracts Windows ConPTY and Unix PTYs behind one API.
  Exposes blocking read/write, resize, and kill/wait on the child process.
  Shell resolution is caller-driven, not OS-probed: Unix reads `$SHELL` with a `/bin/bash`/
  `/bin/sh` fallback; Windows takes an explicit `WindowsShellKind` (`PowerShellCore` →
  `pwsh.exe`, `WindowsPowerShell` → `powershell.exe`, `Wsl` → `wsl.exe`) so no constructor
  probes the host OS, keeping shell selection testable on any single platform.
- `terminal-core` (task F2, not yet built): consumes `pty-core`'s byte stream through
  `alacritty_terminal`'s VT100/grid-state engine, producing cursor/selection/cell-grid state
  that can be asserted against in unit tests using known escape-sequence fixtures.
- A `QPainter`-based grid widget in `ui-shell`'s `cpp/` (task F3, not yet built): paints the
  cell grid `terminal-core` produces and forwards keystrokes back through the bridge.
  Same "humble view" shape as `SyntaxHighlighter`'s Qt-side consumer — no VT100 logic in C++.

Background PTY reads run on a plain `std::thread` doing blocking reads, marshaled to Qt via
`CxxQtThread::queue()` — the same pattern `start_mcp_server` already uses for its listener
thread (`ui-shell/src/bridge.rs:1160-1182`).
No `tokio` or other async runtime in `pty-core` or `terminal-core`; `tokio` stays scoped to
`mcp-server`.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| `QTermWidget` (Qt-based terminal emulator widget) | Puts VT100/escape-sequence interpretation and PTY handling behind Qt, making it untestable without a running Qt event loop and violating the "business rules live in Qt-free crates" layering rule this codebase enforces everywhere else. |
| Hand-rolled VT100 parser | A correctness minefield — full-featured terminal emulation (cursor movement, scrollback, color modes, alternate screen buffer, etc.) is exactly the kind of well-trodden, easy-to-get-subtly-wrong problem a maintained crate (`alacritty_terminal`, extracted from a real terminal emulator and already battle-tested) is worth depending on for. |
| Hand-rolled per-platform PTY code | Windows ConPTY and Unix PTYs have meaningfully different APIs; `portable-pty` already abstracts this split behind one interface and is maintained by the same project (wezterm) that also produces `alacritty_terminal`'s closest competitor, so its Windows path is exercised in practice, not theoretical. |

## Consequences

- Positive: PTY transport and VT100 state are both unit-testable in CI without Qt, matching
  every other domain crate in this codebase.
- Positive: the view stays humble — `cpp/`'s terminal widget only paints cells and forwards
  input, with zero VT100 or PTY logic to maintain in C++.
- Negative / accepted risk: two new native-dependency crates (`portable-pty`, later
  `alacritty_terminal`) whose Windows/ConPTY and MXE cross-build behavior has not yet been
  verified end to end — this is the open item this ADR stays Proposed on.
  `pty-core`'s Linux build and unit tests (spawn/read, resize, kill/wait) are verified clean
  under the Docker `linux-builder` stage as of this task; the MXE cross-build path is not yet
  checked.
- The `WindowsShellKind` enum only names the shell; nothing in `pty-core` verifies the named
  executable actually exists on the target machine before spawning, that's a caller-side
  concern for whichever task builds the terminal dock widget's shell-picker (F3 or later).

## Related

- [ADR-0003: FFI seam conventions](0003-ffi-conventions.md) — typed-error convention `pty-core`
  follows internally now, ahead of crossing the FFI seam in task F3.
- [ADR-0004: MCP transport](0004-mcp-transport.md) — the `std::thread` + `CxxQtThread::queue()`
  background-work pattern this ADR reuses for PTY reads.
- `crates/pty-core/src/lib.rs` — the crate this ADR documents.
- `docs/architecture/language-folding-classview-terminal-search-plan.md` — decision 3 and the
  F1-F3 task breakdown this ADR covers.
