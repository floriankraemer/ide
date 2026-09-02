#include "recent_projects_menu.h"

#include "status_bar.h"
#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QAction>
#include <QMainWindow>
#include <QMenu>
#include <QMessageBox>
#include <QObject>
#include <QStringList>

namespace ui_shell {

void openProjectAndRefreshRecents(ProjectTreeModel *treeModel, QMainWindow *window,
                                  QMenu *recentProjectsMenu, AppSettings *appSettings,
                                  const QString &path)
{
    // `openFolder` is fire-and-forget (ADR-0037: the walk runs on a worker
    // thread) — the outcome arrives later as `projectOpened`/
    // `projectOpenFailed`, so this listens for it instead of branching on a
    // synchronous return. Scoped to this one request: parented to
    // `treeModel` so it can never outlive it, and deletes itself the moment
    // either signal fires (only one ever does for a given open), so a
    // second open started before this one settles doesn't pile up a second
    // dialog/refresh underneath it.
    auto *outcome = new QObject(treeModel);
    QObject::connect(treeModel, &ProjectTreeModel::projectOpened, outcome,
                     [outcome, recentProjectsMenu, appSettings, treeModel, window]() {
                         populateRecentProjectsMenu(recentProjectsMenu, appSettings, treeModel,
                                                    window);
                         outcome->deleteLater();
                     });
    QObject::connect(treeModel, &ProjectTreeModel::projectOpenFailed, outcome,
                     [outcome, window](const FfiResult &result) {
                         QMessageBox::critical(window, QObject::tr("Cannot open folder"),
                                               result.message);
                         outcome->deleteLater();
                     });
    showProjectOpening(window);
    treeModel->openFolder(path);
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
