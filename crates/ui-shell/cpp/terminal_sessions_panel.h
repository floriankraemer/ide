#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QWidget>

class QAction;
class QTabWidget;

namespace ui_shell {

// The Terminal dock (F4-14b): a `QTabWidget` with one `TerminalWidget` per
// open session, all backed by the one `TerminalSupervisor` QObject
// (`TerminalWidget`'s doc comment explains why one adapter, not N — cxx-qt
// registers a `#[qobject]` type's QMetaObject once, so the multiplicity
// this task asked for lives in the session-id-keyed map on the Rust side,
// not in N QObject instances).
//
// Same "a dock holding a QTabWidget, one tab per backend session, tabs
// created/destroyed as sessions start/stop" shape `RunConsolePanel`
// (F4-11) already established for the Run dock — this class mirrors it,
// with one deliberate difference: a run console tab is left open after its
// process exits (F4-11's own review requirement), but a terminal tab *is*
// its session — closing the tab is the only way to end one, so the two
// stay in lock-step here instead.
//
// No shell picker: `pty_core::ShellSpec::unix_default()`/`::windows(kind)`
// only offer a real choice on Windows, and this codebase has no Windows CI
// to exercise a picker UI against — see `resolve_shell()` in
// `bridge/terminal.rs`. `"+"` always opens the platform default.
class TerminalSessionsPanel : public QWidget
{
    Q_OBJECT

public:
    TerminalSessionsPanel(TerminalSupervisor *supervisor, AppSettings *appSettings,
                           QWidget *parent = nullptr);

    // Open a new tab and give it focus — the target of both the "+" button
    // and the `terminal.newSession` action (Ctrl+Shift+T).
    void addSession();

    // Focus the current tab's terminal, for the `view.terminal` action.
    void focusCurrent();

    // Re-apply Copy/Paste's shortcuts to every open tab, and this panel's own
    // `newSession` shortcut, after a keymap rebind (Settings > Keymap > OK).
    void reapplyKeymap();

    // `terminal.newSession` (Ctrl+Shift+T): one QAction on the panel itself,
    // not per tab, so — unlike Copy/Paste — it is long-lived enough to sit
    // in the app-wide `actions` map `main_window.cpp` builds for Settings >
    // Keymap and `applyKeymap()`.
    QAction *newSessionAction() const { return newSessionAction_; }

private:
    void closeTab(int index);

    TerminalSupervisor *supervisor_;
    AppSettings *appSettings_;
    QTabWidget *tabs_ = nullptr;
    QAction *newSessionAction_ = nullptr;
    int sessionCounter_ = 0;
};

} // namespace ui_shell
