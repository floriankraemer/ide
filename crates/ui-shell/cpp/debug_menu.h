#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QHash>
#include <QString>

class QAction;
class QMainWindow;
class QMenu;

namespace ui_shell {

class DebugPanel;
class DockRegistry;
class EditorTabs;
class RunConsolePanel;

// D3-8/D2-5: the "&Debug" menu — start a session, step, and the breakpoint
// actions (toggle on the caret's line, Mute Breakpoints) — plus `view.debug`
// on the View menu.
void buildDebugMenu(QMainWindow *window, DebugService *debugService, DebugPanel *debugPanel,
                     RunConsolePanel *runConsolePanel, EditorTabs *editorTabs, AppSettings *appSettings,
                     QHash<QString, QAction *> &actions, DockRegistry *docks, QMenu *viewMenu);

} // namespace ui_shell
