#include "main_window.h"

#include "ai_chat_panel.h"
#include "appearance_page.h"
#include "class_view_panel.h"
#include "code_editor.h"
#include "declaration_navigator.h"
#include "e2e_mark.h"
#include "editing_actions.h"
#include "editor_tabs.h"
#include "find_bar.h"
#include "find_usages_panel.h"
#include "hex_viewer.h"
#include "icon_cache.h"
#include "ide_main_window.h"
#include "keymap_page.h"
#include "search_everywhere_dialog.h"
#include "problems_panel.h"
#include "icon_decoration_proxy.h"
#include "project_tree_dock.h"
#include "recent_projects_menu.h"
#include "refactor_controller.h"
#include "refactor_preview_dialog.h"
#include "search_results_panel.h"
#include "settings_dialog.h"
#include "splash_screen.h"
#include "syntax_highlighter.h"
#include "terminal_widget.h"
#include "theme.h"
#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include "DockManager.h"
#include "DockWidget.h"

#include <QApplication>
#include <QByteArray>
#include <QSet>
#include <QTimer>
#include <QToolButton>
#include <QToolTip>
#include <QColor>
#include <QDialog>
#include <QFileDialog>
#include <QFont>
#include <QHash>
#include <algorithm>
#include <cstdint>
#include <functional>
#include <QLabel>
#include <QProgressBar>
#include <memory>
#include <QMainWindow>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QPlainTextEdit>
#include <QStringList>
#include <QSplitter>
#include <QStatusBar>
#include <QTabWidget>
#include <QtGui/QTextDocument>
#include <QTreeView>
#include <QWidget>

namespace ui_shell {

namespace {

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
    auto *editingEditor = new EditingEditor(window);
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

    // Built once, outside the lambda, and captured whole: fourteen separate
    // captures is what the parameter object exists to replace (see
    // SettingsContext). Every member is a pointer or a handle that outlives
    // the window, so a by-value capture holds nothing that can dangle.
    const SettingsContext settingsContext{
      appSettings,
      editorTabs,
      keymapEditor,
      actions,
      docManager,
      mcpStatus,
      syntaxColorEditor,
      languageCatalog,
      languageServerEditor,
      editingEditor,
      languageService,
      aiProviderEditor,
      aiChat,
      pluginCatalog,
      uiFontTargets,
    };
    QObject::connect(preferencesAction, &QAction::triggered, window,
                      [window, settingsContext]() {
                          showSettingsDialog(window, settingsContext);
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

    // F1-16: multi-caret, comment toggling, the line operations,
    // expand/shrink selection and the bracket jump. Its own translation
    // unit; every entry routes through EditorTabs to EditorOps.
    buildEditingActions(editMenu, window, appSettings, *actions, editorTabs);

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

    refactorMenu->addSeparator();
    QAction *reformatAction = registerAction(refactorMenu, QStringLiteral("code.reformat"),
                                             QObject::tr("Reformat Code"), appSettings, *actions);
    QObject::connect(reformatAction, &QAction::triggered, window, [editorTabs, languageService]() {
        const QString path = editorTabs->currentPath();
        if (path.isEmpty()) {
            return;
        }
        languageService->requestFormatting(path, editorTabs->documentRevision());
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
