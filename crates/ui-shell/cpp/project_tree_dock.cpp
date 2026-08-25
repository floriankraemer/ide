#include "project_tree_dock.h"

#include "ai_chat_panel.h"
#include "dock_layout.h"
#include "icon_cache.h"
#include "icon_decoration_proxy.h"
#include "keymap_page.h"
#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include "DockAreaWidget.h"
#include "DockManager.h"
#include "DockWidget.h"

#include <QAbstractItemModel>
#include <QAction>
#include <QFileInfo>
#include <QInputDialog>
#include <QLineEdit>
#include <QMainWindow>
#include <QMenu>
#include <QMessageBox>
#include <QModelIndex>
#include <QPoint>
#include <QSize>
#include <QStatusBar>
#include <QTreeView>

namespace ui_shell {

namespace {

// ProjectTreeModel::Roles are offsets from Qt::UserRole, not role numbers —
// cxx-qt cannot give a qenum explicit discriminants, so the variants would
// otherwise collide with Qt::DecorationRole and friends. The Rust side adds
// the same base before it matches on the role.
int treeRole(ProjectTreeModel::Roles role)
{
    return Qt::UserRole + static_cast<int>(role);
}

} // namespace

QTreeView *createProjectTreeDock(ads::CDockManager *dockManager,
                                  ads::CDockAreaWidget *editorArea,
                                  ProjectTreeModel *treeModel,
                                  DockRegistry *docks)
{
    auto *treeView = new QTreeView();
    // The style's small-icon metric rather than a literal 16: it already
    // follows the platform's own scaling settings.
    const int iconPx = smallIconPx(treeView);
    treeView->setIconSize(QSize(iconPx, iconPx));

    // Between model and view, never inside the model: the icon key is the
    // Rust side's answer, and turning it into a decoration is the only part
    // that needs a QIcon. See icon_decoration_proxy.h.
    auto *proxy =
      new IconDecorationProxy(treeRole(ProjectTreeModel::Roles::IconKey), iconPx, treeView);
    proxy->setSourceModel(treeModel);

    treeView->setModel(proxy);
    treeView->setHeaderHidden(true);
    auto *treeDock = new ads::CDockWidget(dockManager, QObject::tr("Project"));
    treeDock->setWidget(treeView);
    docks->registerDock(QStringLiteral("projectTree"), treeDock, ads::LeftDockWidgetArea,
                        editorArea);
    return treeView;
}

void wireProjectTree(QTreeView *treeView,
                     ProjectTreeModel *treeModel,
                     const ProjectTreeActions &actions)
{
    // Indexes from the view belong to the proxy, so every data() lookup goes
    // through the view's own model rather than the source directly.
    QAbstractItemModel *model = treeView->model();

    QObject::connect(
      treeView,
      &QTreeView::clicked,
      treeModel,
      [model, openFile = actions.openFile](const QModelIndex &index) {
          const bool isDir = model->data(index, treeRole(ProjectTreeModel::Roles::IsDir)).toBool();
          if (isDir) {
              return;
          }

          const QString path =
            model->data(index, treeRole(ProjectTreeModel::Roles::Path)).toString();
          openFile(path);
      });

    // Right-click context menu: create/rename/delete from the tree (US-2b).
    // Pure intent-forwarding: dialogs gather names/confirmation, the session
    // performs the operation (including retargeting any open tab).
    treeView->setContextMenuPolicy(Qt::CustomContextMenu);
    QObject::connect(
      treeView,
      &QTreeView::customContextMenuRequested,
      treeView,
      [treeView, treeModel, model, actions](const QPoint &pos) {
          QMainWindow *window = actions.window;
          const QString rootPath = treeModel->rootPath();
          if (rootPath.isEmpty()) {
              return; // No project open.
          }

          const QModelIndex index = treeView->indexAt(pos);
          const bool hasItem = index.isValid();
          QString itemPath;
          bool itemIsDir = false;
          QString targetDir = rootPath;
          if (hasItem) {
              itemPath = model->data(index, treeRole(ProjectTreeModel::Roles::Path)).toString();
              itemIsDir = model->data(index, treeRole(ProjectTreeModel::Roles::IsDir)).toBool();
              targetDir = itemIsDir ? itemPath : QFileInfo(itemPath).absolutePath();
          }

          QMenu menu(treeView);
          QAction *newFileAction = menu.addAction(QObject::tr("New File"));
          QAction *newFolderAction = menu.addAction(QObject::tr("New Folder"));
          QAction *renameAction = nullptr;
          QAction *deleteAction = nullptr;
          QAction *addToChatAction = nullptr;
          QAction *addToNewChatAction = nullptr;
          if (hasItem) {
              menu.addSeparator();
              renameAction = menu.addAction(QObject::tr("Rename"));
              deleteAction = menu.addAction(QObject::tr("Delete"));
              // A folder attaches its contents, which is why the two entries
              // read the same for a file and a folder: what differs is the
              // rule `ai_chat_core::expand_folder` applies, not the gesture.
              menu.addSeparator();
              addToChatAction = menu.addAction(QObject::tr("Add to AI Chat"));
              addToNewChatAction = menu.addAction(QObject::tr("Add to New AI Chat"));
          }

          QAction *chosen = menu.exec(treeView->viewport()->mapToGlobal(pos));
          if (!chosen) {
              return;
          }

          if (chosen == newFileAction) {
              const QString name = QInputDialog::getText(window, QObject::tr("New File"),
                                                           QObject::tr("File name:"));
              if (name.isEmpty()) {
                  return;
              }
              const auto result = treeModel->createFile(targetDir, name);
              if (result.code != 0) {
                  QMessageBox::critical(window, QObject::tr("Cannot create file"), result.message);
              }
          } else if (chosen == newFolderAction) {
              const QString name = QInputDialog::getText(window, QObject::tr("New Folder"),
                                                           QObject::tr("Folder name:"));
              if (name.isEmpty()) {
                  return;
              }
              const auto result = treeModel->createFolder(targetDir, name);
              if (result.code != 0) {
                  QMessageBox::critical(window, QObject::tr("Cannot create folder"),
                                         result.message);
              }
          } else if (chosen == renameAction) {
              const QString currentName = QFileInfo(itemPath).fileName();
              const QString newName = QInputDialog::getText(window, QObject::tr("Rename"),
                                                              QObject::tr("New name:"),
                                                              QLineEdit::Normal, currentName);
              if (newName.isEmpty() || newName == currentName) {
                  return;
              }
              const auto result = treeModel->renamePath(itemPath, newName);
              if (result.code != 0) {
                  QMessageBox::critical(window, QObject::tr("Cannot rename"), result.message);
              }
          } else if (chosen == deleteAction) {
              const QString itemName = QFileInfo(itemPath).fileName();
              const QString warning = itemIsDir
                ? QObject::tr("Delete folder \"%1\" and everything inside it? "
                               "This deletes its contents recursively and cannot be undone.")
                    .arg(itemName)
                : QObject::tr("Delete \"%1\"? This cannot be undone.").arg(itemName);
              const auto choice = QMessageBox::warning(window,
                                                         QObject::tr("Confirm delete"),
                                                         warning,
                                                         QMessageBox::Yes | QMessageBox::No,
                                                         QMessageBox::No);
              if (choice != QMessageBox::Yes) {
                  return;
              }
              const auto result = treeModel->deletePath(itemPath);
              if (result.code != 0) {
                  QMessageBox::critical(window, QObject::tr("Cannot delete"), result.message);
              }
          } else if (chosen == addToChatAction || chosen == addToNewChatAction) {
              if (chosen == addToNewChatAction) {
                  actions.aiChat->newConversation();
              }
              // A folder attaches its contents and a file attaches itself;
              // which files a folder yields, and what to say about the ones
              // it did not, are both `ai-chat-core`'s answers.
              const FfiResult result = itemIsDir ? actions.aiChat->attachFolder(itemPath)
                                                  : actions.aiChat->attachFile(itemPath);
              if (result.code != 0) {
                  QMessageBox::information(window, QObject::tr("AI Chat"), result.message);
                  return;
              }
              actions.docks->show(QStringLiteral("aiChat"));
              // A folder's summary names what was skipped and what did not
              // fit; a single file has nothing to report, so only the folder
              // case is worth a line in the status bar.
              if (itemIsDir && !QString(result.message).isEmpty()) {
                  window->statusBar()->showMessage(result.message, 8000);
              }
              actions.aiChatPanel->attachAndFocus();
          }
      });
}

void wireProjectTreeViewAction(QMenu *viewMenu,
                               DockRegistry *docks,
                               AppSettings *appSettings,
                               QHash<QString, QAction *> &actions)
{
    QAction *projectTreeAction = registerAction(viewMenu, QStringLiteral("view.projectTree"),
                                                QObject::tr("Project"), appSettings, actions);
    QObject::connect(projectTreeAction, &QAction::triggered, viewMenu,
                     [docks]() { docks->show(QStringLiteral("projectTree")); });
}

} // namespace ui_shell
