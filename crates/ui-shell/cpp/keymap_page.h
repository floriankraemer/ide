#pragma once

#include "ui-shell/src/bridge.cxxqt.h"

class QWidget;

namespace ui_shell {

// Settings > Keymap: the table of rebindable actions and the controls that
// change them. Split out of main_window.cpp because — unlike the Appearance
// and Editor pages — it needs nothing from EditorTabs, only the KeymapEditor
// QObject holding the dialog's draft keymap.
//
// Humble view (ADR-0002): every rule (default fallback, which actions a
// shortcut clashes with, what stealing does) is a KeymapEditor call into
// app-config. This file only renders rows and asks the user to confirm.
QWidget *buildKeymapPage(QWidget *parent, KeymapEditor *editor);

} // namespace ui_shell
