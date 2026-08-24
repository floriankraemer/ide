#pragma once

#include <QString>

class AppSettings;
class ProjectTreeModel;
class QMainWindow;
class QMenu;

namespace ui_shell {

// File > Recent Projects (C2): rebuilds `menu` from `appSettings`'s persisted
// list. Each entry opens through openProjectAndRefreshRecents, so clicking one
// reorders the menu the same way "Open Folder..." does.
void populateRecentProjectsMenu(QMenu *menu, AppSettings *appSettings, ProjectTreeModel *treeModel,
                                QMainWindow *window);

// Shared tail for "Open Folder..." and clicking a Recent Projects entry: open,
// report failure, and on success refresh the menu so the just-opened path moves
// to the front (C2).
void openProjectAndRefreshRecents(ProjectTreeModel *treeModel, QMainWindow *window,
                                  QMenu *recentProjectsMenu, AppSettings *appSettings,
                                  const QString &path);

} // namespace ui_shell
