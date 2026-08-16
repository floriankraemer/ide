#include "main_window.h"

#include "code_editor.h"
#include "theme.h"
#include "ui-shell/src/bridge.cxxqt.h"

#include <QApplication>
#include <QCloseEvent>
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
#include <QRect>
#include <QSplitter>
#include <QStringList>
#include <QTabWidget>
#include <QtGui/QTextDocument>
#include <QTreeView>
#include <QVariant>
#include <QWidget>

namespace ui_shell {

namespace {

// app_core::AppError's stable code for the binary-open rejection (ADR-0003,
// pinned by app-core's error_codes_are_stable test) — the one error kind the
// view presents as information rather than as an error.
constexpr int kErrBinaryFile = 3;

// Humble view for the tab strip (ADR-0002): owns the QTabWidget <->
// DocumentManager wiring, decides nothing. Tabs are identified by the
// session's stable TabId (ADR-0003); the TabId <-> page-index mapping lives
// here and only here, as a dynamic property on each page widget — an id
// never shifts when other tabs close, so there is no index lockstep to
// maintain and no parallel title list to keep in sync.
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
        // G2: drag-reorder is safe with no adapter/app-core change because
        // TabId is looked up by scanning each page's dynamic property, not
        // by a maintained index map (see tabIdAt/indexOfTab below) — a
        // reorder can't desynchronize anything.
        tabWidget_->setMovable(true);

        connect(docManager_, &DocumentManager::tabOpened, this, &EditorTabs::onTabOpened);
        connect(docManager_, &DocumentManager::tabClosed, this, &EditorTabs::onTabClosed);
        connect(docManager_,
                &DocumentManager::tabModifiedChanged,
                this,
                &EditorTabs::onTabModifiedChanged);
        connect(tabWidget_, &QTabWidget::tabCloseRequested, this, &EditorTabs::requestCloseTab);
        connect(tabWidget_, &QTabWidget::currentChanged, docManager_, [this](int index) {
            if (index >= 0) {
                docManager_->setActiveTab(tabIdAt(index));
            }
        });
    }

    // Opens `path`, or focuses its tab if already open (US-3). The session
    // decides whether the file may open (binary rejection, readability);
    // this only picks the dialog flavor by error code and shows the result.
    void openFile(const QString &path)
    {
        const auto result = docManager_->openFile(path);
        if (result.code == kErrBinaryFile) {
            QMessageBox::information(window_, tr("Cannot open file"), result.message);
            return;
        }
        if (result.code != 0) {
            QMessageBox::critical(window_, tr("Cannot open file"), result.message);
            return;
        }
        tabWidget_->setCurrentIndex(indexOfTab(result.tab_id));
    }

    QPlainTextEdit *currentEditor() const
    {
        return qobject_cast<QPlainTextEdit *>(tabWidget_->currentWidget());
    }

    // Ctrl+S / File > Save.
    void saveCurrentTab() { saveTab(tabWidget_->currentIndex()); }

    // File > Save As... (L2): the session repoints the tab at the chosen
    // path and writes there; the tree's own watcher picks up the new file
    // for free (no explicit tree-refresh call needed here).
    void saveCurrentTabAs()
    {
        const int index = tabWidget_->currentIndex();
        if (index < 0) {
            return;
        }
        auto *editor = qobject_cast<QPlainTextEdit *>(tabWidget_->widget(index));
        if (!editor) {
            return;
        }
        const QString path = QFileDialog::getSaveFileName(window_, tr("Save As"));
        if (path.isEmpty()) {
            return;
        }
        const quint64 tabId = tabIdAt(index);
        const auto result = docManager_->saveTabAs(tabId, path, editor->toPlainText());
        if (result.code != 0) {
            QMessageBox::critical(window_, tr("Cannot save file"), result.message);
            return;
        }
        editor->document()->setModified(false);
        renderTabText(index, docManager_->tabTitle(tabId), false);
    }

    // Exit / window-close (L1): runs the same unsaved-changes prompt as
    // closing tabs one at a time, stopping at the first Cancel so the
    // caller can abort the close.
    bool confirmCloseAllTabs()
    {
        for (int i = 0; i < tabWidget_->count(); ++i) {
            if (!confirmCloseTab(i)) {
                return false;
            }
        }
        return true;
    }

    // Rename/delete via the tree changed a tab's title (US-2b) — re-render
    // the label, preserving the unsaved-changes indicator.
    void onTabTitleChanged(quint64 tabId, const QString &title)
    {
        const int index = indexOfTab(tabId);
        if (index < 0) {
            return;
        }
        renderTabText(index, title, docManager_->tabIsModified(tabId));
    }

    // US-3's external-change prompt: the tab `tabId` (backed by `path`) was
    // modified outside the editor (filesystem watcher). "Reload" re-reads
    // the file from disk, discarding in-editor edits; "Keep" leaves the
    // editor content untouched but marks the tab dirty, since it's now
    // known to differ from what's on disk.
    void handleExternalChange(quint64 tabId, const QString &path)
    {
        const int index = indexOfTab(tabId);
        if (index < 0) {
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
            const auto result = docManager_->reloadTabFromDisk(tabId);
            if (result.code != 0) {
                QMessageBox::critical(window_, tr("Cannot reload file"), result.message);
                return;
            }
            editor->setPlainText(docManager_->tabContent(tabId));
            editor->document()->setModified(false);
        } else {
            editor->document()->setModified(true);
        }
    }

private:
    // The one TabId <-> index mapping (ADR-0003): the id rides on the page
    // widget itself, so closes and reorders can never desynchronize it.
    quint64 tabIdAt(int index) const
    {
        QWidget *widget = tabWidget_->widget(index);
        return widget ? widget->property("tabId").toULongLong() : 0;
    }

    int indexOfTab(quint64 tabId) const
    {
        for (int i = 0; i < tabWidget_->count(); ++i) {
            if (tabIdAt(i) == tabId) {
                return i;
            }
        }
        return -1;
    }

    // Label rendering: the session's display title verbatim, plus the
    // view's own unsaved-changes dot.
    void renderTabText(int index, const QString &title, bool modified)
    {
        tabWidget_->setTabText(index, modified ? title + QStringLiteral(" •") : title);
    }

    // Writes the tab's content to disk. Shows an error dialog and leaves the
    // dirty state set on failure (US-4: no silent data loss). Returns
    // whether the save succeeded.
    bool saveTab(int index)
    {
        auto *editor = qobject_cast<QPlainTextEdit *>(tabWidget_->widget(index));
        if (!editor) {
            return false;
        }
        const auto result = docManager_->saveTab(tabIdAt(index), editor->toPlainText());
        if (result.code != 0) {
            QMessageBox::critical(window_, tr("Cannot save file"), result.message);
            return false;
        }
        editor->document()->setModified(false);
        return true;
    }

    // Save/Discard/Cancel prompt for a tab with unsaved changes (US-3/US-4).
    // Returns true if the tab is now safe to close. Dirtiness is read from
    // the session — Rust owns that flag (ADR-0003).
    bool confirmCloseTab(int index)
    {
        if (!docManager_->tabIsModified(tabIdAt(index))) {
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
        docManager_->closeTab(tabIdAt(index));
    }

    void onTabOpened(quint64 tabId, const QString &title)
    {
        auto *editor = new CodeEditor(tabWidget_);
        editor->setProperty("tabId", QVariant::fromValue(tabId));
        editor->setPlainText(docManager_->tabContent(tabId));
        editor->document()->setModified(false);

        // Forward QPlainTextEdit's own modified state into the session's
        // authoritative dirty flag (ADR-0003) rather than marshalling
        // keystrokes. The stable id is captured by value — unlike a tab
        // index, it never shifts when other tabs close.
        connect(editor->document(),
                &QTextDocument::modificationChanged,
                docManager_,
                [this, tabId](bool modified) { docManager_->setTabModified(tabId, modified); });

        tabWidget_->addTab(editor, title);
    }

    void onTabClosed(quint64 tabId)
    {
        const int index = indexOfTab(tabId);
        if (index < 0) {
            return;
        }
        QWidget *widget = tabWidget_->widget(index);
        tabWidget_->removeTab(index);
        delete widget;
    }

    void onTabModifiedChanged(quint64 tabId, bool modified)
    {
        const int index = indexOfTab(tabId);
        if (index < 0) {
            return;
        }
        renderTabText(index, docManager_->tabTitle(tabId), modified);
    }

    DocumentManager *docManager_;
    QTabWidget *tabWidget_;
    QWidget *window_;
};

// Subclassed so closeEvent() can run the same unsaved-changes prompt as
// closing a tab, and persist geometry on close (L1). No Q_OBJECT: overriding
// a virtual function needs no signals/slots/qobject_cast, so this adds no
// second moc target.
class IdeMainWindow : public QMainWindow
{
public:
    void setEditorTabs(EditorTabs *editorTabs) { editorTabs_ = editorTabs; }
    void setAppSettings(AppSettings *appSettings) { appSettings_ = appSettings; }

protected:
    void closeEvent(QCloseEvent *event) override
    {
        if (editorTabs_ && !editorTabs_->confirmCloseAllTabs()) {
            event->ignore();
            return;
        }
        if (appSettings_) {
            const QRect g = geometry();
            appSettings_->saveWindowGeometry(g.x(), g.y(), static_cast<quint32>(g.width()),
                                              static_cast<quint32>(g.height()));
        }
        QMainWindow::closeEvent(event);
    }

private:
    EditorTabs *editorTabs_ = nullptr;
    AppSettings *appSettings_ = nullptr;
};

// File > Recent Projects (C2): rebuilds `menu` from `appSettings`'s
// persisted list. Forward-declared so it and openProjectAndRefreshRecents
// (below) can call each other without any lambda self-capture.
void populateRecentProjectsMenu(QMenu *menu, AppSettings *appSettings, ProjectTreeModel *treeModel,
                                 QMainWindow *window);

// Shared tail for "Open Folder..." and clicking a Recent Projects entry:
// open, report failure, and on success refresh the menu so the
// just-opened path moves to the front (C2).
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
    for (qsizetype i = 0; i < projects.size(); ++i) {
        const QString path = projects.at(i);
        QAction *action = menu->addAction(path);
        QObject::connect(action, &QAction::triggered, treeModel,
                          [treeModel, window, menu, appSettings, path]() {
                              openProjectAndRefreshRecents(treeModel, window, menu, appSettings,
                                                            path);
                          });
    }
}

// Sidebar tree + tabbed editor area, PHPStorm-style (US-5): a resizable
// splitter with the project tree on the left and the tab strip on the
// right.
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

    // Filesystem-watcher plumbing: ProjectTreeModel's watcher-driven signal
    // already carries the changed path and already runs on the Qt thread
    // (queued there via CxxQtThread), so relaying it to DocumentManager is a
    // plain same-thread signal/slot connection — no further cross-thread
    // hop. The session decides whether the change warrants a prompt.
    QObject::connect(treeModel,
                      &ProjectTreeModel::filesChangedExternally,
                      docManager,
                      [docManager](const QString &path) { docManager->checkExternalChange(path); });

    QObject::connect(docManager,
                      &DocumentManager::externalChangeDetected,
                      editorTabs,
                      [editorTabs](quint64 tabId, const QString &path) {
                          editorTabs->handleExternalChange(tabId, path);
                      });

    // A tree-driven rename/delete retitled an open tab (US-2b).
    QObject::connect(treeModel,
                      &ProjectTreeModel::tabTitleChanged,
                      editorTabs,
                      [editorTabs](quint64 tabId, const QString &title) {
                          editorTabs->onTabTitleChanged(tabId, title);
                      });

    QObject::connect(
      treeView,
      &QTreeView::clicked,
      treeModel,
      [treeModel, editorTabs](const QModelIndex &index) {
          const bool isDir =
            treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::IsDir)).toBool();
          if (isDir) {
              return;
          }

          const QString path =
            treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::Path)).toString();
          editorTabs->openFile(path);
      });

    // Right-click context menu: create/rename/delete from the tree (US-2b).
    // Pure intent-forwarding: dialogs gather names/confirmation, the session
    // performs the operation (including retargeting any open tab).
    treeView->setContextMenuPolicy(Qt::CustomContextMenu);
    QObject::connect(
      treeView,
      &QTreeView::customContextMenuRequested,
      treeView,
      [treeView, treeModel, window](const QPoint &pos) {
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
          }
      });

    return editorTabs;
}

// Menu structure per US-5 acceptance criteria. "Open Folder..." and the
// Edit/Save actions are wired to the tabbed editor area; the rest remain
// non-functional stubs for later tasks.
QMainWindow *buildMainWindow()
{
    auto *window = new IdeMainWindow();
    window->setWindowTitle(QStringLiteral("IDE"));

    auto *appSettings = new AppSettings(window);
    // Live-switch mechanism (T2): re-applying setStyleSheet() at runtime is
    // exactly how a future theme-picker (S1) switches without a restart —
    // this call is that same path, just fired once at startup with the
    // persisted theme instead of a freshly-picked one.
    qApp->setStyleSheet(styleSheetForTheme(appSettings->themeName()));

    const FfiWindowGeometry savedGeometry = appSettings->windowGeometry();
    if (savedGeometry.width > 0 && savedGeometry.height > 0) {
        window->setGeometry(savedGeometry.x, savedGeometry.y,
                             static_cast<int>(savedGeometry.width),
                             static_cast<int>(savedGeometry.height));
    } else {
        window->resize(1024, 768);
    }

    auto *treeModel = new ProjectTreeModel(window);
    auto *docManager = new DocumentManager(window);
    EditorTabs *editorTabs = buildCentralWidget(window, treeModel, docManager);
    window->setEditorTabs(editorTabs);
    window->setAppSettings(appSettings);

    QMenu *fileMenu = window->menuBar()->addMenu(QObject::tr("&File"));
    QAction *openFolderAction = fileMenu->addAction(QObject::tr("Open Folder..."));
    QMenu *recentProjectsMenu = fileMenu->addMenu(QObject::tr("Recent Projects"));
    populateRecentProjectsMenu(recentProjectsMenu, appSettings, treeModel, window);
    fileMenu->addSeparator();
    QAction *saveAction = fileMenu->addAction(QObject::tr("Save"));
    saveAction->setShortcut(QKeySequence::Save);
    QAction *saveAsAction = fileMenu->addAction(QObject::tr("Save As..."));
    fileMenu->addSeparator();
    QAction *exitAction = fileMenu->addAction(QObject::tr("Exit"));

    QObject::connect(openFolderAction, &QAction::triggered, window,
                      [treeModel, window, recentProjectsMenu, appSettings]() {
                          const QString dir = QFileDialog::getExistingDirectory(
                            window, QObject::tr("Open Folder"), QString(),
                            QFileDialog::ShowDirsOnly);
                          if (dir.isEmpty()) {
                              return;
                          }
                          openProjectAndRefreshRecents(treeModel, window, recentProjectsMenu,
                                                        appSettings, dir);
                      });

    QObject::connect(exitAction, &QAction::triggered, window, [window]() { window->close(); });

    QObject::connect(saveAction, &QAction::triggered, window, [editorTabs]() {
        editorTabs->saveCurrentTab();
    });

    QObject::connect(saveAsAction, &QAction::triggered, window, [editorTabs]() {
        editorTabs->saveCurrentTabAs();
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
    // Reuses the same watcher-start path as "Open Folder...", so the tree
    // is live-refreshing from the moment it's populated.
    treeModel->reopenLastProject();

    return window;
}

} // namespace

int run_app()
{
    int argc = 0;
    QApplication app(argc, nullptr);

    // buildMainWindow() applies the persisted theme (T2) once AppSettings
    // exists; nothing is shown yet at this point, so there's no unstyled
    // frame to flash.
    QMainWindow *window = buildMainWindow();
    window->show();

    return QApplication::exec();
}

} // namespace ui_shell
