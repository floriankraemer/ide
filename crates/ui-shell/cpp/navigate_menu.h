#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QHash>
#include <QString>

class QAction;
class QMainWindow;

namespace ui_shell {

class DockRegistry;
class EditorTabs;
class FindUsagesPanel;

// N8: the "&Navigate" menu — Go to Declaration/Implementation/Interface,
// Find Usages, and the Back/Forward jump history. Every entry routes
// through one `DeclarationNavigator` (owned here) so there is a single
// place that turns a resolution result into a jump. Its own translation
// unit for the same reason `vcs_menu.cpp`/`run_menu.cpp` are:
// `main_window.cpp` sits at its 1200-line ceiling (ADR-0025).
void buildNavigateMenu(QMainWindow *window, LanguageService *languageService,
                        SearchModel *searchModel, EditorTabs *editorTabs, AppSettings *appSettings,
                        QHash<QString, QAction *> &actions, DockRegistry *docks,
                        FindUsagesPanel *findUsagesPanel);

} // namespace ui_shell
