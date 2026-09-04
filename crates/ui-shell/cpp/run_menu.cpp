#include "run_menu.h"

#include "dock_layout.h"
#include "e2e_mark.h"
#include "keymap_page.h"
#include "run_config_dialog.h"
#include "code_editor.h"
#include "editor_tabs.h"
#include "run_console_panel.h"

#include <QAction>
#include <QMainWindow>
#include <QMenu>
#include <QMenuBar>

namespace ui_shell {

void buildRunMenu(QMainWindow *window, RunService *runService, RunConfigEditor *runConfigEditor,
                   AppSettings *appSettings, QHash<QString, QAction *> &actions,
                   DockRegistry *docks, RunConsolePanel *runConsolePanel,
                   ProjectTreeModel *treeModel, EditorTabs *editorTabs, QMenu *viewMenu)
{
    // Detect run configurations (Cargo.toml, package.json, Makefile) the
    // moment a project opens, same lifecycle hook LanguageService and the
    // search index already key off of — the picker has something to run
    // without the user asking for a scan first.
    QObject::connect(treeModel, &ProjectTreeModel::projectOpened, runService,
                      [runService](const QString &) { runService->detectConfigurations(); });

    // Whatever launched it, a started console is worth looking at — reveal
    // the dock without stealing focus from what the user was already doing
    // any more forcefully than opening a new tab in it does.
    QObject::connect(runService, &RunService::consoleStarted, window,
                      [docks](quint64, const QString &) {
                          docks->show(QStringLiteral("runConsole"));
                      });

    QMenu *runMenu = window->menuBar()->addMenu(QObject::tr("&Run"));
    // A top-level menu bar entry never goes through `exec()`, so
    // `aboutToShow`/`aboutToHide` are the only signal an E2E flow has that
    // it is safe to send keystrokes into it — same reasoning as
    // `vcs_menu.cpp`'s "V&CS" markers.
    QObject::connect(runMenu, &QMenu::aboutToShow, runMenu,
                      []() { e2eMark("{\"ev\":\"dialog_shown\",\"name\":\"run_menu\"}"); });
    QObject::connect(runMenu, &QMenu::aboutToHide, runMenu,
                      []() { e2eMark("{\"ev\":\"dialog_closed\",\"name\":\"run_menu\"}"); });

    QAction *runAction = registerAction(runMenu, QStringLiteral("run.run"), QObject::tr("Run"),
                                        appSettings, actions);
    QObject::connect(runAction, &QAction::triggered, runConsolePanel,
                      [runConsolePanel]() { runConsolePanel->runSelected(); });

    // IntelliJ's "run context configuration": run whatever the focused
    // editor's file is, the keyboard door onto the same gutter icon
    // `editor_tabs_run.cpp` wires up.
    QAction *runContextAction = registerAction(runMenu, QStringLiteral("run.runContext"),
                                                QObject::tr("Run File"), appSettings, actions);
    QObject::connect(runContextAction, &QAction::triggered, editorTabs, [editorTabs]() {
        editorTabs->requestRunFor(qobject_cast<CodeEditor *>(editorTabs->currentEditor()));
    });

    QAction *stopAction = registerAction(runMenu, QStringLiteral("run.stop"), QObject::tr("Stop"),
                                         appSettings, actions);
    QObject::connect(stopAction, &QAction::triggered, runConsolePanel,
                      [runConsolePanel]() { runConsolePanel->stopSelected(); });

    QAction *rerunAction = registerAction(runMenu, QStringLiteral("run.rerun"),
                                          QObject::tr("Rerun"), appSettings, actions);
    QObject::connect(rerunAction, &QAction::triggered, runConsolePanel,
                      [runConsolePanel]() { runConsolePanel->rerunSelected(); });

    QAction *selectConfigAction =
      registerAction(runMenu, QStringLiteral("run.selectConfiguration"),
                      QObject::tr("Select Run Configuration..."), appSettings, actions);
    QObject::connect(selectConfigAction, &QAction::triggered, runConsolePanel, [docks,
                                                                                runConsolePanel]() {
        docks->show(QStringLiteral("runConsole"));
        runConsolePanel->focusConfigSelector();
    });

    QAction *editConfigsAction =
      registerAction(runMenu, QStringLiteral("run.editConfigurations"),
                      QObject::tr("Edit Configurations..."), appSettings, actions);
    QObject::connect(editConfigsAction, &QAction::triggered, window,
                      [window, runConfigEditor]() { showRunConfigDialog(window, runConfigEditor); });

    QAction *viewRunConsoleAction =
      registerAction(viewMenu, QStringLiteral("view.runConsole"), QObject::tr("Run Console"),
                      appSettings, actions);
    QObject::connect(viewRunConsoleAction, &QAction::triggered, window,
                      [docks]() { docks->show(QStringLiteral("runConsole")); });
}

} // namespace ui_shell
