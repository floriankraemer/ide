#include "main_window.h"

#include "ui-shell/src/bridge.cxxqt.h"

#include <QApplication>
#include <QFileDialog>
#include <QFileInfo>
#include <QInputDialog>
#include <QLineEdit>
#include <QMainWindow>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QPlainTextEdit>
#include <QPoint>
#include <QPushButton>
#include <QSplitter>
#include <QStringList>
#include <QTabWidget>
#include <QtGui/QTextDocument>
#include <QTreeView>
#include <QVariant>
#include <QWidget>

#include <QDebug>

namespace ui_shell {

namespace {

// Owns the tab strip <-> DocumentManager wiring (Task 6, US-3/US-4): keeps
// each QTabWidget page's QPlainTextEdit and tab index in lockstep with
// editor-core's TabList by only ever mutating both sides through the same
// DocumentManager signals (tabOpened/tabClosed keep both lists reordered
// identically, so a plain parallel QStringList of base titles is enough —
// no separate index-mapping table needed).
class EditorTabs : public QObject
{
public:
    EditorTabs(DocumentManager *docManager, QTabWidget *tabWidget, QWidget *window)
      : docManager_(docManager)
      , tabWidget_(tabWidget)
      , window_(window)
    {
        tabWidget_->setTabsClosable(true);
        tabWidget_->setUsesScrollButtons(true);
        tabWidget_->setMovable(false);

        connect(docManager_, &DocumentManager::tabOpened, this, &EditorTabs::onTabOpened);
        connect(docManager_, &DocumentManager::tabClosed, this, &EditorTabs::onTabClosed);
        connect(docManager_,
                &DocumentManager::tabModifiedChanged,
                this,
                &EditorTabs::onTabModifiedChanged);
        connect(docManager_,
                &DocumentManager::tabTitleChanged,
                this,
                &EditorTabs::onTabTitleChanged);
        connect(tabWidget_, &QTabWidget::tabCloseRequested, this, &EditorTabs::requestCloseTab);
        connect(tabWidget_, &QTabWidget::currentChanged, docManager_, [this](int index) {
            if (index >= 0) {
                docManager_->setActiveTab(index);
            }
        });
    }

    // Opens `path`, or focuses its tab if already open (US-3). Shows an
    // error dialog on failure (e.g. unreadable/non-UTF8 file).
    void openFile(const QString &path)
    {
        const int index = docManager_->openFile(path);
        if (index < 0) {
            QMessageBox::critical(window_, tr("Cannot open file"), docManager_->lastError());
            return;
        }
        tabWidget_->setCurrentIndex(index);
    }

    QPlainTextEdit *currentEditor() const
    {
        return qobject_cast<QPlainTextEdit *>(tabWidget_->currentWidget());
    }

    // Ctrl+S / File > Save.
    void saveCurrentTab() { saveTab(tabWidget_->currentIndex()); }

    // US-3's external-change prompt: the tab at `index` (backed by `path`)
    // was modified outside the editor (filesystem watcher, Task 8).
    // "Reload" re-reads the file from disk, discarding in-editor edits;
    // "Keep" leaves the editor content untouched but marks the tab dirty,
    // since it's now known to differ from what's on disk.
    void handleExternalChange(int index, const QString &path)
    {
        if (index < 0 || index >= tabWidget_->count()) {
            return;
        }
        auto *editor = qobject_cast<QPlainTextEdit *>(tabWidget_->widget(index));
        if (!editor) {
            return;
        }

        QMessageBox box(QMessageBox::Warning,
                         tr("File changed on disk"),
                         tr("\"%1\" was modified outside the editor.")
                           .arg(QFileInfo(path).fileName()),
                         QMessageBox::NoButton,
                         window_);
        QPushButton *reloadButton = box.addButton(tr("Reload"), QMessageBox::AcceptRole);
        box.addButton(tr("Keep My Version"), QMessageBox::RejectRole);
        box.setDefaultButton(reloadButton);
        box.exec();

        if (box.clickedButton() == reloadButton) {
            const QString error = docManager_->reloadTabFromDisk(index);
            if (!error.isEmpty()) {
                QMessageBox::critical(window_, tr("Cannot reload file"), error);
                return;
            }
            editor->setPlainText(docManager_->tabContent(index));
            editor->document()->setModified(false);
        } else {
            editor->document()->setModified(true);
        }
    }

private:
    // Writes the tab's content to disk. Shows an error dialog and leaves the
    // dirty state set on failure (US-4: no silent data loss). Returns
    // whether the save succeeded.
    bool saveTab(int index)
    {
        auto *editor = qobject_cast<QPlainTextEdit *>(tabWidget_->widget(index));
        if (!editor) {
            return false;
        }
        const QString error = docManager_->saveTab(index, editor->toPlainText());
        if (!error.isEmpty()) {
            QMessageBox::critical(window_, tr("Cannot save file"), error);
            return false;
        }
        editor->document()->setModified(false);
        return true;
    }

    // Save/Discard/Cancel prompt for a tab with unsaved changes (US-3/US-4).
    // Returns true if the tab is now safe to close.
    bool confirmCloseTab(int index)
    {
        auto *editor = qobject_cast<QPlainTextEdit *>(tabWidget_->widget(index));
        if (!editor || !editor->document()->isModified()) {
            return true;
        }

        const auto choice = QMessageBox::question(
          window_,
          tr("Unsaved changes"),
          tr("\"%1\" has unsaved changes. Save before closing?").arg(tabWidget_->tabText(index)),
          QMessageBox::Save | QMessageBox::Discard | QMessageBox::Cancel,
          QMessageBox::Save);

        if (choice == QMessageBox::Cancel) {
            return false;
        }
        if (choice == QMessageBox::Save) {
            return saveTab(index);
        }
        return true; // Discard.
    }

    void requestCloseTab(int index)
    {
        if (!confirmCloseTab(index)) {
            return;
        }
        docManager_->closeTab(index);
    }

    void onTabOpened(int index, const QString &title)
    {
        auto *editor = new QPlainTextEdit(tabWidget_);
        editor->setPlainText(docManager_->tabContent(index));
        editor->document()->setModified(false);

        // Mirror QPlainTextEdit's own dirty state into editor-core rather
        // than marshalling keystrokes (mvp-implementation-plan.md §2).
        // `editor->indexOf` is looked up by widget, not captured by value,
        // because tab indices shift when other tabs close.
        connect(editor->document(),
                &QTextDocument::modificationChanged,
                docManager_,
                [this, editor](bool modified) {
                    const int idx = tabWidget_->indexOf(editor);
                    if (idx >= 0) {
                        docManager_->setTabModified(idx, modified);
                    }
                });

        titles_.insert(index, title);
        tabWidget_->insertTab(index, editor, title);
    }

    void onTabClosed(int index)
    {
        if (index < 0 || index >= tabWidget_->count()) {
            return;
        }
        QWidget *widget = tabWidget_->widget(index);
        tabWidget_->removeTab(index);
        delete widget;
        titles_.removeAt(index);
    }

    void onTabModifiedChanged(int index, bool modified)
    {
        if (index < 0 || index >= titles_.size()) {
            return;
        }
        const QString base = titles_.at(index);
        tabWidget_->setTabText(index, modified ? base + QStringLiteral(" •") : base);
    }

    // A tree-driven rename or delete (US-2b) changed the tab's title
    // without opening/closing it — update the base title used by
    // onTabModifiedChanged so the unsaved-changes dot keeps working.
    void onTabTitleChanged(int index, const QString &title)
    {
        if (index < 0 || index >= titles_.size()) {
            return;
        }
        titles_[index] = title;
        auto *editor = qobject_cast<QPlainTextEdit *>(tabWidget_->widget(index));
        const bool modified = editor && editor->document()->isModified();
        tabWidget_->setTabText(index, modified ? title + QStringLiteral(" •") : title);
    }

    DocumentManager *docManager_;
    QTabWidget *tabWidget_;
    QWidget *window_;
    QStringList titles_; // Base titles (no dirty indicator), parallel to tabWidget_'s pages.
};

// Sidebar tree + tabbed editor area, PHPStorm-style (US-5): a resizable
// splitter with the project tree on the left and the tab strip on the
// right (Task 6).
EditorTabs *buildCentralWidget(QMainWindow *window, ProjectTreeModel *treeModel,
                                DocumentManager *docManager)
{
    auto *splitter = new QSplitter(Qt::Horizontal, window);

    auto *treeView = new QTreeView(splitter);
    treeView->setModel(treeModel);
    treeView->setHeaderHidden(true);
    splitter->addWidget(treeView);

    auto *tabWidget = new QTabWidget(splitter);
    splitter->addWidget(tabWidget);

    splitter->setStretchFactor(0, 0);
    splitter->setStretchFactor(1, 1);

    window->setCentralWidget(splitter);

    auto *editorTabs = new EditorTabs(docManager, tabWidget, window);

    // Filesystem-watcher plumbing (Task 8, mvp-implementation-plan.md §2):
    // ProjectTreeModel's watcher-driven signal already carries the changed
    // path and already runs on the Qt thread (queued there via
    // CxxQtThread), so relaying it to DocumentManager is a plain
    // same-thread signal/slot connection — no further cross-thread hop.
    QObject::connect(treeModel,
                      &ProjectTreeModel::filesChangedExternally,
                      docManager,
                      [docManager](const QString &path) { docManager->checkExternalChange(path); });

    QObject::connect(docManager,
                      &DocumentManager::externalChangeDetected,
                      docManager,
                      [docManager, editorTabs](const QString &path) {
                          const int index = docManager->tabIndexForPath(path);
                          editorTabs->handleExternalChange(index, path);
                      });

    QObject::connect(
      treeView,
      &QTreeView::clicked,
      treeModel,
      [treeModel, editorTabs, window](const QModelIndex &index) {
          const bool isDir =
            treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::IsDir)).toBool();
          if (isDir) {
              return;
          }

          const QString path =
            treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::Path)).toString();

          if (treeModel->isBinaryFile(path)) {
              QMessageBox::information(
                window,
                QObject::tr("Cannot open file"),
                QObject::tr("\"%1\" is a binary file and cannot be opened as text.").arg(path));
              return;
          }

          editorTabs->openFile(path);
      });

    // Right-click context menu: create/rename/delete from the tree (US-2b).
    treeView->setContextMenuPolicy(Qt::CustomContextMenu);
    QObject::connect(
      treeView,
      &QTreeView::customContextMenuRequested,
      treeView,
      [treeView, treeModel, docManager, window](const QPoint &pos) {
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
              itemPath =
                treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::Path)).toString();
              itemIsDir =
                treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::IsDir)).toBool();
              targetDir = itemIsDir ? itemPath : QFileInfo(itemPath).absolutePath();
          }

          QMenu menu(treeView);
          QAction *newFileAction = menu.addAction(QObject::tr("New File"));
          QAction *newFolderAction = menu.addAction(QObject::tr("New Folder"));
          QAction *renameAction = nullptr;
          QAction *deleteAction = nullptr;
          if (hasItem) {
              menu.addSeparator();
              renameAction = menu.addAction(QObject::tr("Rename"));
              deleteAction = menu.addAction(QObject::tr("Delete"));
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
              const QString error = treeModel->createFile(targetDir, name);
              if (!error.isEmpty()) {
                  QMessageBox::critical(window, QObject::tr("Cannot create file"), error);
              }
          } else if (chosen == newFolderAction) {
              const QString name = QInputDialog::getText(window, QObject::tr("New Folder"),
                                                           QObject::tr("Folder name:"));
              if (name.isEmpty()) {
                  return;
              }
              const QString error = treeModel->createFolder(targetDir, name);
              if (!error.isEmpty()) {
                  QMessageBox::critical(window, QObject::tr("Cannot create folder"), error);
              }
          } else if (chosen == renameAction) {
              const QString currentName = QFileInfo(itemPath).fileName();
              const QString newName = QInputDialog::getText(window, QObject::tr("Rename"),
                                                              QObject::tr("New name:"),
                                                              QLineEdit::Normal, currentName);
              if (newName.isEmpty() || newName == currentName) {
                  return;
              }
              const QString newPath = QFileInfo(itemPath).absolutePath() + QLatin1Char('/') + newName;
              const QString error = treeModel->renamePath(itemPath, newName);
              if (!error.isEmpty()) {
                  QMessageBox::critical(window, QObject::tr("Cannot rename"), error);
                  return;
              }
              docManager->notifyPathRenamed(itemPath, newPath);
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
              const QString error = treeModel->deletePath(itemPath);
              if (!error.isEmpty()) {
                  QMessageBox::critical(window, QObject::tr("Cannot delete"), error);
                  return;
              }
              docManager->notifyPathDeleted(itemPath);
          }
      });

    return editorTabs;
}

// Menu structure per US-5 acceptance criteria. "Open Folder..." and the
// Edit/Save actions are wired to the tabbed editor area (Task 6); the rest
// remain non-functional stubs for later tasks.
QMainWindow *buildMainWindow()
{
    auto *window = new QMainWindow();
    window->setWindowTitle(QStringLiteral("IDE"));
    window->resize(1024, 768);

    auto *treeModel = new ProjectTreeModel(window);
    auto *docManager = new DocumentManager(window);
    EditorTabs *editorTabs = buildCentralWidget(window, treeModel, docManager);

    QMenu *fileMenu = window->menuBar()->addMenu(QObject::tr("&File"));
    QAction *openFolderAction = fileMenu->addAction(QObject::tr("Open Folder..."));
    QAction *saveAction = fileMenu->addAction(QObject::tr("Save"));
    saveAction->setShortcut(QKeySequence::Save);
    fileMenu->addAction(QObject::tr("Save As..."));
    fileMenu->addSeparator();
    fileMenu->addAction(QObject::tr("Exit"));

    QObject::connect(openFolderAction, &QAction::triggered, window, [treeModel, window]() {
        const QString dir = QFileDialog::getExistingDirectory(
          window, QObject::tr("Open Folder"), QString(), QFileDialog::ShowDirsOnly);
        if (dir.isEmpty()) {
            return;
        }

        const QString error = treeModel->openFolder(dir);
        if (!error.isEmpty()) {
            QMessageBox::critical(window, QObject::tr("Cannot open folder"), error);
        }
    });

    QObject::connect(saveAction, &QAction::triggered, window, [editorTabs]() {
        editorTabs->saveCurrentTab();
    });

    QMenu *editMenu = window->menuBar()->addMenu(QObject::tr("&Edit"));
    QAction *undoAction = editMenu->addAction(QObject::tr("Undo"));
    undoAction->setShortcut(QKeySequence::Undo);
    QAction *redoAction = editMenu->addAction(QObject::tr("Redo"));
    redoAction->setShortcut(QKeySequence::Redo);
    editMenu->addSeparator();
    QAction *cutAction = editMenu->addAction(QObject::tr("Cut"));
    cutAction->setShortcut(QKeySequence::Cut);
    QAction *copyAction = editMenu->addAction(QObject::tr("Copy"));
    copyAction->setShortcut(QKeySequence::Copy);
    QAction *pasteAction = editMenu->addAction(QObject::tr("Paste"));
    pasteAction->setShortcut(QKeySequence::Paste);

    QObject::connect(undoAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->undo();
        }
    });
    QObject::connect(redoAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->redo();
        }
    });
    QObject::connect(cutAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->cut();
        }
    });
    QObject::connect(copyAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->copy();
        }
    });
    QObject::connect(pasteAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->paste();
        }
    });

    // US-1: relaunching the app reopens the last project automatically.
    // Reuses the same watcher-start path as "Open Folder..." (Task 8), so
    // the tree is live-refreshing from the moment it's populated.
    treeModel->reopenLastProject();

    return window;
}

} // namespace

int run_app()
{
    int argc = 0;
    QApplication app(argc, nullptr);

    QMainWindow *window = buildMainWindow();
    window->show();

    return QApplication::exec();
}

} // namespace ui_shell
