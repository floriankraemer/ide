#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QHash>
#include <QString>

class QAction;
class QMainWindow;

namespace ui_shell {

class AiChatPanel;
class DockRegistry;
class EditorTabs;

// ADR-0021: routes the AI panel to the editor it has no direct reference
// to — the buffer the user is looking at (`setCurrentTextProvider`), what
// "Apply" means for a proposed change (`setApplyHandler`), and the
// agent-mode tool signals (`toolOpenedTab`/`toolEditedBuffer`/
// `toolSavedBuffer`) that reach the widget the same way MCP's edit_buffer
// does. Its own translation unit for the same reason `vcs_menu.cpp`/
// `run_menu.cpp` are: `main_window.cpp` sits at its 1200-line ceiling
// (ADR-0025).
void wireAiChatToEditor(QMainWindow *window, AiChat *aiChat, AiChatPanel *aiChatPanel,
                         EditorTabs *editorTabs, SearchModel *searchModel);

// ADR-0021: the "&AI" menu — attaching the current selection or file to the
// chat, starting a new conversation, and the panel's own toggle shortcut.
void buildAiMenu(QMainWindow *window, AiChat *aiChat, EditorTabs *editorTabs,
                  AppSettings *appSettings, QHash<QString, QAction *> &actions,
                  DockRegistry *docks, AiChatPanel *aiChatPanel);

} // namespace ui_shell
