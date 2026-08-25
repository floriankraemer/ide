#pragma once

#include <QHash>
#include <QString>
#include <functional>

class AiChat;
class ProjectTreeModel;

class QAction;
class QMainWindow;
class QMenu;
class QTreeView;
class AppSettings;

namespace ads {
class CDockAreaWidget;
class CDockManager;
} // namespace ads

namespace ui_shell {

class AiChatPanel;
class DockRegistry;

// Everything the project tree's context menu acts on.
//
// A struct rather than several parameters: the list is long because the menu
// reaches the AI chat, and a positional call of that length is one
// transposition away from a bug the compiler cannot see.
struct ProjectTreeActions
{
    QMainWindow *window;
    AiChat *aiChat;
    AiChatPanel *aiChatPanel;
    // Reveals the AI Chat dock (F0-7): re-adds it to its default placement
    // first if a restored layout left it homeless, same as every other dock.
    DockRegistry *docks;
    // Opening a file is EditorTabs' job, and the tree does not depend on
    // editor_tabs.h to say so — a callback keeps it that way, the same
    // shape the search results panel already takes.
    std::function<void(const QString &)> openFile;
};

// Builds the Project dock: the tree view, the icon-decoration proxy between
// it and the model, and the dock widget itself, docked left of `editorArea`.
// Returns the tree view, which the caller needs for the interface font
// scale.
QTreeView *createProjectTreeDock(ads::CDockManager *dockManager,
                                 ads::CDockAreaWidget *editorArea,
                                 ProjectTreeModel *treeModel,
                                 DockRegistry *docks);

// Wires the tree's gestures: click to open, right-click for the
// create/rename/delete/attach menu (US-2b). Separate from construction only
// because the panels the menu reaches are built after the dock is.
void wireProjectTree(QTreeView *treeView,
                     ProjectTreeModel *treeModel,
                     const ProjectTreeActions &actions);

// Adds the `view.projectTree` action to `viewMenu` so the dock can be raised
// again after its "x" is closed (#117) — same registerAction/DockRegistry
// shape as every other dock's View-menu entry, split out here rather than
// main_window.cpp only to stay under that file's line ceiling.
void wireProjectTreeViewAction(QMenu *viewMenu,
                               DockRegistry *docks,
                               AppSettings *appSettings,
                               QHash<QString, QAction *> &actions);

} // namespace ui_shell
