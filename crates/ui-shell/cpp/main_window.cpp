#include "main_window.h"

#include "ai_chat_panel.h"
#include "ai_providers_page.h"
#include "appearance_page.h"
#include "code_editor.h"
#include "e2e_mark.h"
#include "editor_tabs.h"
#include "find_bar.h"
#include "hex_viewer.h"
#include "icon_cache.h"
#include "keymap_page.h"
#include "language_servers_page.h"
#include "languages_page.h"
#include "plugins_page.h"
#include "syntax_colors_page.h"
#include "search_everywhere_dialog.h"
#include "problems_panel.h"
#include "icon_decoration_proxy.h"
#include "project_tree_dock.h"
#include "recent_projects_menu.h"
#include "refactor_preview_dialog.h"
#include "search_results_panel.h"
#include "splash_screen.h"
#include "syntax_highlighter.h"
#include "terminal_widget.h"
#include "theme.h"
#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include "DockManager.h"
#include "DockWidget.h"

#include <QApplication>
#include <QByteArray>
#include <QCheckBox>
#include <QCloseEvent>
#include <QElapsedTimer>
#include <QKeyEvent>
#include <QSet>
#include <QTextBlock>
#include <QTimer>
#include <QToolButton>
#include <QToolTip>
#include <QVector>
#include <QTreeWidget>
#include <QColor>
#include <QColorDialog>
#include <QComboBox>
#include <QDialog>
#include <QDialogButtonBox>
#include <QFileDialog>
#include <QFileInfo>
#include <QFont>
#include <QFormLayout>
#include <QHash>
#include <algorithm>
#include <cstdint>
#include <functional>
#include <QHBoxLayout>
#include <QHash>
#include <QInputDialog>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QProgressBar>
#include <QStatusBar>
#include <QTextCursor>
#include <QToolButton>
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
#include <QSet>
#include <QSpinBox>
#include <QStackedWidget>
#include <QStringList>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QSplitter>
#include <QTabBar>
#include <QTabWidget>
#include <QtGui/QTextDocument>
#include <QTreeView>
#include <QTreeWidget>
#include <QTreeWidgetItem>
#include <QVariant>
#include <QVBoxLayout>
#include <QWidget>

namespace ui_shell {

namespace {

// Shared by ClassViewPanel, QuickOpenDialog and FindUsagesPanel (Tasks D/I/J)
// so the "class"/"method"/... label text is spelled once.
QString symbolKindLabel(FfiSymbolKind kind)
{
    switch (kind) {
    case FfiSymbolKind::Class:
        return QStringLiteral("class");
    case FfiSymbolKind::Struct:
        return QStringLiteral("struct");
    case FfiSymbolKind::Enum:
        return QStringLiteral("enum");
    case FfiSymbolKind::Interface:
        return QStringLiteral("interface");
    case FfiSymbolKind::Method:
        return QStringLiteral("method");
    case FfiSymbolKind::Function:
        return QStringLiteral("function");
    case FfiSymbolKind::Field:
    default:
        return QStringLiteral("field");
    }
}

// Class View dock panel: a QTreeWidget with two data-source tiers, toggled
// by `modeCombo_` (Task I extends Task D's original per-file-only panel —
// "same widget/model, second data-source impl" per the plan doc, not a
// second panel). Humble view per CLAUDE.md's hard rule — outline/symbol
// extraction is entirely `syntax_core`/`index_core`'s job; this only builds
// tree items and forwards double-clicks to a caret jump. Same dock-panel
// shape FindInFilesPanel (Task H) established above.
//
// Per-file tier (Task D): populated from `DocumentManager::tabOutline()`
// for whichever tab is current, reconstructing nesting from each
// `FfiSymbolNode`'s `depth` (per that struct's own doc comment).
//
// Project tier (Task I): populated by streamed `SearchModel::projectSymbolFound`
// signals off `index_core::TextIndex::find_definitions("")` (an empty query
// matches every definition — see the bridge's doc comment), grouped by file
// then by container. Ephemeral view state, like folding's collapsed-state
// (plan doc, Task I) — the toggle's position isn't persisted, and switching
// to Project mode doesn't track tab-switch/save events the way the per-file
// tier does (`refresh()` becomes a no-op in project mode); switching back
// re-syncs it.
class ClassViewPanel : public QWidget
{
public:
    // Task J: `onFindUsagesRequested` is called with a symbol's exact name
    // when the user picks "Find Usages" from a leaf item's context menu —
    // the panel doesn't know or care what happens with that name (main_window
    // wires it to FindUsagesPanel), keeping this class's only job "show the
    // outline, forward intents".
    ClassViewPanel(DocumentManager *docManager, SearchModel *searchModel, EditorTabs *editorTabs,
                    std::function<void(const QString &)> onFindUsagesRequested, QWidget *parent)
      : QWidget(parent)
      , docManager_(docManager)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
      , onFindUsagesRequested_(std::move(onFindUsagesRequested))
    {
        modeCombo_ = new QComboBox(this);
        modeCombo_->addItem(tr("Current File"));
        modeCombo_->addItem(tr("Project"));

        // PhpStorm-style toggle: off (default) shows definition order,
        // on sorts each tree level alphabetically by its item text — the
        // symbol name is the leading token so text sort already reads as
        // name sort, no per-item comparator needed.
        sortButton_ = new QToolButton(this);
        sortButton_->setText(tr("A→Z"));
        sortButton_->setToolTip(tr("Sort Alphabetically"));
        sortButton_->setCheckable(true);
        connect(sortButton_, &QToolButton::toggled, this, [this](bool on) {
            tree_->setSortingEnabled(on);
            if (on) {
                tree_->sortByColumn(0, Qt::AscendingOrder);
            }
        });

        tree_ = new QTreeWidget(this);
        tree_->setHeaderHidden(true);
        tree_->setContextMenuPolicy(Qt::CustomContextMenu);
        auto *topLayout = new QHBoxLayout();
        topLayout->addWidget(modeCombo_, 1);
        topLayout->addWidget(sortButton_);
        auto *layout = new QVBoxLayout(this);
        layout->addLayout(topLayout);
        layout->addWidget(tree_);

        connect(tree_, &QTreeWidget::itemDoubleClicked, this, &ClassViewPanel::onItemDoubleClicked);
        connect(tree_, &QTreeWidget::customContextMenuRequested, this,
                &ClassViewPanel::onContextMenuRequested);
        connect(modeCombo_, &QComboBox::currentIndexChanged, this, [this](int index) {
            projectMode_ = (index == 1);
            if (projectMode_) {
                refreshProject();
            } else {
                refresh(editorTabs_->currentTabId());
            }
        });

        connect(searchModel_, &SearchModel::projectSymbolFound, this, &ClassViewPanel::addProjectSymbol);
        connect(searchModel_, &SearchModel::projectSymbolsFinished, this,
                [this]() { tree_->expandAll(); });
        connect(searchModel_, &SearchModel::projectSymbolsFailed, this,
                [this](const QString &message) {
                    tree_->clear();
                    new QTreeWidgetItem(tree_, QStringList { tr("Project symbols unavailable: %1").arg(message) });
                });
    }

    // Repopulate the tree from `tabId`'s current outline — called on tab
    // open, on tab switch, and whenever a tab becomes clean (a proxy for
    // "just saved"; see buildCentralWidget's wiring comment for why). A
    // no-op while the Project tier is active (see class doc comment).
    // `tabId == 0` (no tab open) just clears the tree.
    void refresh(quint64 tabId)
    {
        if (projectMode_) {
            return;
        }
        tree_->clear();
        if (tabId == 0) {
            return;
        }
        const rust::Vec<FfiSymbolNode> symbols = docManager_->tabOutline(tabId);

        // `depth` reconstructs the tree from this pre-order-flattened list
        // (see FfiSymbolNode's doc comment): `parents[d]` is the open
        // QTreeWidgetItem at depth d-1 that the next depth-d item attaches
        // under, or nullptr for a root (attaches to the QTreeWidget itself).
        QVector<QTreeWidgetItem *> parents;
        for (const auto &symbol : symbols) {
            const int depth = static_cast<int>(symbol.depth);
            parents.resize(depth + 1);
            auto *item = new QTreeWidgetItem();
            item->setText(0, symbol.name + QStringLiteral(" (") + symbolKindLabel(symbol.kind)
                                + QStringLiteral(")"));
            item->setData(0, Qt::UserRole, static_cast<quint64>(symbol.name_start));
            // Task J: the bare name, for "Find Usages" — kept separate from
            // the display text above, which has the "(kind)" suffix baked in.
            item->setData(0, Qt::UserRole + 2, symbol.name);
            if (depth == 0) {
                tree_->addTopLevelItem(item);
            } else {
                parents[depth - 1]->addChild(item);
            }
            parents[depth] = item;
        }
        tree_->expandAll();
    }

private:
    // Task I: (re)issue a project-wide query. Results stream back via
    // `addProjectSymbol` below; `fileItems_`/`containerItems_` are rebuilt
    // from scratch each time, keyed off this call's own tree.
    void refreshProject()
    {
        tree_->clear();
        fileItems_.clear();
        containerItems_.clear();
        searchModel_->projectSymbols();
    }

    // One project-wide symbol definition, grouped under a per-file top-level
    // item and (when it has one) a per-container item nested under that —
    // `containerItems_` is keyed by `path + container` since two files can
    // each have their own same-named class.
    void addProjectSymbol(const FfiSymbolMatch &row)
    {
        QTreeWidgetItem *fileItem = fileItems_.value(row.path, nullptr);
        if (!fileItem) {
            fileItem = new QTreeWidgetItem(tree_, QStringList { QFileInfo(row.path).fileName() });
            fileItems_.insert(row.path, fileItem);
        }
        QTreeWidgetItem *parent = fileItem;
        if (!row.container.isEmpty()) {
            const QString key = row.path + QChar(0x1f) + row.container;
            QTreeWidgetItem *containerItem = containerItems_.value(key, nullptr);
            if (!containerItem) {
                containerItem = new QTreeWidgetItem(fileItem, QStringList { row.container });
                containerItems_.insert(key, containerItem);
            }
            parent = containerItem;
        }
        auto *item = new QTreeWidgetItem(
          parent,
          QStringList { row.name + QStringLiteral(" (") + symbolKindLabel(row.kind)
                        + QStringLiteral(")") });
        item->setData(0, Qt::UserRole, row.path);
        item->setData(0, Qt::UserRole + 1, row.line);
        // Task J: bare name for "Find Usages" — group nodes (file/container,
        // built above with QStringList-only constructors) never get this
        // role set, so the context menu naturally has nothing to offer them.
        item->setData(0, Qt::UserRole + 2, row.name);
        item->setData(0, Qt::UserRole + 3, row.column);
    }

    void onItemDoubleClicked(QTreeWidgetItem *item)
    {
        if (!item) {
            return;
        }
        if (projectMode_) {
            // File/container group nodes carry no data — only leaf symbol
            // items do (see addProjectSymbol above).
            const QVariant pathData = item->data(0, Qt::UserRole);
            if (!pathData.isValid()) {
                return;
            }
            editorTabs_->openFileAtLine(pathData.toString(),
                                         item->data(0, Qt::UserRole + 1).toInt(),
                                         item->data(0, Qt::UserRole + 3).toInt());
        } else {
            editorTabs_->jumpToByteOffset(item->data(0, Qt::UserRole).toULongLong());
        }
    }

    // Task J: "Find Usages" on a leaf symbol item (per-file or project
    // tier alike — both stash the bare name at UserRole+2).
    void onContextMenuRequested(const QPoint &pos)
    {
        QTreeWidgetItem *item = tree_->itemAt(pos);
        if (!item) {
            return;
        }
        const QVariant nameData = item->data(0, Qt::UserRole + 2);
        if (!nameData.isValid() || !onFindUsagesRequested_) {
            return;
        }
        QMenu menu(tree_);
        QAction *findUsagesAction = menu.addAction(tr("Find Usages"));
        QAction *chosen = menu.exec(tree_->viewport()->mapToGlobal(pos));
        if (chosen == findUsagesAction) {
            onFindUsagesRequested_(nameData.toString());
        }
    }

    DocumentManager *docManager_;
    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    std::function<void(const QString &)> onFindUsagesRequested_;
    QTreeWidget *tree_ = nullptr;
    QComboBox *modeCombo_ = nullptr;
    QToolButton *sortButton_ = nullptr;
    bool projectMode_ = false;
    QHash<QString, QTreeWidgetItem *> fileItems_;
    QHash<QString, QTreeWidgetItem *> containerItems_;
};

// Find Usages results dock (Task J): reuses FindInFilesPanel's dockable
// "list of locations, double-click to jump" shape rather than inventing a
// new one — find-usages results are the same kind of thing (a list of
// file:line locations), just fed by `SearchModel::findUsages` instead of
// `search`, and triggered from Class View's context menu instead of typed
// free text, so there's no query box here.
class FindUsagesPanel : public QWidget
{
public:
    FindUsagesPanel(SearchModel *searchModel, EditorTabs *editorTabs, QWidget *parent)
      : QWidget(parent)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
    {
        statusLabel_ = new QLabel(this);
        resultsList_ = new QListWidget(this);

        auto *layout = new QVBoxLayout(this);
        layout->addWidget(statusLabel_);
        layout->addWidget(resultsList_, 1);

        connect(resultsList_,
                &QListWidget::itemDoubleClicked,
                this,
                &FindUsagesPanel::openSelected);
        connect(searchModel_, &SearchModel::usagesFound, this, &FindUsagesPanel::addUsage);
        connect(searchModel_, &SearchModel::usagesFinished, this, [this]() {
            statusLabel_->setText(tr("%1 result(s).").arg(resultsList_->count()));
        });
        connect(searchModel_, &SearchModel::usagesFailed, this, [this](const QString &message) {
            statusLabel_->setText(tr("Search failed: %1").arg(message));
        });
    }

    // Called from ClassViewPanel's "Find Usages" context-menu action and
    // from Navigate > Find Usages (via main_window's wiring) with the
    // symbol's exact name.
    void findUsages(const QString &name)
    {
        beginQuery(tr("Searching usages of \"%1\"...").arg(name));
        searchModel_->findUsages(name);
    }

    // N3: Navigate > Go to Implementation / Go to Interface. Both are
    // lists of file:line locations, which is exactly what this dock
    // already renders, so they stream in on the same signals rather than
    // getting a near-identical panel of their own.
    void findImplementations(const QString &name)
    {
        beginQuery(tr("Searching implementations of \"%1\"...").arg(name));
        searchModel_->findImplementations(name);
    }

    void findSupertypes(const QString &name)
    {
        beginQuery(tr("Searching supertypes of \"%1\"...").arg(name));
        searchModel_->findSupertypes(name);
    }

private:
    // `index_core::TextIndex::find_usages` already returns results sorted
    // by (path, line) — see `SearchModel::find_usages` — so consecutive
    // rows here already read as grouped by file with no extra tree
    // structure needed.
    void beginQuery(const QString &status)
    {
        resultsList_->clear();
        statusLabel_->setText(status);
    }

    void addUsage(const FfiSymbolMatch &row)
    {
        const QString kindLabel = row.is_definition ? tr("def") : tr("ref");
        const QString label = row.container.isEmpty()
          ? tr("%1:%2 [%3]").arg(QFileInfo(row.path).fileName()).arg(row.line).arg(kindLabel)
          : tr("%1:%2 [%3] in %4")
              .arg(QFileInfo(row.path).fileName())
              .arg(row.line)
              .arg(kindLabel, row.container);
        auto *item = new QListWidgetItem(label, resultsList_);
        item->setData(Qt::UserRole, row.path);
        item->setData(Qt::UserRole + 1, row.line);
        item->setData(Qt::UserRole + 2, row.column);
    }

    void openSelected(QListWidgetItem *item)
    {
        if (!item) {
            return;
        }
        editorTabs_->openFileAtLine(item->data(Qt::UserRole).toString(),
                                     item->data(Qt::UserRole + 1).toInt(),
                                     item->data(Qt::UserRole + 2).toInt());
    }

    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    QLabel *statusLabel_ = nullptr;
    QListWidget *resultsList_ = nullptr;
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
    void setDocumentManager(DocumentManager *docManager) { docManager_ = docManager; }
    // Opens Search Everywhere. Set once the popup exists; until then the
    // double-Shift gesture is simply inert.
    void setSearchEverywhereTrigger(std::function<void()> trigger)
    {
        searchEverywhere_ = std::move(trigger);
    }

protected:
    // JetBrains' double-Shift gesture: two Shift presses inside
    // kDoubleShiftMs open Search Everywhere. Handled here rather than as a
    // QShortcut because a bare modifier is not a key sequence Qt can bind.
    void keyPressEvent(QKeyEvent *event) override
    {
        static constexpr int kDoubleShiftMs = 300;
        // A Shift held together with another modifier is part of a
        // shortcut, not a gesture: Ctrl+Shift+N followed within the window by
        // any capital letter would otherwise open the popup a second time.
        const bool bareShift = (event->modifiers() & ~Qt::ShiftModifier) == Qt::NoModifier;
        if (event->key() == Qt::Key_Shift && !event->isAutoRepeat() && bareShift) {
            if (lastShift_.isValid() && lastShift_.elapsed() < kDoubleShiftMs) {
                lastShift_.invalidate();
                if (searchEverywhere_) {
                    searchEverywhere_();
                }
                return;
            }
            lastShift_.start();
        } else if (!event->text().isEmpty()) {
            // Any real keystroke between the two presses means the user was
            // typing, not gesturing.
            lastShift_.invalidate();
        }
        QMainWindow::keyPressEvent(event);
    }

    void closeEvent(QCloseEvent *event) override
    {
        if (editorTabs_ && !editorTabs_->confirmCloseAllTabs()) {
            event->ignore();
            return;
        }
        if (appSettings_) {
            // normalGeometry(), not geometry(): a maximised or minimised
            // window reports its current screen rect (0x0 while minimised),
            // and restoring that is not what the user last sized the window
            // to. Rust drops a rect it cannot use.
            const QRect g = normalGeometry();
            appSettings_->saveWindowGeometry(g.x(), g.y(), static_cast<quint32>(qMax(0, g.width())),
                                              static_cast<quint32>(qMax(0, g.height())));
            if (dockManager_) {
                // D4: window_state is a plain Rust String (must be valid
                // UTF-8); ADS's saveState() returns raw QByteArray, so
                // base64 round-trips it through that constraint.
                const QString state =
                  QString::fromLatin1(dockManager_->saveState().toBase64());
                appSettings_->saveWindowState(state);
            }
            if (editorTabs_) {
                // The editor split layout is the view's own JSON (ADS knows
                // nothing about the splitter tree inside the editor dock).
                appSettings_->saveEditorLayout(editorTabs_->saveLayout());
            }
        }
        if (docManager_) {
            // Takes the discovery file with it — one left behind points the
            // next agent that reads it at a port nothing answers on.
            docManager_->shutdownMcpServer();
        }
        QMainWindow::closeEvent(event);
    }

private:
    std::function<void()> searchEverywhere_;
    QElapsedTimer lastShift_;
    EditorTabs *editorTabs_ = nullptr;
    AppSettings *appSettings_ = nullptr;
    ads::CDockManager *dockManager_ = nullptr;
    DocumentManager *docManager_ = nullptr;
};

// RF11: every refactoring gesture, in one place.
//
// It contains no refactoring logic and no rules about when a refactoring is
// safe. What it does is ask, then paint what came back: whether a preview is
// required is `lsp_core::EditPlan::touches_other_files`, whether a rename may
// go ahead at all is `lsp_core::rename`'s and `index_core`'s to say, and
// which sites of a name-based rename start ticked is decided before the
// dialog is built. Every branch below is on a flag or a signal, never on a
// judgement made here.
// One line of an edit, for the preview. A multi-line insertion is shown by
// its first line: the dialog says what is changing and where, not what the
// new text is in full. Free rather than a member of RefactorController: the
// AI panel's Apply runs the same preview over the same FfiTextEdit rows
// (ADR-0021 §5), and a second copy of this would drift.
QString previewText(const QString &newText)
{
    const QString first = newText.split(QLatin1Char('\n')).value(0).trimmed();
    if (first.isEmpty()) {
        return QObject::tr("(removed)");
    }
    return first.size() > 80 ? first.left(77) + QStringLiteral("...") : first;
}

class RefactorController : public QObject
{
public:
    RefactorController(LanguageService *languageService, SearchModel *searchModel,
                        EditorTabs *editorTabs, QMainWindow *window)
      : QObject(window)
      , languageService_(languageService)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
      , window_(window)
    {
        connect(languageService_, &LanguageService::renamePrepared, this,
                &RefactorController::askForNewName);
        connect(languageService_, &LanguageService::renameRejected, this,
                [this](const QString &reason) {
                    QMessageBox::information(window_, tr("Rename"), reason);
                });
        connect(languageService_, &LanguageService::refactorReady, this,
                &RefactorController::onRefactorReady);
        connect(languageService_, &LanguageService::refactorFallback, this,
                &RefactorController::askIndexToRename);
        connect(languageService_, &LanguageService::refactorFailed, this,
                [this](const QString &message) {
                    // The user's whole gesture failed, so it is said out
                    // loud. The status bar is for outcomes they can already
                    // see in the editor.
                    QMessageBox::warning(window_, tr("Refactoring failed"), message);
                });
        connect(languageService_, &LanguageService::codeActionsReady, this,
                &RefactorController::onCodeActionsReady);

        connect(searchModel_, &SearchModel::indexRenameReady, this,
                &RefactorController::onIndexRenameReady);
        connect(searchModel_, &SearchModel::indexRenameFailed, this,
                &RefactorController::onRenameRefused);
        // The count the user cares about is every file that changed, not
        // just the ones written to disk: a refactoring confined to open
        // editors writes nothing, and reporting "0 file(s)" for it reads as
        // a failure.
        connect(searchModel_, &SearchModel::refactorFilesFinished, this,
                [this](quint32 files, quint32 skipped) {
                    const int changed = static_cast<int>(files) + bufferFiles_;
                    bufferFiles_ = 0;
                    if (skipped > 0) {
                        report(tr("Refactored %n file(s); %1 could not be changed.", "", changed)
                                 .arg(skipped));
                        return;
                    }
                    if (files == 0 && changed > 0) {
                        report(tr("Refactored %n open file(s) — save to write the changes.", "",
                                  changed));
                        return;
                    }
                    report(tr("Refactored %n file(s).", "", changed));
                });
        connect(searchModel_, &SearchModel::refactorFilesFailed, this,
                [this](const QString &message) { report(tr("Refactoring failed: %1").arg(message)); });
    }

    // Shift+F6. Asks the server whether the symbol can be renamed at all;
    // a server that does not implement the question answers "go ahead",
    // which is `lsp_core::rename::prepare_outcome`'s rule.
    void renameSymbol()
    {
        if (editorTabs_->currentPath().isEmpty()) {
            return;
        }
        pendingWord_ = editorTabs_->wordUnderCursor();
        languageService_->prepareRename(editorTabs_->currentPath(),
                                         caret().first,
                                         caret().second);
    }

    // Ctrl+Alt+M and its siblings: ask for a kind family, then offer
    // whatever the server actually has.
    void extract(const QString &kind, const QString &nothingFound)
    {
        if (editorTabs_->currentPath().isEmpty()) {
            return;
        }
        nothingFound_ = nothingFound;
        const auto range = editorTabs_->selectionRange();
        languageService_->codeActionsAt(editorTabs_->currentPath(),
                                         range.first.first,
                                         range.first.second,
                                         range.second.first,
                                         range.second.second,
                                         kind);
    }

private:
    QPair<quint32, quint32> caret() const
    {
        return editorTabs_->lspPositionAt(editorTabs_->caretPosition());
    }

    void askForNewName(const QString &placeholder)
    {
        const QString suggestion = placeholder.isEmpty() ? pendingWord_ : placeholder;
        bool accepted = false;
        const QString newName = QInputDialog::getText(window_,
                                                       tr("Rename"),
                                                       tr("New name:"),
                                                       QLineEdit::Normal,
                                                       suggestion,
                                                       &accepted);
        if (!accepted || newName.isEmpty() || newName == suggestion) {
            return;
        }
        pendingName_ = newName;
        revision_ = editorTabs_->documentRevision();
        languageService_->renameAt(editorTabs_->currentPath(),
                                    caret().first,
                                    caret().second,
                                    newName,
                                    revision_);
    }

    // ADR-0016's fallback, reached only when `lsp_core` said no server
    // answered — never from a condition evaluated here.
    void askIndexToRename()
    {
        if (pendingName_.isEmpty()) {
            return;
        }
        searchModel_->planIndexRename(editorTabs_->currentPath(),
                                       editorTabs_->currentContent(),
                                       editorTabs_->byteOffsetAt(editorTabs_->caretPosition()),
                                       pendingName_,
                                       editorTabs_->hasUnsavedChanges());
    }

    // Why a name-based rename will not run. Three cases are a sentence; the
    // unsaved-files case is a dead end the user can get out of, so it offers
    // the way out instead of describing it.
    //
    // These used to go to the status bar, where a message the user did not
    // happen to be looking at made a refused rename indistinguishable from a
    // broken one.
    void onRenameRefused(FfiRenameRefusal reason, const QString &message)
    {
        if (reason != FfiRenameRefusal::UnsavedChanges) {
            QMessageBox::information(window_, tr("Rename"), message);
            return;
        }

        const auto answer = QMessageBox::question(
          window_,
          tr("Rename"),
          tr("%1\n\nSave all files and rename now?").arg(message),
          QMessageBox::Save | QMessageBox::Cancel,
          QMessageBox::Save);
        if (answer != QMessageBox::Save) {
            return;
        }
        if (!editorTabs_->saveAllModified()) {
            // saveTab already said which file could not be written.
            return;
        }
        askIndexToRename();
    }

    void onRefactorReady(const FfiRefactorSummary &summary)
    {
        // A refactoring confined to the file the user is looking at applies
        // straight away and is undone with Ctrl+Z. Anything wider is shown
        // first — and which of the two this is was decided in Rust.
        if (!summary.touches_other_files) {
            applyPending();
            return;
        }

        QList<RefactorPreviewDialog::Row> rows;
        for (const FfiTextEdit &edit : languageService_->pendingEdits()) {
            rows.append({edit.path, static_cast<int>(edit.start_line),
                          previewText(edit.new_text), true, true});
        }
        RefactorPreviewDialog dialog(summary.title,
                                      tr("%n change(s) across %1 file(s). Changes to files that "
                                         "are not open are written to disk and cannot be undone.",
                                         "", static_cast<int>(summary.edit_count))
                                        .arg(summary.document_count),
                                      rows,
                                      window_);
        if (dialog.exec() != QDialog::Accepted) {
            languageService_->cancelRefactor();
            return;
        }
        for (const QString &path : dialog.excludedPaths()) {
            languageService_->excludeFromRefactor(path);
        }
        applyPending();
    }

    void applyPending()
    {
        const ::rust::Vec<FfiTextEdit> edits = languageService_->takePendingEdits(revision_);
        if (edits.empty()) {
            report(tr("The file changed while the refactoring was being prepared; nothing was "
                      "applied."));
            e2eMark("{\"ev\":\"workspace_edit_refused\",\"reason\":\"stale\"}");
            return;
        }
        bufferFiles_ = countBufferFiles(edits);
        e2eMark(QStringLiteral("{\"ev\":\"workspace_edit_applied\",\"documents\":%1}")
                  .arg(countFiles(edits)));
        editorTabs_->applyBufferEdits(edits);
        // Files nobody has open are rewritten and re-indexed by the index
        // worker; it ignores the buffer edits in the same vector.
        searchModel_->applyFileEdits(edits);
    }

    void onIndexRenameReady(const QString &name, bool ambiguous)
    {
        QList<RefactorPreviewDialog::Row> rows;
        for (const FfiRenameSite &site : searchModel_->indexRenameSites()) {
            rows.append({site.path, static_cast<int>(site.line) - 1,
                          site.is_definition ? tr("declaration of %1").arg(name)
                                             : tr("use of %1").arg(name),
                          site.resolved, site.checked});
        }
        if (rows.isEmpty()) {
            report(tr("No occurrences of \"%1\" were found.").arg(name));
            return;
        }

        // The honesty this dialog exists for: with no language server this
        // is name matching, and the user has to be told so before it writes.
        const QString explanation =
          ambiguous
            ? tr("No language server answered, so these sites were found by name. More than one "
                 "symbol in this project is called \"%1\", so the uncertain sites are unticked. "
                 "Files that are not open are written to disk and cannot be undone.")
                .arg(name)
            : tr("No language server answered, so these sites were found by name. Files that are "
                 "not open are written to disk and cannot be undone.");

        RefactorPreviewDialog dialog(tr("Rename %1 to %2").arg(name, pendingName_),
                                      explanation,
                                      rows,
                                      window_);
        if (dialog.exec() != QDialog::Accepted) {
            return;
        }
        for (const QString &path : dialog.excludedPaths()) {
            searchModel_->excludeFromIndexRename(path);
        }
        // Files the user has open are spliced in their buffers, so the
        // rename is undoable where it is visible and the editor does not
        // prompt about a change it made itself. Taking them also removes
        // them from the plan, so the disk pass below cannot apply them
        // twice.
        bufferFiles_ = 0;
        for (const QString &path : editorTabs_->openPaths()) {
            const ::rust::Vec<FfiTextEdit> edits =
              searchModel_->takeIndexRenameBufferEdits(path);
            if (!edits.empty()) {
                ++bufferFiles_;
                editorTabs_->applyBufferEdits(edits);
            }
        }
        e2eMark(QStringLiteral("{\"ev\":\"workspace_edit_applied\",\"documents\":%1}")
                  .arg(countPaths(rows) - dialog.excludedPaths().size()));
        searchModel_->applyIndexRename();
    }

    void onCodeActionsReady()
    {
        const ::rust::Vec<FfiCodeAction> actions = languageService_->codeActions();
        if (actions.empty()) {
            report(nothingFound_);
            return;
        }
        revision_ = editorTabs_->documentRevision();
        if (actions.size() == 1 && QString(actions[0].disabled_reason).isEmpty()) {
            languageService_->applyCodeAction(0, revision_);
            return;
        }

        // Several offers, or one the server marked unavailable: show them
        // rather than choosing. A disabled row is listed greyed with its
        // reason, because a menu that changes shape with the caret reads as
        // a bug.
        QMenu menu(window_);
        for (std::size_t i = 0; i < actions.size(); ++i) {
            const QString reason = actions[i].disabled_reason;
            QAction *entry = menu.addAction(reason.isEmpty()
                                              ? QString(actions[i].title)
                                              : tr("%1 — %2").arg(QString(actions[i].title), reason));
            entry->setEnabled(reason.isEmpty());
            const quint32 index = static_cast<quint32>(i);
            connect(entry, &QAction::triggered, this,
                    [this, index]() { languageService_->applyCodeAction(index, revision_); });
        }
        menu.exec(QCursor::pos());
    }

    // How many distinct files a batch of edits changes in their buffers.
    static int countFiles(const ::rust::Vec<FfiTextEdit> &edits)
    {
        QSet<QString> paths;
        for (const FfiTextEdit &edit : edits) {
            paths.insert(edit.path);
        }
        return paths.size();
    }

    static int countPaths(const QList<RefactorPreviewDialog::Row> &rows)
    {
        QSet<QString> paths;
        for (const RefactorPreviewDialog::Row &row : rows) {
            paths.insert(row.path);
        }
        return paths.size();
    }

    static int countBufferFiles(const ::rust::Vec<FfiTextEdit> &edits)
    {
        QSet<QString> paths;
        for (const FfiTextEdit &edit : edits) {
            if (edit.in_buffer) {
                paths.insert(edit.path);
            }
        }
        return paths.size();
    }

    void report(const QString &message) { window_->statusBar()->showMessage(message, 6000); }

    LanguageService *languageService_;
    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    QMainWindow *window_;
    QString pendingWord_;
    QString pendingName_;
    QString nothingFound_;
    int revision_ = 0;
    // Files changed in their buffers by the refactoring being applied, so
    // the outcome can be reported as a whole rather than as the disk half.
    int bufferFiles_ = 0;
};

// Go to Declaration (N2/N8/L4): turns a resolution — from the language
// server or from the index — into either a jump or a chooser.
//
// Two rules this class does *not* contain: which candidate is best
// (`index_core::resolve_declaration` and the server both answer ranked, and
// the first candidate to arrive is the winner), and who answers at all —
// `lsp_core::definition_outcome` decides that, and reaches here as either
// the definitionFound/definitionFinished pair or definitionFallback.
// Presentation is all that is decided here: one candidate jumps straight
// there, several offer the list (resolution may legitimately be ambiguous —
// name-based per ADR-0008, or genuinely several targets per LSP), none
// reports why nothing happened.
class DeclarationNavigator : public QObject
{
public:
    DeclarationNavigator(LanguageService *languageService, SearchModel *searchModel,
                          EditorTabs *editorTabs, QMainWindow *window)
      : QObject(window)
      , languageService_(languageService)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
      , window_(window)
    {
        connect(searchModel_, &SearchModel::declarationFound, this,
                [this](const FfiSymbolMatch &row) {
                    candidates_.append(
                      Candidate{row.path, row.line, row.column,
                                 row.has_kind ? symbolKindLabel(row.kind) : QString(),
                                 row.container});
                });
        connect(searchModel_, &SearchModel::declarationFinished, this,
                &DeclarationNavigator::finish);
        connect(searchModel_, &SearchModel::declarationFailed, this,
                [this](const QString &message) {
                    candidates_.clear();
                    report(tr("Go to Declaration failed: %1").arg(message));
                });

        // L4: the language server's answer, when it had one. Targets carry
        // no kind or container — a server answers with places, not with the
        // index's symbol metadata.
        connect(languageService_, &LanguageService::definitionFound, this,
                [this](const FfiDefinition &target) {
                    candidates_.append(
                      Candidate{target.path, target.line, target.column, QString(), QString()});
                });
        connect(languageService_, &LanguageService::definitionFinished, this, [this]() {
            // Project tier: several targets are several real answers, so
            // they are offered rather than silently reduced to the first.
            finish(FfiResolutionTier::Project, QString());
        });
        connect(languageService_, &LanguageService::definitionFallback, this,
                &DeclarationNavigator::askIndex);
    }

    // Entry point for both the Ctrl+Click gesture and the menu action.
    // `documentPosition` is a UTF-16 document position; the byte offset
    // the index speaks is derived by EditorTabs, which owns the buffer.
    void resolveAt(int documentPosition)
    {
        const QString path = editorTabs_->currentPath();
        if (path.isEmpty()) {
            return;
        }
        candidates_.clear();
        position_ = documentPosition;
        const QPair<quint32, quint32> at = editorTabs_->lspPositionAt(documentPosition);
        languageService_->resolveDefinition(path, at.first, at.second);
    }

    // ADR-0016's fallback: ADR-0011's name-based index answers whenever the
    // server did not. Never called from a condition evaluated here — it is
    // wired to definitionFallback, which is `lsp_core`'s verdict.
    void askIndex()
    {
        const QString path = editorTabs_->currentPath();
        if (path.isEmpty()) {
            return;
        }
        candidates_.clear();
        searchModel_->resolveDeclaration(path, editorTabs_->currentContent(),
                                          editorTabs_->byteOffsetAt(position_));
    }

private:
    struct Candidate
    {
        QString path;
        quint32 line;
        quint32 column;
        QString kind;
        QString container;
    };

    void finish(FfiResolutionTier tier, const QString &name)
    {
        const QList<Candidate> candidates = std::move(candidates_);
        candidates_.clear();
        if (candidates.isEmpty()) {
            report(name.isEmpty() ? tr("No identifier under the caret.")
                                  : tr("No declaration found for \"%1\".").arg(name));
            return;
        }
        // A local-file result is ranked, not merely listed: the first
        // candidate is the innermost binding that shadows the caret, so
        // offering a chooser would contradict the ranking that made it
        // first. Only project-tier ambiguity is genuine — same name,
        // unrelated symbols, nothing to prefer between them.
        if (candidates.size() == 1 || tier == FfiResolutionTier::LocalFile) {
            jumpTo(candidates.first());
            return;
        }
        chooseAmong(candidates, name);
    }

    void jumpTo(const Candidate &candidate)
    {
        editorTabs_->openFileAtLine(candidate.path, static_cast<int>(candidate.line),
                                     static_cast<int>(candidate.column));
    }

    // Several same-named declarations: offer them at the caret rather than
    // picking one. A popup menu (not a dialog) keeps the gesture as light
    // as the click that started it.
    void chooseAmong(const QList<Candidate> &candidates, const QString &name)
    {
        QMenu menu(window_);
        menu.setTitle(tr("Declarations of \"%1\"").arg(name));
        for (const Candidate &candidate : candidates) {
            const QString file = QFileInfo(candidate.path).fileName();
            QString label = tr("%1:%2").arg(file).arg(candidate.line);
            if (!candidate.container.isEmpty()) {
                label = tr("%1 in %2").arg(label, candidate.container);
            }
            if (!candidate.kind.isEmpty()) {
                label = tr("%1 (%2)").arg(label, candidate.kind);
            }
            QAction *action = menu.addAction(label);
            connect(action, &QAction::triggered, this,
                    [this, candidate]() { jumpTo(candidate); });
        }
        menu.exec(QCursor::pos());
    }

    void report(const QString &message)
    {
        window_->statusBar()->showMessage(message, 4000);
    }

    static QString symbolKindLabel(FfiSymbolKind kind)
    {
        switch (kind) {
        case FfiSymbolKind::Class:
            return tr("class");
        case FfiSymbolKind::Struct:
            return tr("struct");
        case FfiSymbolKind::Enum:
            return tr("enum");
        case FfiSymbolKind::Interface:
            return tr("interface");
        case FfiSymbolKind::Method:
            return tr("method");
        case FfiSymbolKind::Function:
            return tr("function");
        case FfiSymbolKind::Field:
            return tr("field");
        }
        return QString();
    }

    LanguageService *languageService_;
    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    QMainWindow *window_;
    QList<Candidate> candidates_;
    // The document position of the gesture being resolved, kept so the
    // index fallback can re-ask about the same spot in its own units.
    int position_ = 0;
};


// Settings dialog (S1: category list + stacked detail pane; S2: font +
// editor colors on the Editor page). Every control applies live as it's
// changed (theme via applyTheme(), font/colors via `editorTabs`) so
// the effect is visible immediately; OK persists that already-applied state
// through `appSettings`, Cancel restores exactly what was active when the
// dialog opened. Modal and blocking, so every lambda below capturing
// `&dialog` only ever runs while `dialog` is still alive on this stack frame.

void showSettingsDialog(QWidget *parent, AppSettings *appSettings, EditorTabs *editorTabs,
                        KeymapEditor *keymapEditor, const QHash<QString, QAction *> &actions,
                        DocumentManager *docManager, const std::shared_ptr<QString> &mcpStatus,
                        SyntaxColorEditor *syntaxColorEditor, LanguageCatalog *languageCatalog,
                        LanguageServerEditor *languageServerEditor,
                        LanguageService *languageService,
                        AiProviderEditor *aiProviderEditor, AiChat *aiChat,
                        PluginCatalog *pluginCatalog,
                        const UiFontTargets &uiFontTargets)
{
    const FfiEditorFont originalFont = appSettings->editorFont();
    const FfiEditorColors originalColors = appSettings->editorColors();

    QDialog dialog(parent);
    dialog.setWindowTitle(QObject::tr("Settings"));
    // The pages' own minimums add up to roughly 740x510, which is enough to
    // lay a page out but not enough to read one: the Languages tree needs
    // room for four columns before Matches has anything to elide. Sized here
    // rather than in the pages because the dialog is what the user sees, and
    // one number beats four minimums fighting over the same window.
    dialog.resize(960, 640);

    auto *categoryList = new QListWidget(&dialog);
    categoryList->addItem(QObject::tr("Appearance"));
    categoryList->addItem(QObject::tr("Editor"));
    categoryList->addItem(QObject::tr("Syntax Colors"));
    categoryList->addItem(QObject::tr("Keymap"));
    categoryList->addItem(QObject::tr("Languages"));
    categoryList->addItem(QObject::tr("Language Servers"));
    categoryList->addItem(QObject::tr("AI Providers"));
    categoryList->addItem(QObject::tr("Plugins"));
    categoryList->addItem(QObject::tr("MCP"));
    // Derived from the widest category rather than a fixed 150px: the
    // interface font scale below can make "Language Servers" wider than any
    // constant chosen for one font size, and a clipped category list is the
    // first thing a user of the scale setting would see.
    categoryList->setMaximumWidth(
      categoryList->fontMetrics().horizontalAdvance(QObject::tr("Language Servers")) + 40);

    auto *pages = new QStackedWidget(&dialog);

    // Every cached icon behind the tree, dropped: called by the Appearance
    // page when either theme changes, and by the Plugins page when a plugin
    // that contributes icons is switched off.
    auto refreshIcons = [uiFontTargets, editorTabs]() {
        refreshTreeIcons(uiFontTargets.projectTree);
        editorTabs->refreshTabIcons();
    };

    const AppearancePage appearance = buildAppearancePage(
      &dialog, appSettings, uiFontTargets,
      AppearanceHooks{
        [editorTabs]() { editorTabs->refreshHighlighting(); },
        refreshIcons,
        [categoryList]() {
            // The dialog is scaling under its own feet: its category list
            // was sized for the font in force when it opened.
            categoryList->setMaximumWidth(
              categoryList->fontMetrics().horizontalAdvance(QObject::tr("Language Servers")) + 40);
        },
      });
    pages->addWidget(appearance.widget);

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
    auto currentLineColor = std::make_shared<QString>(originalColors.current_line);
    auto applyColorsLive = [editorTabs, backgroundColor, foregroundColor, currentLineColor]() {
        editorTabs->setEditorColors(*backgroundColor, *foregroundColor, *currentLineColor);
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

    auto *currentLineButton = new QPushButton(QObject::tr("Current Line Color..."), editorPage);
    QObject::connect(currentLineButton, &QPushButton::clicked, &dialog,
                      [&dialog, currentLineColor, applyColorsLive]() {
                          // Empty means "derived from the theme", which has no
                          // single hex to seed the picker with — the editor
                          // background is the closest starting point.
                          const QColor initial = currentLineColor->isEmpty()
                            ? qApp->palette().color(QPalette::Base)
                            : QColor(*currentLineColor);
                          const QColor chosen = QColorDialog::getColor(
                            initial, &dialog, QObject::tr("Current Line Color"));
                          if (chosen.isValid()) {
                              *currentLineColor = chosen.name();
                              applyColorsLive();
                          }
                      });
    editorForm->addRow(currentLineButton);

    pages->addWidget(editorPage);

    // Syntax Colors follows Appearance rather than Keymap: it applies live,
    // so the user sees the colour in the open editor while picking it, and
    // the Cancel branch below reverts it the same way the theme is reverted.
    syntaxColorEditor->beginEdit();
    pages->addWidget(buildSyntaxColorsPage(
      &dialog, syntaxColorEditor, QFont(originalFont.family, static_cast<int>(originalFont.size)),
      [editorTabs]() { editorTabs->refreshHighlighting(); }));

    // Unlike Appearance/Editor, the keymap isn't applied live: the page edits
    // a draft held in Rust, so Cancel discards it by never committing, and
    // the next beginEdit() re-reads from disk.
    keymapEditor->beginEdit();
    pages->addWidget(buildKeymapPage(&dialog, keymapEditor));

    // Languages needs no draft: nothing on it is a setting. Adding a
    // language, clearing a quarantine and reloading all take effect when
    // pressed, which is why the page offers no OK-shaped promise.
    pages->addWidget(buildLanguagesPage(
      &dialog, languageCatalog,
      [&dialog, editorTabs](const QString &path) {
          editorTabs->openFileAtLine(path, 1, 1);
          dialog.accept();
      },
      [editorTabs]() { editorTabs->reloadHighlighterLanguages(); }));

    // Language Servers commits on OK, like Keymap and MCP: starting and
    // stopping a server on every keystroke in a command field is not a
    // preview.
    languageServerEditor->beginEdit();
    pages->addWidget(buildLanguageServersPage(&dialog, languageServerEditor, languageService));

    // AI Providers sits next to Language Servers — both configure an
    // external process the IDE talks to — and commits on OK for the same
    // reason: a half-typed base URL is not a setting worth applying. There
    // is no API key field on the page, by ADR-0021 decision 3.
    aiProviderEditor->beginEdit();
    pages->addWidget(buildAiProvidersPage(&dialog, aiProviderEditor));

    // Plugins needs no draft, for the reason Languages needs none: nothing
    // on it is a setting the dialog holds. Switching a plugin off rebuilds
    // the registry there and then, which is why the page makes no
    // OK-shaped promise.
    pages->addWidget(buildPluginsPage(&dialog, pluginCatalog, refreshIcons));

    // MCP, like Keymap and unlike Appearance/Editor, commits on OK rather
    // than applying live: restarting the server on every keystroke in the
    // port field would bind a series of half-typed port numbers.
    auto *mcpPage = new QWidget(&dialog);
    auto *mcpForm = new QFormLayout(mcpPage);
    auto *mcpEnabledCheck = new QCheckBox(QObject::tr("Enable MCP server"), mcpPage);
    mcpEnabledCheck->setChecked(appSettings->mcpEnabled());
    mcpForm->addRow(mcpEnabledCheck);

    auto *mcpPortSpin = new QSpinBox(mcpPage);
    mcpPortSpin->setRange(0, 65535);
    mcpPortSpin->setSpecialValueText(QObject::tr("Automatic"));
    mcpPortSpin->setValue(static_cast<int>(appSettings->mcpPort()));
    mcpPortSpin->setEnabled(mcpEnabledCheck->isChecked());
    mcpForm->addRow(QObject::tr("Port:"), mcpPortSpin);
    QObject::connect(mcpEnabledCheck, &QCheckBox::toggled, mcpPortSpin, &QSpinBox::setEnabled);

    auto *mcpStatusLabel = new QLabel(*mcpStatus, mcpPage);
    mcpStatusLabel->setWordWrap(true);
    mcpForm->addRow(QObject::tr("Status:"), mcpStatusLabel);
    // Live only while the dialog is open, so a failed restart on OK is
    // visible without reopening Settings.
    QObject::connect(docManager, &DocumentManager::mcpStarted, &dialog,
                      [mcpStatusLabel](std::uint16_t port) {
                          mcpStatusLabel->setText(
                            QObject::tr("Listening on 127.0.0.1:%1").arg(port));
                      });
    QObject::connect(docManager, &DocumentManager::mcpStopped, &dialog, [mcpStatusLabel]() {
        mcpStatusLabel->setText(QObject::tr("Disabled"));
    });
    QObject::connect(docManager, &DocumentManager::mcpFailed, &dialog,
                      [mcpStatusLabel](const QString &message) {
                          mcpStatusLabel->setText(message);
                      });

    // The port and token an agent needs are written here on every start,
    // so the useful thing to show is where to read them from.
    auto *mcpDiscoveryEdit = new QLineEdit(appSettings->mcpDiscoveryFilePath(), mcpPage);
    mcpDiscoveryEdit->setReadOnly(true);
    mcpForm->addRow(QObject::tr("Discovery file:"), mcpDiscoveryEdit);

    pages->addWidget(mcpPage);

    QObject::connect(categoryList, &QListWidget::currentRowChanged, pages,
                      &QStackedWidget::setCurrentIndex);
    categoryList->setCurrentRow(0);

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, &dialog);
    // OK runs the AI page's commit first, because it is the one page that
    // can refuse: `settings-model` validates the draft and says what is
    // wrong with it, and a false answer means the dialog stays open on the
    // field the user has to fix. Nothing else is committed until it passes.
    QObject::connect(buttons, &QDialogButtonBox::accepted, &dialog,
                      [&dialog, aiProviderEditor]() {
                          if (commitAiProvidersPage(&dialog, aiProviderEditor)) {
                              dialog.accept();
                          }
                      });
    QObject::connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);

    auto *bodyLayout = new QHBoxLayout();
    bodyLayout->addWidget(categoryList);
    bodyLayout->addWidget(pages, 1);

    auto *mainLayout = new QVBoxLayout(&dialog);
    mainLayout->addLayout(bodyLayout);
    mainLayout->addWidget(buttons);

    if (dialog.exec() == QDialog::Accepted) {
        appearance.commit();
        appSettings->saveEditorFont(fontFamilyEdit->text(),
                                     static_cast<quint32>(fontSizeSpin->value()));
        appSettings->saveEditorColors(*backgroundColor, *foregroundColor, *currentLineColor);
        keymapEditor->commit();
        applyKeymap(actions, appSettings);
        appSettings->saveMcpSettings(mcpEnabledCheck->isChecked(),
                                      static_cast<quint16>(mcpPortSpin->value()));
        // Unconditional: applyMcpSettings is idempotent, and working out
        // whether anything changed here would be the view deciding
        // something the Rust side already decides.
        docManager->applyMcpSettings();
        // The AI draft was already committed by the OK handler above; this
        // is the chat session re-reading the provider, the mode and the
        // persistence setting it had cached.
        aiChat->applyAiSettings();
        languageServerEditor->commit();
        // Reconciling is the Rust side's decision: it stops what the new
        // settings no longer describe and leaves the rest running, and the
        // re-announcement below starts the replacements.
        languageService->applyServerSettings();
        editorTabs->reannounceDocuments();
    } else {
        aiProviderEditor->revert();
        syntaxColorEditor->revert();
        appearance.revert();
        editorTabs->refreshHighlighting();
        editorTabs->setEditorFont(QFont(originalFont.family, static_cast<int>(originalFont.size)));
        editorTabs->setEditorColors(originalColors.background, originalColors.foreground,
                                     originalColors.current_line);
    }
}

// Sidebar tree + tabbed editor area, PHPStorm-style (US-5): each panel is
// its own ADS CDockWidget (D3) — float/redock each independently, room left
// for future dock widgets (search, run console, MCP activity log) without
// restructuring this function again. The editor stays one dock widget (not
// one per open file, per the plan's migration scope): the splits inside it
// are a QSplitter tree of tab groups owned by EditorTabs (D5), invisible to
// ADS, and G2's drag-reorder stays internal to each group's QTabWidget.
// Return value of buildCentralWidget(): the tab-strip adapter (needed by
// menu wiring) plus the dock manager (needed by IdeMainWindow for D4's
// close-time saveState()) — one caller, so a tiny struct beats an
// out-param.
struct CentralWidgets
{
    EditorTabs *editorTabs;
    ads::CDockManager *dockManager;
    // Only reason it escapes: the project tree carries its own interface
    // font scale, so buildMainWindow has to be able to hand it to
    // applyUiFontScales().
    QTreeView *projectTree;
    SearchResultsPanel *searchResultsPanel;
    ads::CDockWidget *searchResultsDock;
    ClassViewPanel *classViewPanel;
    ads::CDockWidget *classViewDock;
    ads::CDockWidget *terminalDock;
    TerminalWidget *terminalWidget;
    FindUsagesPanel *findUsagesPanel;
    ads::CDockWidget *findUsagesDock;
    SearchEverywhereDialog *searchEverywhereDialog;
    ProblemsPanel *problemsPanel;
    ads::CDockWidget *problemsDock;
    AiChatPanel *aiChatPanel;
    ads::CDockWidget *aiChatDock;
};

CentralWidgets buildCentralWidget(QMainWindow *window, ProjectTreeModel *treeModel,
                                   DocumentManager *docManager, AppSettings *appSettings,
                                   SearchModel *searchModel, TerminalSession *terminalSession,
                                   LanguageService *languageService, AiChat *aiChat)
{
    // Constructing with `window` (a QMainWindow) as parent makes the dock
    // manager install itself as the central widget automatically (ADS's own
    // CDockManager::CDockManager) — no explicit QMainWindow::setCentralWidget().
    auto *dockManager = new ads::CDockManager(window);

    // The editor area is a QSplitter tree of tab groups (see EditorTabs) so
    // a tab can be split off into a second pane; ADS still sees the whole
    // tree as this one dock widget, leaving D4's dock save/restore alone.
    auto *editorRoot = new QSplitter(Qt::Horizontal);
    auto *editorDock = new ads::CDockWidget(dockManager, QObject::tr("Editor"));
    editorDock->setWidget(editorRoot);
    // The editor is ADS's *central* dock widget, not an ordinary center-area
    // one: a central widget absorbs the leftover space, so the side and
    // bottom panels keep their size hints instead of splitting the window
    // into equal shares and squeezing the editor down to nothing.
    auto *editorArea = dockManager->setCentralWidget(editorDock);

    QTreeView *treeView = createProjectTreeDock(dockManager, editorArea, treeModel);

    auto *editorTabs = new EditorTabs(docManager, languageService, editorRoot, window);

    // Task H: bottom dock panel, matching where JetBrains/VS-style IDEs
    // dock their Find in Files results. Reuses the one EditorTabs instance
    // above (its openFileAtLine) to open a match rather than a second,
    // parallel "open file" path.
    auto openAt = [editorTabs](const QString &path, int line, int column) {
        editorTabs->openFileAtLine(path, line, column);
    };
    auto *searchResultsPanel = new SearchResultsPanel(searchModel, openAt, dockManager);
    auto *searchResultsDock = new ads::CDockWidget(dockManager, QObject::tr("Search Results"));
    searchResultsDock->setWidget(searchResultsPanel);
    // First bottom panel: it creates the bottom dock area; every panel after
    // it is added *into* that area (CenterDockWidgetArea) so they become tabs
    // rather than each stacking one more split between editor and status bar.
    auto *bottomArea =
      dockManager->addDockWidget(ads::BottomDockWidgetArea, searchResultsDock, editorArea);

    // Task J: bottom dock panel, tabbed alongside Find in Files — same
    // "list of locations" shape, just fed by a symbol name instead of typed
    // free text. Built before ClassViewPanel so its "Find Usages" callback
    // (below) can capture this panel and its dock widget.
    auto *findUsagesPanel = new FindUsagesPanel(searchModel, editorTabs, dockManager);
    auto *findUsagesDock = new ads::CDockWidget(dockManager, QObject::tr("Find Usages"));
    findUsagesDock->setWidget(findUsagesPanel);
    dockManager->addDockWidget(ads::CenterDockWidgetArea, findUsagesDock, bottomArea);

    // Task D: right-side dock panel, matching where JetBrains-style IDEs
    // dock their Class/Structure View. Reuses the one EditorTabs instance
    // above (its jumpToByteOffset) rather than a second navigation path.
    // Task J extends it with a "Find Usages" context-menu action that
    // raises findUsagesDock and runs the query there.
    auto *classViewPanel = new ClassViewPanel(
      docManager, searchModel, editorTabs,
      [findUsagesPanel, findUsagesDock](const QString &name) {
          findUsagesDock->toggleView(true);
          findUsagesDock->raise();
          findUsagesPanel->findUsages(name);
      },
      dockManager);
    auto *classViewDock = new ads::CDockWidget(dockManager, QObject::tr("Class View"));
    classViewDock->setWidget(classViewPanel);
    auto *rightArea =
      dockManager->addDockWidget(ads::RightDockWidgetArea, classViewDock, editorArea);

    // AC16/AC17: the AI Chat dock, tabbed into the right-hand area
    // (CenterDockWidgetArea) exactly as Find Usages and Problems tab into
    // the bottom one — it sits beside the code it is talking about rather
    // than squeezing a third split into the window. Its callbacks (the
    // current buffer text, and applying a code block) are set in
    // buildMainWindow, which is where the editor lives.
    auto *aiChatPanel = new AiChatPanel(aiChat, searchModel, dockManager);
    auto *aiChatDock = new ads::CDockWidget(dockManager, QObject::tr("AI Chat"));
    aiChatDock->setWidget(aiChatPanel);
    dockManager->addDockWidget(ads::CenterDockWidgetArea, aiChatDock, rightArea);

    // Search Everywhere: a transient popup parented to the top-level window
    // (not the dock manager) since it's a floating overlay, not a dock
    // widget. It hands a query off to the Search Results dock on Ctrl+Enter,
    // which is why it is built after that panel. The action map it triggers
    // commands through is filled later in buildMainWindow, so it takes a
    // pointer to the map rather than a copy.
    auto *searchEverywhereDialog =
      new SearchEverywhereDialog(searchModel, openAt, searchResultsPanel, window);

    // Task F3: bottom dock panel, tabbed alongside Find in Files — the
    // conventional spot for an embedded shell in JetBrains/VS-style IDEs.
    // The widget itself only starts the PTY once it's actually shown/sized
    // (TerminalWidget::showEvent/resizeEvent), not eagerly here.
    // Task L2: the Problems panel, tabbed into the same bottom area as Find
    // in Files and Find Usages — the same "list of locations" shape, fed by
    // the language servers instead of a query.
    auto *problemsPanel = new ProblemsPanel(languageService, openAt, dockManager);
    auto *problemsDock = new ads::CDockWidget(dockManager, QObject::tr("Problems"));
    problemsDock->setWidget(problemsPanel);
    dockManager->addDockWidget(ads::CenterDockWidgetArea, problemsDock, bottomArea);
    // Hidden until there is something to show (or the View menu asks): it
    // opens itself once per session, the first time a diagnostic arrives.
    problemsDock->toggleView(false);
    problemsPanel->setFirstDiagnosticCallback([problemsDock]() {
        problemsDock->toggleView(true);
    });
    // The squiggles and the panel read the same store, so one signal drives
    // both.
    QObject::connect(languageService, &LanguageService::diagnosticsChanged, editorTabs,
                      [editorTabs]() { editorTabs->applyDiagnostics(); });

    auto *terminalWidget = new TerminalWidget(terminalSession, appSettings, dockManager);
    auto *terminalDock = new ads::CDockWidget(dockManager, QObject::tr("Terminal"));
    terminalDock->setWidget(terminalWidget);
    dockManager->addDockWidget(ads::CenterDockWidgetArea, terminalDock, bottomArea);

    // Class View tracks whatever tab is current: refresh on open, on
    // switch, and whenever a tab becomes clean. `tabModifiedChanged`
    // firing with `modified == false` doubles as "just saved" — there is
    // no separate "save completed" signal, and this one already fires
    // exactly when EditorTabs::saveTab succeeds (it forwards
    // QTextDocument::modificationChanged, which setModified(false) there
    // triggers). It also fires on initial load and on undo-to-clean, both
    // harmless extra refreshes of the same content.
    QObject::connect(docManager, &DocumentManager::tabOpened, classViewPanel,
                      [classViewPanel, editorTabs](quint64, const QString &) {
                          classViewPanel->refresh(editorTabs->currentTabId());
                      });
    editorTabs->setActiveTabChangedCallback([classViewPanel, editorTabs, problemsPanel]() {
        classViewPanel->refresh(editorTabs->currentTabId());
        // The current file's group sorts to the top of the Problems panel.
        problemsPanel->setCurrentFile(editorTabs->currentPath());
    });
    QObject::connect(docManager, &DocumentManager::tabModifiedChanged, classViewPanel,
                      [classViewPanel, editorTabs](quint64 tabId, bool modified) {
                          if (!modified && tabId == editorTabs->currentTabId()) {
                              classViewPanel->refresh(tabId);
                          }
                      });

    // Open the project's text index off the same project-open lifecycle
    // event the tree/watcher already use (no second, parallel hook). Opening
    // reuses whatever is already on disk and re-reads only what changed, so
    // a warm start costs a walk rather than a full index build.
    QObject::connect(treeModel,
                      &ProjectTreeModel::projectOpened,
                      searchModel,
                      [searchModel](const QString &rootPath) { searchModel->openIndex(rootPath); });

    QObject::connect(treeModel, &ProjectTreeModel::projectOpened, treeModel,
                      [](const QString &rootPath) {
                          e2eMark(QStringLiteral("{\"ev\":\"project_opened\",\"root\":%1}")
                                    .arg(e2eJson(rootPath)));
                      });

    // Same project-open lifecycle event for the language servers: the root is
    // what `initialize` reports, and re-opening a project must not leave the
    // previous one's servers running.
    QObject::connect(treeModel,
                      &ProjectTreeModel::projectOpened,
                      languageService,
                      [languageService](const QString &rootPath) {
                          languageService->openProject(rootPath);
                      });

    // Initial bottom-panel height: without it the area is sized from the
    // terminal's tiny size hint (~60px), which is unusable for every panel
    // tabbed there. Overridden by restoreState() below once a layout has
    // been saved.
    dockManager->setSplitterSizes(bottomArea, {520, 200});

    // D4: restored after both dock widgets exist for this layout to apply
    // to (ADS matches saved widgets by their title/object name). Empty
    // means nothing was ever saved — first launch, or window_state predates
    // D4 — so the layout built above (tree left of editor) stands as-is.
    const QString savedState = appSettings->windowState();
    if (!savedState.isEmpty()) {
        dockManager->restoreState(QByteArray::fromBase64(savedState.toLatin1()));
    }

    // Filesystem-watcher plumbing: ProjectTreeModel's watcher-driven signal
    // already carries the changed path and already runs on the Qt thread
    // (queued there via CxxQtThread), so relaying it to DocumentManager is a
    // plain same-thread signal/slot connection — no further cross-thread
    // hop. The session decides whether the change warrants a prompt.
    QObject::connect(treeModel,
                      &ProjectTreeModel::filesChangedExternally,
                      docManager,
                      [docManager](const QString &path) { docManager->checkExternalChange(path); });

    // Keep the search index in step with the disk. Paths are coalesced over a
    // short window because a single save can produce several watcher events,
    // and re-indexing a file is far more expensive than remembering its name.
    auto *dirtyPaths = new QSet<QString>();
    auto *reindexTimer = new QTimer(window);
    reindexTimer->setSingleShot(true);
    reindexTimer->setInterval(300);
    QObject::connect(reindexTimer, &QTimer::timeout, searchModel, [searchModel, dirtyPaths]() {
        // The whole window goes over as one call: whether a path is
        // re-indexed or dropped is decided in Rust from whether it still
        // exists, and the batch shares a single commit.
        searchModel->syncIndexedFiles(QStringList(dirtyPaths->values()));
        dirtyPaths->clear();
    });
    QObject::connect(treeModel,
                      &ProjectTreeModel::filesChangedExternally,
                      searchModel,
                      [dirtyPaths, reindexTimer](const QString &path) {
                          dirtyPaths->insert(path);
                          reindexTimer->start();
                      });

    // Every file the user opens feeds Search Everywhere's Recent tier.
    QObject::connect(docManager,
                      &DocumentManager::tabOpened,
                      searchModel,
                      [searchModel, docManager](quint64 tabId, const QString &) {
                          const QString path = docManager->tabPath(tabId);
                          if (!path.isEmpty()) {
                              searchModel->noteRecentFile(path);
                          }
                      });

    QObject::connect(docManager,
                      &DocumentManager::externalChangeDetected,
                      editorTabs,
                      [editorTabs](quint64 tabId, const QString &path) {
                          editorTabs->handleExternalChange(tabId, path);
                      });

    // MCP's edit_buffer tool (M5) changed a tab's content (M3's listener
    // thread relayed it here via CxxQtThread::queue already) — reflect it
    // in the widget.
    QObject::connect(docManager,
                      &DocumentManager::bufferEditedExternally,
                      editorTabs,
                      [editorTabs](quint64 tabId, const QString &content) {
                          editorTabs->onBufferEditedExternally(tabId, content);
                      });

    // A tree-driven rename/delete retitled an open tab (US-2b).
    QObject::connect(treeModel,
                      &ProjectTreeModel::tabTitleChanged,
                      editorTabs,
                      [editorTabs](quint64 tabId, const QString &title) {
                          editorTabs->onTabTitleChanged(tabId, title);
                      });

    wireProjectTree(treeView,
                    treeModel,
                    ProjectTreeActions{window,
                                       aiChat,
                                       aiChatPanel,
                                       aiChatDock,
                                       classViewDock,
                                       dockManager,
                                       [editorTabs](const QString &path) {
                                           editorTabs->openFile(path);
                                       }});

    return CentralWidgets{editorTabs,        dockManager,     treeView,
                           searchResultsPanel, searchResultsDock, classViewPanel,
                           classViewDock,     terminalDock,    terminalWidget,
                           findUsagesPanel,   findUsagesDock,  searchEverywhereDialog,
                           problemsPanel,     problemsDock,    aiChatPanel,
                           aiChatDock};
}

// Menu structure per US-5 acceptance criteria. "Open Folder..." and the
// Edit/Save actions are wired to the tabbed editor area; the rest remain
// non-functional stubs for later tasks.
// `progress` is called once per startup stage (1-based, see
// SplashScreen::StageCount) so the splash can show what is taking time. The
// stages are the blocking steps below, in the order they already ran.
QMainWindow *buildMainWindow(AppSettings *appSettings,
                              const std::function<void(int, const QString &)> &progress)
{
    progress(1, QObject::tr("Loading settings..."));

    auto *window = new IdeMainWindow();
    window->setWindowTitle(QStringLiteral("IDE"));

    // Created by run_app() before the splash so the persisted theme is known
    // early enough to paint it; adopted by the window here as before.
    appSettings->setParent(window);
    // Holds the Settings > Keymap page's draft between beginEdit() and
    // commit(); parented to the window so it outlives each dialog.
    auto *keymapEditor = new KeymapEditor(window);
    // The same arrangement for the three language-platform pages (T4, G3,
    // L6): each holds its page's state between the dialog's beginEdit() and
    // its commit/revert, and is parented to the window so it outlives the
    // dialog.
    auto *syntaxColorEditor = new SyntaxColorEditor(window);
    auto *languageCatalog = new LanguageCatalog(window);
    auto *languageServerEditor = new LanguageServerEditor(window);
    // P7's Plugins page, the same arrangement again: it holds the rows of
    // the last scan between the dialog's refresh() calls.
    auto *pluginCatalog = new PluginCatalog(window);

    const FfiWindowGeometry savedGeometry = appSettings->windowGeometry();
    if (savedGeometry.width > 0 && savedGeometry.height > 0) {
        window->setGeometry(savedGeometry.x, savedGeometry.y,
                             static_cast<int>(savedGeometry.width),
                             static_cast<int>(savedGeometry.height));
    } else {
        window->resize(1024, 768);
    }

    progress(2, QObject::tr("Starting services..."));

    auto *treeModel = new ProjectTreeModel(window);
    auto *docManager = new DocumentManager(window);
    auto *searchModel = new SearchModel(window);
    // Task F3: one terminal session for the one "Terminal" dock widget —
    // same one-QObject-per-dock-widget shape SearchModel/DocumentManager
    // establish above. The shell isn't spawned yet (TerminalSession::start
    // hasn't been called) until TerminalWidget knows its own pixel size.
    auto *terminalSession = new TerminalSession(window);
    // Task L2: one language-server adapter per window, alongside the other
    // per-window QObjects. It launches nothing until a project is opened and
    // a file of a configured language is opened in it.
    auto *languageService = new LanguageService(window);
    // ADR-0021: one AI chat session per window, alongside the other
    // per-window QObjects, plus the Settings > AI Providers draft — the same
    // arrangement KeymapEditor and LanguageServerEditor use, parented to the
    // window so it outlives each dialog. Both read the persisted AI settings
    // on the way up, so both are built after appSettings.
    auto *aiChat = new AiChat(window);
    auto *aiProviderEditor = new AiProviderEditor(window);
    // One MCP server per process, brought up right after the shared
    // DocumentManager exists — the listener thread it spawns dispatches
    // every EditorCommand back onto this same QObject's Qt thread. Whether
    // it listens at all, and on which port, is the Rust side's decision
    // from settings; this call only says "make it match".
    //
    // The status string outlives the Settings dialog, so reopening Settings
    // shows what the server is actually doing rather than a stale guess.
    auto mcpStatus = std::make_shared<QString>(QObject::tr("Starting..."));
    QObject::connect(docManager, &DocumentManager::mcpStarted, window,
                      [mcpStatus](std::uint16_t port) {
                          *mcpStatus = QObject::tr("Listening on 127.0.0.1:%1").arg(port);
                      });
    QObject::connect(docManager, &DocumentManager::mcpStopped, window,
                      [mcpStatus]() { *mcpStatus = QObject::tr("Disabled"); });
    QObject::connect(docManager, &DocumentManager::mcpFailed, window,
                      [mcpStatus](const QString &message) { *mcpStatus = message; });
    docManager->applyMcpSettings();
    progress(3, QObject::tr("Building workspace..."));
    const CentralWidgets central =
      buildCentralWidget(window, treeModel, docManager, appSettings, searchModel, terminalSession,
                          languageService, aiChat);
    EditorTabs *editorTabs = central.editorTabs;

    // Every path that shows the AI chat goes through here, because a dock
    // that a restored layout never mentioned needs putting back before it
    // can be raised.
    //
    // ADS flags a dock absent from the saved blob as unassigned
    // (DockManager::restoreDockWidgetsOpenState): closed, un-parented and
    // with no dock area. Reopening one in that state takes the floating
    // path (CDockWidget::showDockWidget, which floats when DockArea is
    // null), so a user whose window_state predates this dock would get a
    // detached window instead of the tab beside Class View that
    // buildCentralWidget arranged. Re-adding it first is what keeps
    // "show the panel" meaning the same thing on every config.
    const auto showAiChat = [central]() {
        showAiChatDock(central.dockManager, central.aiChatDock, central.classViewDock);
    };

    window->setEditorTabs(editorTabs);
    window->setAppSettings(appSettings);
    window->setDockManager(central.dockManager);
    window->setDocumentManager(docManager);

    // S2: applied before reopenLastProject() (below) opens any tabs, so
    // every tab — including ones opened at startup — starts with the
    // persisted font/colors rather than the QPlainTextEdit default.
    const FfiEditorFont savedFont = appSettings->editorFont();
    editorTabs->setEditorFont(QFont(savedFont.family, static_cast<int>(savedFont.size)));
    const FfiEditorColors savedColors = appSettings->editorColors();
    editorTabs->setEditorColors(savedColors.background, savedColors.foreground,
                                 savedColors.current_line);

    // The AI panel has no route to the editor, so the window hands it the
    // two things it asks for: the buffer the user is looking at, and what
    // Apply means.
    AiChatPanel *aiChatPanel = central.aiChatPanel;
    aiChatPanel->setCurrentTextProvider([editorTabs]() { return editorTabs->currentContent(); });
    aiChatPanel->setApplyHandler(
      [window, aiChat, aiChatPanel, editorTabs, searchModel](quint64 messageIndex,
                                                              quint64 blockIndex) {
          // The same protocol — and the same discipline — a refactoring
          // runs (ADR-0021 §5): the revision is read *before* the plan is
          // made and handed back to `takePendingEdits`, so an answer
          // applied to a buffer that has since moved is refused by
          // `lsp_core::EditGate` instead of being spliced in blind.
          const int revision = editorTabs->documentRevision();
          const FfiRefactorSummary summary =
            aiChat->prepareApply(messageIndex, blockIndex, aiChatPanel->currentText(), revision);
          if (summary.title.isEmpty()) {
              // Nothing was planned. Why is `ai-chat-core`'s sentence, and
              // the user pressed a button, so it is said out loud rather
              // than dropped in the status bar.
              QMessageBox::information(window, QObject::tr("AI Chat"),
                                        aiChat->applyRefusal().message);
              return;
          }

          // A change confined to the file the user is looking at applies
          // straight away and is undone with Ctrl+Z; anything wider is
          // shown first — and which of the two this is was decided in Rust,
          // exactly as for a refactoring.
          if (summary.touches_other_files) {
              QList<RefactorPreviewDialog::Row> rows;
              for (const FfiTextEdit &edit : aiChat->pendingEdits()) {
                  rows.append({edit.path, static_cast<int>(edit.start_line),
                                previewText(edit.new_text), true, true});
              }
              RefactorPreviewDialog dialog(
                summary.title,
                QObject::tr("%n change(s) across %1 file(s). Changes to files that are not open "
                            "are written to disk and cannot be undone.",
                            "", static_cast<int>(summary.edit_count))
                  .arg(summary.document_count),
                rows, window);
              if (dialog.exec() != QDialog::Accepted) {
                  aiChat->cancelApply();
                  return;
              }
              for (const QString &path : dialog.excludedPaths()) {
                  aiChat->excludeFromApply(path);
              }
          }

          const ::rust::Vec<FfiTextEdit> edits = aiChat->takePendingEdits(revision);
          if (edits.empty()) {
              window->statusBar()->showMessage(
                QObject::tr("The file changed while the change was being prepared; nothing was "
                            "applied."),
                6000);
              return;
          }
          editorTabs->applyBufferEdits(edits);
          // Files nobody has open are rewritten and re-indexed by the index
          // worker; it ignores the buffer edits in the same vector.
          searchModel->applyFileEdits(edits);
      });

    // Agent-mode tools take the same route MCP's edit_buffer does: the run
    // thread has already marshalled these onto the Qt thread
    // (CxxQtThread::queue), and each lands on the handler DocumentManager's
    // own signal would have reached. Without them the Rust Document moves
    // under an agent's edit while the widget keeps showing stale text.
    QObject::connect(aiChat, &AiChat::toolOpenedTab, editorTabs,
                      [editorTabs](quint64 tabId, const QString &title) {
                          editorTabs->onTabOpened(tabId, title);
                      });
    QObject::connect(aiChat, &AiChat::toolEditedBuffer, editorTabs,
                      [editorTabs](quint64 tabId, const QString &content) {
                          editorTabs->onBufferEditedExternally(tabId, content);
                      });
    QObject::connect(aiChat, &AiChat::toolSavedBuffer, editorTabs, [editorTabs](quint64 tabId) {
        editorTabs->onTabModifiedChanged(tabId, false);
    });

    // L3: line:col + language update per current tab / cursor move; "UTF-8"
    // is static since only UTF-8 is supported today (US-2b's binary-file
    // rejection already rules out anything else reaching an open tab).
    auto *statusBar = window->statusBar();
    auto *languageLabel = new QLabel(statusBar);
    auto *positionLabel = new QLabel(statusBar);
    auto *encodingLabel = new QLabel(QStringLiteral("UTF-8"), statusBar);
    // Task L2: a compact problem counter, coloured by the worst severity
    // present and empty when there is nothing wrong. A button rather than a
    // label because clicking it opens the Problems dock.
    auto *problemsButton = new QToolButton(statusBar);
    problemsButton->setAutoRaise(true);
    problemsButton->setVisible(false);
    QObject::connect(problemsButton, &QToolButton::clicked, window, [central]() {
        central.problemsDock->toggleView(true);
        central.problemsDock->raise();
        central.problemsPanel->focusTree();
    });
    const auto updateProblemsButton = [problemsButton, languageService]() {
        const FfiDiagnosticCounts counts = languageService->diagnosticCounts();
        const bool any = counts.errors > 0 || counts.warnings > 0;
        problemsButton->setVisible(any);
        if (!any) {
            return;
        }
        problemsButton->setText(QObject::tr("%1 errors, %2 warnings")
                                   .arg(counts.errors)
                                   .arg(counts.warnings));
        const QColor color = severityColor(counts.errors > 0 ? FfiSeverity::Error
                                                             : FfiSeverity::Warning);
        problemsButton->setStyleSheet(QStringLiteral("color: %1;").arg(color.name()));
    };
    QObject::connect(languageService, &LanguageService::diagnosticsChanged, window,
                      updateProblemsButton);
    // The project index builds on a background thread for seconds to minutes
    // after a folder is opened. Until this existed the only way to find that
    // out was to run a search and be told to try again later.
    // Two plain permanent widgets rather than a laid-out container: the
    // status bar already spaces its own children, and a container's label
    // stretches to fill whatever room is going, which pushed the bar a
    // hand's width away from its own caption.
    auto *indexLabel = new QLabel(statusBar);
    auto *indexBar = new QProgressBar(statusBar);
    indexLabel->setVisible(false);
    indexBar->setVisible(false);
    indexBar->setTextVisible(false);
    indexBar->setFixedWidth(90);
    indexBar->setFixedHeight(statusBar->fontMetrics().height());

    // Everything a font scale has to reach now exists: the menu bar is
    // created lazily by menuBar() just below, the tree came out of
    // buildCentralWidget, and the indexing bar is right above. Applied here
    // (rather than only in run_app) because the two per-widget scales have no
    // widget to land on until this point.
    const UiFontTargets uiFontTargets{window->menuBar(), central.projectTree, indexBar};
    applyUiFontScales(appSettings->uiFontScales(), uiFontTargets);
    QObject::connect(searchModel, &SearchModel::indexProgress, window,
                      [indexLabel, indexBar](quint32 done, quint32 total) {
                          const QString text =
                              QObject::tr("Indexing... %1/%2").arg(done).arg(total);
                          // Reserve the width of the widest reading this run
                          // will ever show — `total/total`. Without it the
                          // label is sized for "565/2223" one frame and
                          // "1204/2223" the next, and clips while it catches
                          // up.
                          indexLabel->setMinimumWidth(indexLabel->fontMetrics().horizontalAdvance(
                              QObject::tr("Indexing... %1/%2").arg(total).arg(total)));
                          indexLabel->setStyleSheet(QString());
                          indexLabel->setText(text);
                          indexBar->setRange(0, static_cast<int>(total));
                          indexBar->setValue(static_cast<int>(done));
                          indexLabel->setVisible(true);
                          indexBar->setVisible(true);
                      });
    QObject::connect(searchModel, &SearchModel::indexReady, window,
                      [indexLabel, indexBar]() {
                          indexLabel->setMinimumWidth(0);
                          indexLabel->setVisible(false);
                          indexBar->setVisible(false);
                      });
    QObject::connect(searchModel, &SearchModel::indexFailed, window,
                      [indexLabel, indexBar](const QString &message) {
                          indexLabel->setMinimumWidth(0);
                          indexBar->setVisible(false);
                          indexLabel->setStyleSheet(QStringLiteral("color: %1;")
                                                       .arg(severityColor(FfiSeverity::Error).name()));
                          indexLabel->setText(QObject::tr("Index failed: %1").arg(message));
                          indexLabel->setVisible(true);
                      });

    statusBar->addPermanentWidget(indexLabel);
    statusBar->addPermanentWidget(indexBar);
    statusBar->addPermanentWidget(problemsButton);
    statusBar->addPermanentWidget(languageLabel);
    statusBar->addPermanentWidget(positionLabel);
    statusBar->addPermanentWidget(encodingLabel);
    editorTabs->attachStatusBar(positionLabel, languageLabel);

    // Every menu action is registered under a stable id from
    // app_config::ACTIONS and takes its shortcut from the persisted keymap,
    // so Settings > Keymap can rebind any of them (nothing here hardcodes a
    // QKeySequence any more).
    // Boxed so the Preferences lambda (which runs long after this function
    // returns, and needs the *complete* registry including actions added
    // below it) shares one instance instead of capturing a dangling
    // reference — the same std::make_shared trick the settings dialog's
    // colour pickers use.
    progress(4, QObject::tr("Preparing menus..."));
    auto actions = std::make_shared<QHash<QString, QAction *>>();

    // The terminal's Copy/Paste are QActions on the terminal widget itself
    // (widget-scoped shortcuts, so Ctrl+C keeps reaching the shell), but they
    // are registered in the same map as the menu actions so Settings > Keymap
    // lists them and applyKeymap() re-applies a rebinding without a restart.
    actions->insert(QStringLiteral("terminal.copy"), central.terminalWidget->copyAction());
    actions->insert(QStringLiteral("terminal.paste"), central.terminalWidget->pasteAction());

    QMenu *fileMenu = window->menuBar()->addMenu(QObject::tr("&File"));
    QAction *openFolderAction = registerAction(fileMenu, QStringLiteral("file.openFolder"),
                                                QObject::tr("Open Folder..."), appSettings, *actions);
    QMenu *recentProjectsMenu = fileMenu->addMenu(QObject::tr("Recent Projects"));
    populateRecentProjectsMenu(recentProjectsMenu, appSettings, treeModel, window);
    fileMenu->addSeparator();
    QAction *saveAction = registerAction(fileMenu, QStringLiteral("file.save"),
                                          QObject::tr("Save"), appSettings, *actions);
    QAction *saveAsAction = registerAction(fileMenu, QStringLiteral("file.saveAs"),
                                            QObject::tr("Save As..."), appSettings, *actions);
    fileMenu->addSeparator();
    QAction *preferencesAction = registerAction(fileMenu, QStringLiteral("file.preferences"),
                                                 QObject::tr("Preferences..."), appSettings, *actions);
    fileMenu->addSeparator();
    QAction *exitAction = registerAction(fileMenu, QStringLiteral("file.exit"),
                                          QObject::tr("Exit"), appSettings, *actions);

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
                      [window, appSettings, editorTabs, keymapEditor, actions, docManager,
                       mcpStatus, syntaxColorEditor, languageCatalog, languageServerEditor,
                       languageService, aiProviderEditor, aiChat, pluginCatalog,
                       uiFontTargets]() {
                          showSettingsDialog(window, appSettings, editorTabs, keymapEditor,
                                              *actions, docManager, mcpStatus,
                                              syntaxColorEditor, languageCatalog,
                                              languageServerEditor, languageService,
                                              aiProviderEditor, aiChat,
                                              pluginCatalog, uiFontTargets);
                      });

    QMenu *editMenu = window->menuBar()->addMenu(QObject::tr("&Edit"));
    QAction *undoAction = registerAction(editMenu, QStringLiteral("edit.undo"),
                                          QObject::tr("Undo"), appSettings, *actions);
    QAction *redoAction = registerAction(editMenu, QStringLiteral("edit.redo"),
                                          QObject::tr("Redo"), appSettings, *actions);
    editMenu->addSeparator();
    QAction *cutAction = registerAction(editMenu, QStringLiteral("edit.cut"),
                                         QObject::tr("Cut"), appSettings, *actions);
    QAction *copyAction = registerAction(editMenu, QStringLiteral("edit.copy"),
                                          QObject::tr("Copy"), appSettings, *actions);
    QAction *pasteAction = registerAction(editMenu, QStringLiteral("edit.paste"),
                                           QObject::tr("Paste"), appSettings, *actions);
    editMenu->addSeparator();
    QAction *findAction = registerAction(editMenu, QStringLiteral("edit.find"),
                                         QObject::tr("Find..."), appSettings, *actions);
    QObject::connect(findAction, &QAction::triggered, window,
                     [editorTabs]() { editorTabs->showFindBar(); });
    QAction *replaceAction = registerAction(editMenu, QStringLiteral("edit.replace"),
                                            QObject::tr("Replace..."), appSettings, *actions);
    QObject::connect(replaceAction, &QAction::triggered, window,
                     [editorTabs]() { editorTabs->showReplaceBar(); });
    QAction *findNextAction = registerAction(editMenu, QStringLiteral("edit.findNext"),
                                             QObject::tr("Find Next"), appSettings, *actions);
    QObject::connect(findNextAction, &QAction::triggered, window,
                     [editorTabs]() { editorTabs->findNext(); });
    QAction *findPreviousAction = registerAction(editMenu, QStringLiteral("edit.findPrevious"),
                                                 QObject::tr("Find Previous"), appSettings,
                                                 *actions);
    QObject::connect(findPreviousAction, &QAction::triggered, window,
                     [editorTabs]() { editorTabs->findPrevious(); });
    QAction *findInFilesAction = registerAction(editMenu, QStringLiteral("edit.findInFiles"),
                                                 QObject::tr("Find in Files..."), appSettings,
                                                 *actions);

    // RF12: the hover signature fallback. `lsp_core::hover_outcome` decides
    // whether the server answered; this only starts the index leg when it
    // says no, and shows whatever comes back the same way a server's hover
    // is shown.
    editorTabs->setHoverFallbackCallback([editorTabs, searchModel]() {
        const QString path = editorTabs->currentPath();
        if (path.isEmpty()) {
            return;
        }
        searchModel->hoverSignature(path,
                                     editorTabs->currentContent(),
                                     editorTabs->byteOffsetAt(editorTabs->hoverPosition()));
    });
    editorTabs->setHoverCanceledCallback(
      [searchModel]() { searchModel->cancelHoverSignature(); });
    QObject::connect(languageService, &LanguageService::hoverFallback, window,
                      [editorTabs]() { editorTabs->hoverFallback(); });
    QObject::connect(searchModel, &SearchModel::hoverSignatureReady, window,
                      [](const QString &html) { QToolTip::showText(QCursor::pos(), html); });

    // RF11: the Refactor menu. Every entry routes through the one
    // RefactorController, so there is a single place that turns a server's
    // answer into an edit.
    auto *refactorer = new RefactorController(languageService, searchModel, editorTabs, window);
    QMenu *refactorMenu = window->menuBar()->addMenu(QObject::tr("Re&factor"));
    QAction *renameAction = registerAction(refactorMenu, QStringLiteral("refactor.rename"),
                                            QObject::tr("Rename..."), appSettings, *actions);
    QObject::connect(renameAction, &QAction::triggered, window,
                      [refactorer]() { refactorer->renameSymbol(); });

    refactorMenu->addSeparator();
    QAction *extractMethodAction =
      registerAction(refactorMenu, QStringLiteral("refactor.extractMethod"),
                      QObject::tr("Extract Method..."), appSettings, *actions);
    QObject::connect(extractMethodAction, &QAction::triggered, window, [refactorer]() {
        refactorer->extract(
          QStringLiteral("refactor.extract"),
          QObject::tr("The language server offers no method extraction for this selection."));
    });

    QAction *extractClassAction =
      registerAction(refactorMenu, QStringLiteral("refactor.extractClass"),
                      QObject::tr("Extract Class..."), appSettings, *actions);
    QObject::connect(extractClassAction, &QAction::triggered, window, [refactorer]() {
        refactorer->extract(
          QStringLiteral("refactor.extract.class"),
          QObject::tr("The language server offers no class extraction for this selection."));
    });

    QAction *refactorThisAction =
      registerAction(refactorMenu, QStringLiteral("refactor.refactorThis"),
                      QObject::tr("Refactor This..."), appSettings, *actions);
    QObject::connect(refactorThisAction, &QAction::triggered, window, [refactorer]() {
        refactorer->extract(QString(),
                            QObject::tr("The language server offers no refactorings here."));
    });

    // The same gestures on the editor's right-click menu. The actions are
    // looked up by id rather than captured, so this does not depend on which
    // menus have been built yet — and because they are the *same* QActions,
    // their shortcuts show here and a rebinding in Settings > Keymap reaches
    // both places at once.
    editorTabs->setContextMenuCallback([actions](QMenu *menu) {
        const auto append = [menu, actions](const QString &id) {
            if (QAction *action = actions->value(id)) {
                menu->addAction(action);
            }
        };
        menu->addSeparator();
        append(QStringLiteral("navigate.goToDeclaration"));
        append(QStringLiteral("navigate.findUsages"));
        menu->addSeparator();
        append(QStringLiteral("refactor.rename"));
        append(QStringLiteral("refactor.extractMethod"));
        append(QStringLiteral("refactor.extractClass"));
        append(QStringLiteral("refactor.refactorThis"));
        menu->addSeparator();
        append(QStringLiteral("ai.addSelection"));
        append(QStringLiteral("ai.addSelectionNewChat"));
    });

    QMenu *viewMenu = window->menuBar()->addMenu(QObject::tr("&View"));
    QAction *classViewAction = registerAction(viewMenu, QStringLiteral("view.classView"),
                                               QObject::tr("Class View"), appSettings, *actions);
    QObject::connect(classViewAction, &QAction::triggered, window, [central]() {
        central.classViewDock->toggleView(true);
        central.classViewDock->raise();
    });
    // The AI panel's show-action belongs here with every other dock's, not
    // only on the AI menu: a user looking for a hidden panel opens View.
    QAction *aiChatViewAction = registerAction(viewMenu, QStringLiteral("view.aiChat"),
                                               QObject::tr("AI Chat"), appSettings, *actions);
    QObject::connect(aiChatViewAction, &QAction::triggered, window, [central, showAiChat]() {
        showAiChat();
        central.aiChatPanel->focusComposer();
    });
    QAction *problemsAction = registerAction(viewMenu, QStringLiteral("view.problems"),
                                             QObject::tr("Problems"), appSettings, *actions);
    QObject::connect(problemsAction, &QAction::triggered, window, [central]() {
        central.problemsDock->toggleView(true);
        central.problemsDock->raise();
        central.problemsPanel->focusTree();
    });
    QAction *terminalAction = registerAction(viewMenu, QStringLiteral("view.terminal"),
                                             QObject::tr("Terminal"), appSettings, *actions);
    QObject::connect(terminalAction, &QAction::triggered, window, [central]() {
        central.terminalDock->toggleView(true);
        central.terminalDock->raise();
        if (QWidget *w = central.terminalDock->widget()) {
            w->setFocus();
        }
    });
    // Every entry point opens the same popup, just preselected on a
    // different tab — one search surface, several doors into it.
    QAction *searchEverywhereAction =
      registerAction(viewMenu, QStringLiteral("view.searchEverywhere"),
                     QObject::tr("Search Everywhere..."), appSettings, *actions);
    QObject::connect(searchEverywhereAction, &QAction::triggered, window, [central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::All);
    });
    QAction *goToFileAction = registerAction(viewMenu, QStringLiteral("view.goToFile"),
                                             QObject::tr("Go to File..."), appSettings, *actions);
    QObject::connect(goToFileAction, &QAction::triggered, window, [central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::Files);
    });
    QAction *findActionAction = registerAction(viewMenu, QStringLiteral("view.findAction"),
                                               QObject::tr("Find Action..."), appSettings, *actions);
    QObject::connect(findActionAction, &QAction::triggered, window, [central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::Actions);
    });
    QAction *goToSymbolAction = registerAction(viewMenu, QStringLiteral("view.goToSymbol"),
                                               QObject::tr("Go to Symbol..."), appSettings, *actions);
    QObject::connect(goToSymbolAction, &QAction::triggered, window, [central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::Symbols);
    });
    QAction *goToLineAction = registerAction(viewMenu, QStringLiteral("view.goToLine"),
                                             QObject::tr("Go to Line..."), appSettings, *actions);
    QObject::connect(goToLineAction, &QAction::triggered, window, [editorTabs]() {
        editorTabs->goToLine();
    });

    // N8: code navigation. The Ctrl+Click gesture and every action below
    // route through the one DeclarationNavigator, so there is a single
    // place that turns a resolution result into a jump.
    auto *navigator = new DeclarationNavigator(languageService, searchModel, editorTabs, window);
    editorTabs->setDeclarationRequestedCallback(
      [navigator](int position) { navigator->resolveAt(position); });

    QMenu *navigateMenu = window->menuBar()->addMenu(QObject::tr("&Navigate"));
    QAction *goToDeclarationAction =
      registerAction(navigateMenu, QStringLiteral("navigate.goToDeclaration"),
                      QObject::tr("Go to Declaration"), appSettings, *actions);
    QObject::connect(goToDeclarationAction, &QAction::triggered, window,
                      [editorTabs]() { editorTabs->requestDeclarationAtCaret(); });

    QAction *findUsagesAction =
      registerAction(navigateMenu, QStringLiteral("navigate.findUsages"),
                      QObject::tr("Find Usages"), appSettings, *actions);
    QObject::connect(findUsagesAction, &QAction::triggered, window, [central, editorTabs]() {
        const QString name = editorTabs->wordUnderCursor();
        if (name.isEmpty()) {
            return;
        }
        central.findUsagesDock->toggleView(true);
        central.findUsagesDock->raise();
        central.findUsagesPanel->findUsages(name);
    });

    QAction *goToImplementationAction =
      registerAction(navigateMenu, QStringLiteral("navigate.goToImplementation"),
                      QObject::tr("Go to Implementation"), appSettings, *actions);
    QObject::connect(goToImplementationAction, &QAction::triggered, window,
                      [central, editorTabs]() {
                          const QString name = editorTabs->wordUnderCursor();
                          if (name.isEmpty()) {
                              return;
                          }
                          central.findUsagesDock->toggleView(true);
                          central.findUsagesDock->raise();
                          central.findUsagesPanel->findImplementations(name);
                      });

    QAction *goToInterfaceAction =
      registerAction(navigateMenu, QStringLiteral("navigate.goToInterface"),
                      QObject::tr("Go to Interface"), appSettings, *actions);
    QObject::connect(goToInterfaceAction, &QAction::triggered, window, [central, editorTabs]() {
        const QString name = editorTabs->wordUnderCursor();
        if (name.isEmpty()) {
            return;
        }
        central.findUsagesDock->toggleView(true);
        central.findUsagesDock->raise();
        central.findUsagesPanel->findSupertypes(name);
    });

    navigateMenu->addSeparator();
    QAction *backAction = registerAction(navigateMenu, QStringLiteral("navigate.back"),
                                          QObject::tr("Back"), appSettings, *actions);
    QObject::connect(backAction, &QAction::triggered, window,
                      [editorTabs]() { editorTabs->jumpBack(); });
    QAction *forwardAction = registerAction(navigateMenu, QStringLiteral("navigate.forward"),
                                             QObject::tr("Forward"), appSettings, *actions);
    QObject::connect(forwardAction, &QAction::triggered, window,
                      [editorTabs]() { editorTabs->jumpForward(); });

    // Enabled state comes from the session's stack, not from a second copy
    // kept here. Applied once now and again after every jump.
    auto refreshNavigationActions = [editorTabs, backAction, forwardAction]() {
        backAction->setEnabled(editorTabs->canJumpBack());
        forwardAction->setEnabled(editorTabs->canJumpForward());
    };
    editorTabs->setNavigationChangedCallback(refreshNavigationActions);
    refreshNavigationActions();

    // ADR-0021: the AI menu. Every entry is a registered action, so its
    // shortcut comes from the persisted keymap and Settings > Keymap can
    // rebind it like any other.
    QMenu *aiMenu = window->menuBar()->addMenu(QObject::tr("&AI"));

    // Both selection entries share this: the only difference between them
    // is whether the conversation is cleared first, and duplicating the
    // 0-based-to-1-based conversion is how one of the two copies ends up
    // off by one.
    const auto attachSelection = [window, aiChat, central, editorTabs, showAiChat](bool newChat) {
        if (newChat) {
            aiChat->newConversation();
        }
        // The protocol positions the editor reports are 0-based; an
        // attachment names the lines the way the user reads them off the
        // gutter.
        const auto range = editorTabs->selectionRange();
        const FfiResult result = aiChat->attachSelection(editorTabs->currentPath(),
                                                          range.first.first + 1,
                                                          range.second.first + 1,
                                                          editorTabs->selectedText());
        if (result.code != 0) {
            // An attachment can be refused — a secret-shaped file, a path
            // outside the project — and the reason is Rust's sentence, not
            // one composed here.
            QMessageBox::information(window, QObject::tr("AI Chat"), result.message);
            return;
        }
        showAiChat();
        central.aiChatPanel->attachAndFocus();
    };

    QAction *aiAddSelectionAction =
      registerAction(aiMenu, QStringLiteral("ai.addSelection"),
                      QObject::tr("Add Selection to AI Chat"), appSettings, *actions);
    QObject::connect(aiAddSelectionAction, &QAction::triggered, window,
                      [attachSelection]() { attachSelection(false); });

    QAction *aiAddSelectionNewChatAction =
      registerAction(aiMenu, QStringLiteral("ai.addSelectionNewChat"),
                      QObject::tr("Add Selection to New AI Chat"), appSettings, *actions);
    QObject::connect(aiAddSelectionNewChatAction, &QAction::triggered, window,
                      [attachSelection]() { attachSelection(true); });

    QAction *aiAddFileAction = registerAction(aiMenu, QStringLiteral("ai.addFile"),
                                               QObject::tr("Add File to AI Chat"), appSettings,
                                               *actions);
    QObject::connect(aiAddFileAction, &QAction::triggered, window,
                      [window, aiChat, central, editorTabs, showAiChat]() {
                          const FfiResult result = aiChat->attachFile(editorTabs->currentPath());
                          if (result.code != 0) {
                              QMessageBox::information(window, QObject::tr("AI Chat"),
                                                        result.message);
                              return;
                          }
                          showAiChat();
                          central.aiChatPanel->attachAndFocus();
                      });

    aiMenu->addSeparator();
    QAction *aiNewChatAction = registerAction(aiMenu, QStringLiteral("ai.newChat"),
                                               QObject::tr("New AI Chat"), appSettings, *actions);
    QObject::connect(aiNewChatAction, &QAction::triggered, window,
                      [central, aiChat, showAiChat]() {
        aiChat->newConversation();
        showAiChat();
        central.aiChatPanel->attachAndFocus();
    });

    QAction *aiTogglePanelAction = registerAction(aiMenu, QStringLiteral("ai.togglePanel"),
                                                   QObject::tr("AI Chat"), appSettings, *actions);
    QObject::connect(aiTogglePanelAction, &QAction::triggered, window,
                      [central, showAiChat]() {
        // A real toggle, unlike the View menu's panels: this one has a
        // shortcut of its own, and a shortcut that only ever opens a panel
        // gives the user no way back with the same keys.
        if (central.aiChatDock->isClosed()) {
            showAiChat();
            central.aiChatPanel->focusComposer();
        } else {
            central.aiChatDock->toggleView(false);
        }
    });

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
    QObject::connect(findInFilesAction, &QAction::triggered, window, [central]() {
        central.searchResultsDock->toggleView(true);
        central.searchResultsDock->raise();
        central.searchResultsPanel->focusQuery();
    });

    // The popup triggers commands through this registry, which only exists
    // once every menu above has been built.
    central.searchEverywhereDialog->setActions(actions.get());
    // JetBrains' double-Shift gesture, on top of the rebindable shortcut.
    window->setSearchEverywhereTrigger([central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::All);
    });

    // US-1: relaunching the app reopens the last project automatically.
    // Reuses the same watcher-start path as "Open Folder...", so the tree
    // is live-refreshing from the moment it's populated.
    progress(5, QObject::tr("Restoring project..."));
    treeModel->reopenLastProject();

    // Reopens the persisted editor split layout last: after the font/color
    // settings above (so restored tabs are styled like any other) and after
    // the project is back, so restored files show up under a live tree.
    // Files are addressed by absolute path and reopen even if they sit
    // outside the reopened project.
    progress(6, QObject::tr("Restoring editors..."));
    editorTabs->restoreLayout(appSettings->editorLayout());

    return window;
}

} // namespace

int run_app()
{
    int argc = 0;
    QApplication app(argc, nullptr);

    // Parentless for now: the splash needs the persisted theme before any
    // window exists, and buildMainWindow() adopts this object as soon as it
    // has one.
    auto *appSettings = new AppSettings(nullptr);
    // Applying the theme (T2) before anything is shown means neither the
    // splash nor the main window ever flashes an unstyled frame.
    applyTheme(appSettings->themeName());
    // The global half of the interface font scale, before the splash for the
    // same reason: no frame is ever painted at a size the user did not pick.
    // The menu bar's and project tree's own scales are applied in
    // buildMainWindow(), where those widgets exist.
    applyUiFontScale(static_cast<int>(appSettings->uiFontScales().ui));
    // Build the language registry from what the config directory holds and
    // which languages the user turned off, before the first file can be
    // opened — otherwise a disabled language would come back every restart.
    appSettings->reloadLanguages();

    SplashScreen splash(appSettings->themeName());
    splash.show();

    QMainWindow *window =
      buildMainWindow(appSettings, [&splash](int step, const QString &text) {
          splash.setStage(step, text);
      });
    window->show();
    // Closes the splash exactly when the main window is up — no timer, no gap.
    splash.finish(window);
    e2eMark("{\"ev\":\"main_window_shown\"}");

    return QApplication::exec();
}

} // namespace ui_shell
