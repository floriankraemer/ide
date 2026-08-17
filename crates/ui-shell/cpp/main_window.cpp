#include "main_window.h"

#include "code_editor.h"
#include "syntax_highlighter.h"
#include "theme.h"
#include "ui-shell/src/bridge.cxxqt.h"

#include "DockManager.h"
#include "DockWidget.h"

#include <QApplication>
#include <QByteArray>
#include <QCloseEvent>
#include <QColor>
#include <QColorDialog>
#include <QComboBox>
#include <QDialog>
#include <QDialogButtonBox>
#include <QFileDialog>
#include <QFileInfo>
#include <QFont>
#include <QFormLayout>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QStatusBar>
#include <QTextCursor>
#include <memory>
#include <QMainWindow>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QPalette>
#include <QPlainTextEdit>
#include <QPoint>
#include <QPushButton>
#include <QRect>
#include <QSpinBox>
#include <QStackedWidget>
#include <QStringList>
#include <QTabWidget>
#include <QtGui/QTextDocument>
#include <QTreeView>
#include <QVariant>
#include <QVBoxLayout>
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
            updateStatusBar();
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

    // L3: registers the status bar's line:col and language labels, and
    // fills them in immediately for whatever tab is already current.
    void attachStatusBar(QLabel *positionLabel, QLabel *languageLabel)
    {
        positionLabel_ = positionLabel;
        languageLabel_ = languageLabel;
        updateStatusBar();
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

    // S2 live-apply: updates every open tab immediately and remembers the
    // choice so tabs opened afterward pick it up too. No persistence here —
    // the settings dialog decides via AppSettings whether to keep (OK) or
    // revert (Cancel) this.
    void setEditorFont(const QFont &font)
    {
        editorFont_ = font;
        for (int i = 0; i < tabWidget_->count(); ++i) {
            if (auto *editor = qobject_cast<QPlainTextEdit *>(tabWidget_->widget(i))) {
                editor->setFont(font);
            }
        }
    }

    // `backgroundHex`/`foregroundHex` empty means "use the theme's default
    // palette role" (A3): starting from qApp's own palette and overriding
    // only the roles with a value keeps that default live even after a
    // theme switch, rather than freezing whatever color was current when
    // the override was set.
    void setEditorColors(const QString &backgroundHex, const QString &foregroundHex)
    {
        editorBackground_ = backgroundHex;
        editorForeground_ = foregroundHex;
        for (int i = 0; i < tabWidget_->count(); ++i) {
            if (auto *editor = qobject_cast<QPlainTextEdit *>(tabWidget_->widget(i))) {
                applyEditorPalette(editor);
            }
        }
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

    // L3: line:col + language for whatever tab is current, or blank when
    // no tab is open. The "UTF-8" label is static (set once in
    // buildMainWindow) since only UTF-8 is supported today — nothing here
    // needs to touch it.
    void updateStatusBar()
    {
        if (!positionLabel_ || !languageLabel_) {
            return;
        }
        auto *editor = currentEditor();
        if (!editor) {
            positionLabel_->clear();
            languageLabel_->clear();
            return;
        }
        const QTextCursor cursor = editor->textCursor();
        positionLabel_->setText(QObject::tr("Ln %1, Col %2")
                                   .arg(cursor.blockNumber() + 1)
                                   .arg(cursor.columnNumber() + 1));
        languageLabel_->setText(docManager_->tabLanguageName(tabIdAt(tabWidget_->currentIndex())));
    }

    // Shared with setEditorColors, and with onTabOpened's initial apply.
    void applyEditorPalette(QPlainTextEdit *editor)
    {
        QPalette pal = qApp->palette();
        if (!editorBackground_.isEmpty()) {
            pal.setColor(QPalette::Base, QColor(editorBackground_));
        }
        if (!editorForeground_.isEmpty()) {
            pal.setColor(QPalette::Text, QColor(editorForeground_));
        }
        editor->setPalette(pal);
    }

    void onTabOpened(quint64 tabId, const QString &title)
    {
        auto *editor = new CodeEditor(tabWidget_);
        editor->setProperty("tabId", QVariant::fromValue(tabId));
        editor->setPlainText(docManager_->tabContent(tabId));
        editor->document()->setModified(false);
        editor->setFont(editorFont_);
        applyEditorPalette(editor);
        // Y2: self-parents to editor->document(), no manual lifetime
        // management needed. PlainText (unrecognized/no extension) yields
        // no spans from highlight_line, so this is a harmless no-op then.
        new SyntaxHighlighter(editor->document(), docManager_->tabExtension(tabId));

        // L3: only the visible tab's cursor should move the status bar —
        // guards against a background tab's programmatic cursor change
        // (e.g. a reload) touching labels that describe a different tab.
        connect(editor, &QPlainTextEdit::cursorPositionChanged, this, [this, editor]() {
            if (tabWidget_->currentWidget() == editor) {
                updateStatusBar();
            }
        });

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
    QFont editorFont_;
    QString editorBackground_;
    QString editorForeground_;
    QLabel *positionLabel_ = nullptr;
    QLabel *languageLabel_ = nullptr;
};

// Subclassed so closeEvent() can run the same unsaved-changes prompt as
// closing a tab, and persist geometry + dock layout on close (L1, D4). No
// Q_OBJECT: overriding a virtual function needs no signals/slots/
// qobject_cast, so this adds no second moc target.
class IdeMainWindow : public QMainWindow
{
public:
    void setEditorTabs(EditorTabs *editorTabs) { editorTabs_ = editorTabs; }
    void setAppSettings(AppSettings *appSettings) { appSettings_ = appSettings; }
    void setDockManager(ads::CDockManager *dockManager) { dockManager_ = dockManager; }

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
            if (dockManager_) {
                // D4: window_state is a plain Rust String (must be valid
                // UTF-8); ADS's saveState() returns raw QByteArray, so
                // base64 round-trips it through that constraint.
                const QString state =
                  QString::fromLatin1(dockManager_->saveState().toBase64());
                appSettings_->saveWindowState(state);
            }
        }
        QMainWindow::closeEvent(event);
    }

private:
    EditorTabs *editorTabs_ = nullptr;
    AppSettings *appSettings_ = nullptr;
    ads::CDockManager *dockManager_ = nullptr;
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

// Settings dialog (S1: category list + stacked detail pane; S2: font +
// editor colors on the Editor page). Every control applies live as it's
// changed (theme via qApp->setStyleSheet(), font/colors via `editorTabs`) so
// the effect is visible immediately; OK persists that already-applied state
// through `appSettings`, Cancel restores exactly what was active when the
// dialog opened. Modal and blocking, so every lambda below capturing
// `&dialog` only ever runs while `dialog` is still alive on this stack frame.
void showSettingsDialog(QWidget *parent, AppSettings *appSettings, EditorTabs *editorTabs)
{
    const QString originalTheme = appSettings->themeName();
    const FfiEditorFont originalFont = appSettings->editorFont();
    const FfiEditorColors originalColors = appSettings->editorColors();

    QDialog dialog(parent);
    dialog.setWindowTitle(QObject::tr("Settings"));

    auto *categoryList = new QListWidget(&dialog);
    categoryList->addItem(QObject::tr("Appearance"));
    categoryList->addItem(QObject::tr("Editor"));
    categoryList->setMaximumWidth(150);

    auto *pages = new QStackedWidget(&dialog);

    auto *appearancePage = new QWidget(&dialog);
    auto *appearanceForm = new QFormLayout(appearancePage);
    auto *themeCombo = new QComboBox(appearancePage);
    themeCombo->addItem(QObject::tr("Dark"), QStringLiteral("dark"));
    themeCombo->addItem(QObject::tr("Light"), QStringLiteral("light"));
    themeCombo->setCurrentIndex(originalTheme == QStringLiteral("light") ? 1 : 0);
    appearanceForm->addRow(QObject::tr("Theme:"), themeCombo);
    pages->addWidget(appearancePage);

    QObject::connect(themeCombo, &QComboBox::currentIndexChanged, &dialog, [themeCombo]() {
        qApp->setStyleSheet(styleSheetForTheme(themeCombo->currentData().toString()));
    });

    auto *editorPage = new QWidget(&dialog);
    auto *editorForm = new QFormLayout(editorPage);
    auto *fontFamilyEdit = new QLineEdit(originalFont.family, editorPage);
    auto *fontSizeSpin = new QSpinBox(editorPage);
    fontSizeSpin->setRange(6, 72);
    fontSizeSpin->setValue(static_cast<int>(originalFont.size));
    editorForm->addRow(QObject::tr("Font family:"), fontFamilyEdit);
    editorForm->addRow(QObject::tr("Font size:"), fontSizeSpin);

    auto applyFontLive = [editorTabs, fontFamilyEdit, fontSizeSpin]() {
        editorTabs->setEditorFont(QFont(fontFamilyEdit->text(), fontSizeSpin->value()));
    };
    QObject::connect(fontFamilyEdit, &QLineEdit::textChanged, &dialog, applyFontLive);
    QObject::connect(fontSizeSpin, &QSpinBox::valueChanged, &dialog, applyFontLive);

    // Boxed so the color-picker lambdas (which need to both read and update
    // the chosen value across separate clicks) share one instance rather
    // than each capturing a stale copy.
    auto backgroundColor = std::make_shared<QString>(originalColors.background);
    auto foregroundColor = std::make_shared<QString>(originalColors.foreground);
    auto applyColorsLive = [editorTabs, backgroundColor, foregroundColor]() {
        editorTabs->setEditorColors(*backgroundColor, *foregroundColor);
    };

    auto *backgroundButton = new QPushButton(QObject::tr("Background Color..."), editorPage);
    QObject::connect(backgroundButton, &QPushButton::clicked, &dialog,
                      [&dialog, backgroundColor, applyColorsLive]() {
                          const QColor initial = backgroundColor->isEmpty()
                            ? QColor(Qt::white)
                            : QColor(*backgroundColor);
                          const QColor chosen = QColorDialog::getColor(
                            initial, &dialog, QObject::tr("Background Color"));
                          if (chosen.isValid()) {
                              *backgroundColor = chosen.name();
                              applyColorsLive();
                          }
                      });
    editorForm->addRow(backgroundButton);

    auto *foregroundButton = new QPushButton(QObject::tr("Text Color..."), editorPage);
    QObject::connect(foregroundButton, &QPushButton::clicked, &dialog,
                      [&dialog, foregroundColor, applyColorsLive]() {
                          const QColor initial = foregroundColor->isEmpty()
                            ? QColor(Qt::black)
                            : QColor(*foregroundColor);
                          const QColor chosen = QColorDialog::getColor(
                            initial, &dialog, QObject::tr("Text Color"));
                          if (chosen.isValid()) {
                              *foregroundColor = chosen.name();
                              applyColorsLive();
                          }
                      });
    editorForm->addRow(foregroundButton);

    pages->addWidget(editorPage);

    QObject::connect(categoryList, &QListWidget::currentRowChanged, pages,
                      &QStackedWidget::setCurrentIndex);
    categoryList->setCurrentRow(0);

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, &dialog);
    QObject::connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    QObject::connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);

    auto *bodyLayout = new QHBoxLayout();
    bodyLayout->addWidget(categoryList);
    bodyLayout->addWidget(pages, 1);

    auto *mainLayout = new QVBoxLayout(&dialog);
    mainLayout->addLayout(bodyLayout);
    mainLayout->addWidget(buttons);

    if (dialog.exec() == QDialog::Accepted) {
        appSettings->saveTheme(themeCombo->currentData().toString());
        appSettings->saveEditorFont(fontFamilyEdit->text(),
                                     static_cast<quint32>(fontSizeSpin->value()));
        appSettings->saveEditorColors(*backgroundColor, *foregroundColor);
    } else {
        qApp->setStyleSheet(styleSheetForTheme(originalTheme));
        editorTabs->setEditorFont(QFont(originalFont.family, static_cast<int>(originalFont.size)));
        editorTabs->setEditorColors(originalColors.background, originalColors.foreground);
    }
}

// Sidebar tree + tabbed editor area, PHPStorm-style (US-5): each panel is
// its own ADS CDockWidget (D3) — float/redock each independently, room left
// for future dock widgets (search, run console, MCP activity log) without
// restructuring this function again. The tab strip stays one QTabWidget
// inside its dock widget (not one dock widget per open file, per the plan's
// migration scope) — G2's drag-reorder is unaffected either way, since it's
// internal to that QTabWidget.
// Return value of buildCentralWidget(): the tab-strip adapter (needed by
// menu wiring) plus the dock manager (needed by IdeMainWindow for D4's
// close-time saveState()) — one caller, so a tiny struct beats an
// out-param.
struct CentralWidgets
{
    EditorTabs *editorTabs;
    ads::CDockManager *dockManager;
};

CentralWidgets buildCentralWidget(QMainWindow *window, ProjectTreeModel *treeModel,
                                   DocumentManager *docManager, AppSettings *appSettings)
{
    // Constructing with `window` (a QMainWindow) as parent makes the dock
    // manager install itself as the central widget automatically (ADS's own
    // CDockManager::CDockManager) — no explicit setCentralWidget() call.
    auto *dockManager = new ads::CDockManager(window);

    auto *tabWidget = new QTabWidget();
    auto *editorDock = new ads::CDockWidget(dockManager, QObject::tr("Editor"));
    editorDock->setWidget(tabWidget);
    auto *editorArea = dockManager->addDockWidget(ads::CenterDockWidgetArea, editorDock);

    auto *treeView = new QTreeView();
    treeView->setModel(treeModel);
    treeView->setHeaderHidden(true);
    auto *treeDock = new ads::CDockWidget(dockManager, QObject::tr("Project"));
    treeDock->setWidget(treeView);
    dockManager->addDockWidget(ads::LeftDockWidgetArea, treeDock, editorArea);

    // D4: restored after both dock widgets exist for this layout to apply
    // to (ADS matches saved widgets by their title/object name). Empty
    // means nothing was ever saved — first launch, or window_state predates
    // D4 — so the layout built above (tree left of editor) stands as-is.
    const QString savedState = appSettings->windowState();
    if (!savedState.isEmpty()) {
        dockManager->restoreState(QByteArray::fromBase64(savedState.toLatin1()));
    }

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

    return CentralWidgets{editorTabs, dockManager};
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
    const CentralWidgets central = buildCentralWidget(window, treeModel, docManager, appSettings);
    EditorTabs *editorTabs = central.editorTabs;
    window->setEditorTabs(editorTabs);
    window->setAppSettings(appSettings);
    window->setDockManager(central.dockManager);

    // S2: applied before reopenLastProject() (below) opens any tabs, so
    // every tab — including ones opened at startup — starts with the
    // persisted font/colors rather than the QPlainTextEdit default.
    const FfiEditorFont savedFont = appSettings->editorFont();
    editorTabs->setEditorFont(QFont(savedFont.family, static_cast<int>(savedFont.size)));
    const FfiEditorColors savedColors = appSettings->editorColors();
    editorTabs->setEditorColors(savedColors.background, savedColors.foreground);

    // L3: line:col + language update per current tab / cursor move; "UTF-8"
    // is static since only UTF-8 is supported today (US-2b's binary-file
    // rejection already rules out anything else reaching an open tab).
    auto *statusBar = window->statusBar();
    auto *languageLabel = new QLabel(statusBar);
    auto *positionLabel = new QLabel(statusBar);
    auto *encodingLabel = new QLabel(QStringLiteral("UTF-8"), statusBar);
    statusBar->addPermanentWidget(languageLabel);
    statusBar->addPermanentWidget(positionLabel);
    statusBar->addPermanentWidget(encodingLabel);
    editorTabs->attachStatusBar(positionLabel, languageLabel);

    QMenu *fileMenu = window->menuBar()->addMenu(QObject::tr("&File"));
    QAction *openFolderAction = fileMenu->addAction(QObject::tr("Open Folder..."));
    QMenu *recentProjectsMenu = fileMenu->addMenu(QObject::tr("Recent Projects"));
    populateRecentProjectsMenu(recentProjectsMenu, appSettings, treeModel, window);
    fileMenu->addSeparator();
    QAction *saveAction = fileMenu->addAction(QObject::tr("Save"));
    saveAction->setShortcut(QKeySequence::Save);
    QAction *saveAsAction = fileMenu->addAction(QObject::tr("Save As..."));
    fileMenu->addSeparator();
    QAction *preferencesAction = fileMenu->addAction(QObject::tr("Preferences..."));
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

    QObject::connect(preferencesAction, &QAction::triggered, window,
                      [window, appSettings, editorTabs]() {
                          showSettingsDialog(window, appSettings, editorTabs);
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
