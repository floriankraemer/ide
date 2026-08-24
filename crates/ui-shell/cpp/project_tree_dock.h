#pragma once

#include <QString>
#include <functional>

class AiChat;
class ProjectTreeModel;

class QMainWindow;
class QTreeView;

namespace ads {
class CDockAreaWidget;
class CDockManager;
class CDockWidget;
} // namespace ads

namespace ui_shell {

class AiChatPanel;

// Everything the project tree's context menu acts on.
//
// A struct rather than eight parameters: the list is long because the menu
// reaches the AI chat, and a positional call of that length is one
// transposition away from a bug the compiler cannot see.
struct ProjectTreeActions
{
    QMainWindow *window;
    AiChat *aiChat;
    AiChatPanel *aiChatPanel;
    ads::CDockWidget *aiChatDock;
    ads::CDockWidget *classViewDock;
    ads::CDockManager *dockManager;
    // Opening a file is EditorTabs' job, and EditorTabs is private to
    // main_window.cpp — a callback keeps it that way, the same shape the
    // search results panel already takes.
    std::function<void(const QString &)> openFile;
};

// Builds the Project dock: the tree view, the icon-decoration proxy between
// it and the model, and the dock widget itself, docked left of `editorArea`.
// Returns the tree view, which the caller needs for the interface font
// scale.
QTreeView *createProjectTreeDock(ads::CDockManager *dockManager,
                                 ads::CDockAreaWidget *editorArea,
                                 ProjectTreeModel *treeModel);

// Wires the tree's gestures: click to open, right-click for the
// create/rename/delete/attach menu (US-2b). Separate from construction only
// because the panels the menu reaches are built after the dock is.
void wireProjectTree(QTreeView *treeView,
                     ProjectTreeModel *treeModel,
                     const ProjectTreeActions &actions);

// Shows the AI chat dock, putting it back in its tab strip first if a
// restored layout left it homeless.
//
// ADS flags a dock absent from a saved layout as unassigned
// (DockManager::restoreDockWidgetsOpenState): closed, un-parented, no dock
// area. Reopening one in that state takes CDockWidget's floating path, so a
// user whose window_state predates this dock would get a detached window
// rather than the tab beside Class View that buildCentralWidget arranged.
// Both the tree's context menu and the menu bar go through here so "show the
// panel" means the same thing wherever it is asked for.
void showAiChatDock(ads::CDockManager *dockManager,
                    ads::CDockWidget *aiChatDock,
                    ads::CDockWidget *classViewDock);

} // namespace ui_shell
