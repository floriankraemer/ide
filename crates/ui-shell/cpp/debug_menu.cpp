#include "debug_menu.h"

#include "code_editor.h"
#include "debug_panel.h"
#include "dock_layout.h"
#include "e2e_mark.h"
#include "editor_tabs.h"
#include "keymap_page.h"
#include "run_console_panel.h"

#include <QAction>
#include <QMainWindow>
#include <QMenu>
#include <QMenuBar>
#include <QPlainTextEdit>
#include <QTextCursor>

namespace ui_shell {

void buildDebugMenu(QMainWindow *window, DebugService *debugService, DebugPanel *debugPanel,
                     RunConsolePanel *runConsolePanel, EditorTabs *editorTabs, AppSettings *appSettings,
                     QHash<QString, QAction *> &actions, DockRegistry *docks, QMenu *viewMenu)
{
    QMenu *debugMenu = window->menuBar()->addMenu(QObject::tr("&Debug"));
    QObject::connect(debugMenu, &QMenu::aboutToShow, debugMenu,
                      []() { e2eMark("{\"ev\":\"dialog_shown\",\"name\":\"debug_menu\"}"); });
    QObject::connect(debugMenu, &QMenu::aboutToHide, debugMenu,
                      []() { e2eMark("{\"ev\":\"dialog_closed\",\"name\":\"debug_menu\"}"); });

    const auto showDock = [docks]() { docks->show(QStringLiteral("debug")); };

    QAction *debugAction = registerAction(debugMenu, QStringLiteral("debug.debug"),
                                          QObject::tr("Debug"), appSettings, actions);
    QObject::connect(debugAction, &QAction::triggered, debugPanel, [runConsolePanel, showDock]() {
        showDock();
        // The same configuration the toolbar is showing, started by the
        // adapter instead of by the run console — which is the only
        // difference between Run and Debug.
        runConsolePanel->debugSelected();
    });

    struct Entry
    {
        const char *id;
        const char *label;
        void (DebugPanel::*action)();
    };
    // The stepping set IntelliJ's debugging page names, minus the ones that
    // need a capability no adapter here declares yet (force step into,
    // smart step into) — those arrive with D4 rather than as buttons that
    // do nothing.
    const Entry entries[] = {
      {"debug.resume", "Resume Program", &DebugPanel::resume},
      {"debug.pause", "Pause Program", &DebugPanel::pause},
      {"debug.stepOver", "Step Over", &DebugPanel::stepOver},
      {"debug.stepInto", "Step Into", &DebugPanel::stepInto},
      {"debug.stepOut", "Step Out", &DebugPanel::stepOut},
      {"debug.stop", "Stop Debugging", &DebugPanel::stopSession},
    };
    for (const Entry &entry : entries) {
        QAction *action = registerAction(debugMenu, QString::fromLatin1(entry.id),
                                          QObject::tr(entry.label), appSettings, actions);
        const auto method = entry.action;
        QObject::connect(action, &QAction::triggered, debugPanel,
                          [debugPanel, method]() { (debugPanel->*method)(); });
    }

    debugMenu->addSeparator();

    QAction *toggleBreakpoint = registerAction(debugMenu, QStringLiteral("debug.toggleBreakpoint"),
                                                QObject::tr("Toggle Breakpoint"), appSettings,
                                                actions);
    QObject::connect(toggleBreakpoint, &QAction::triggered, editorTabs, [editorTabs]() {
        auto *editor = qobject_cast<CodeEditor *>(editorTabs->currentEditor());
        if (!editor) {
            return;
        }
        editorTabs->toggleBreakpointAt(editor, editor->textCursor().blockNumber());
    });

    QAction *muteBreakpoints = registerAction(debugMenu, QStringLiteral("debug.muteBreakpoints"),
                                               QObject::tr("Mute Breakpoints"), appSettings,
                                               actions);
    muteBreakpoints->setCheckable(true);
    muteBreakpoints->setChecked(debugService->muted());
    QObject::connect(muteBreakpoints, &QAction::toggled, debugService,
                      [debugService](bool muted) { debugService->setMuted(muted); });

    QAction *viewDebugAction = registerAction(viewMenu, QStringLiteral("view.debug"),
                                               QObject::tr("Debug"), appSettings, actions);
    QObject::connect(viewDebugAction, &QAction::triggered, window, showDock);
}

} // namespace ui_shell
