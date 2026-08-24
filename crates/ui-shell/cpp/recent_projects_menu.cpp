#include "recent_projects_menu.h"

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QAction>
#include <QMainWindow>
#include <QMenu>
#include <QMessageBox>
#include <QStringList>

namespace ui_shell {

void openProjectAndRefreshRecents(ProjectTreeModel *treeModel, QMainWindow *window,
                                  QMenu *recentProjectsMenu, AppSettings *appSettings,
                                  const QString &path)
{
    const auto result = treeModel->openFolder(path);
    if (result.code != 0) {
        QMessageBox::critical(window, QObject::tr("Cannot open folder"), result.message);
        return;
    }
    populateRecentProjectsMenu(recentProjectsMenu, appSettings, treeModel, window);
}

void populateRecentProjectsMenu(QMenu *menu, AppSettings *appSettings, ProjectTreeModel *treeModel,
                                QMainWindow *window)
{
    menu->clear();
    const QStringList projects = appSettings->recentProjects();
    if (projects.isEmpty()) {
        QAction *empty = menu->addAction(QObject::tr("(No Recent Projects)"));
        empty->setEnabled(false);
        return;
    }
    for (const QString &path : projects) {
        QAction *action = menu->addAction(path);
        QObject::connect(action, &QAction::triggered, treeModel,
                         [treeModel, window, menu, appSettings, path]() {
                             openProjectAndRefreshRecents(treeModel, window, menu, appSettings,
                                                          path);
                         });
    }
}

} // namespace ui_shell
