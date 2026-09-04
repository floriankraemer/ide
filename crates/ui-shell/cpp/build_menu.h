#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QHash>
#include <QString>

class QAction;
class QMainWindow;
class QMenu;

namespace ui_shell {

class BuildPanel;
class DockRegistry;

// B1-7: the "&Build" menu (`build.build`/`build.rebuild`/`build.stop`) and
// `view.build` on the existing View menu.
//
// Its own menu rather than more entries on "&Run" for the reason IntelliJ
// separates them too: building and running are different verbs, and a user
// looking for Build does not look under Run.
void buildBuildMenu(QMainWindow *window, BuildPanel *buildPanel, AppSettings *appSettings,
                     QHash<QString, QAction *> &actions, DockRegistry *docks, QMenu *viewMenu);

} // namespace ui_shell
