#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QHash>
#include <QString>

class QAction;
class QMenu;
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

// Adds a menu action whose shortcut comes from the persisted keymap rather
// than a literal QKeySequence, and records it under its stable action id so
// Settings > Keymap can re-apply a rebinding without rebuilding the menus.
//
// `id` must be one of app_config::ACTIONS' ids — that catalog, not the view,
// is where an action's default shortcut lives.
QAction *registerAction(QMenu *menu, const QString &id, const QString &text,
                        AppSettings *appSettings, QHash<QString, QAction *> &actions);

// Re-reads every registered action's shortcut from settings — run after the
// Keymap page commits, so a rebinding takes effect without a restart. An
// action left unbound gets an empty QKeySequence, which Qt renders as no
// accelerator at all.
void applyKeymap(const QHash<QString, QAction *> &actions, AppSettings *appSettings);

} // namespace ui_shell
