#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QHash>
#include <QString>
#include <QWidget>

#include <functional>

class QPlainTextEdit;
class QTabWidget;

namespace ads {
class CDockAreaWidget;
class CDockManager;
} // namespace ads

namespace ui_shell {

class DockRegistry;
class RunToolbar;

// The Run Console dock (F4-11): a `RunToolbar` (configuration picker,
// Run/Stop/Rerun) over a `QTabWidget` with one read-only console per active
// or finished run.
//
// Humble view: what output a console produced, and what `file:line` a
// position in it resolves to, are `RunService`/`run-core` calls
// (`consoleOutput`, `resolveLink`); this widget only appends the text it is
// given to the right tab and turns a resolved link into `openAt`. See
// `bridge/run/mod.rs`'s `ConsoleState::ansi` doc comment for why the text
// arriving here is already ANSI-stripped rather than colored — a v1
// decision, not an omission.
class RunConsolePanel : public QWidget
{
public:
    using OpenAt = std::function<void(const QString &, int, int)>;

    RunConsolePanel(RunService *runService, OpenAt openAt, QWidget *parent);

    // Forwarded to the embedded RunToolbar — the targets of the global
    // `run.*` shortcuts, which must act on the toolbar's current selection
    // regardless of which widget has focus.
    void runSelected();
    void stopSelected();
    void rerunSelected();
    void focusConfigSelector();

private:
    void onConsoleStarted(quint64 consoleId, const QString &configId);
    void onConsoleOutput(quint64 consoleId, const QString &text);
    void onConsoleTruncated(quint64 consoleId);
    void onConsoleFinished(quint64 consoleId, int exitCode, bool escaped);
    void onLinkActivated(quint64 consoleId, int textPosition);

    RunService *runService_;
    OpenAt openAt_;
    RunToolbar *toolbar_ = nullptr;
    QTabWidget *tabs_ = nullptr;

    struct ConsoleTab
    {
        QPlainTextEdit *edit;
        bool truncationNoticeShown = false;
    };
    QHash<quint64, ConsoleTab> consoles_;
};

// Builds the panel, wraps it in a dock widget and registers it with `docks`
// under id `"runConsole"`, exactly the pattern `ChangesPanel`/
// `FileHistoryPanel` follow in `main_window.cpp` — pulled out to one call so
// wiring a new dock there costs one line, not five (the file sits at its
// 1200-line ceiling, ADR-0025).
RunConsolePanel *buildRunConsoleDock(ads::CDockManager *dockManager, DockRegistry *docks,
                                     ads::CDockAreaWidget *relativeTo, RunService *runService,
                                     RunConsolePanel::OpenAt openAt);

} // namespace ui_shell
