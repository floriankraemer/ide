#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <functional>

class QWidget;

namespace ui_shell {

class EditorTabs;

// Settings > Editor: the editor font and the three editor colours.
//
// Everything on this page previews live through `EditorTabs`, so — like
// `appearance_page.*`, whose shape this follows — the page owns both halves
// of what the dialog's buttons mean for it: `commit` persists what is
// already on screen, `revert` puts back what was in force when it opened.
// Keeping the pair next to the widgets is what stops a Cancel path from
// drifting out of step with the controls it is supposed to undo.
struct EditorPage
{
    QWidget *widget;
    std::function<void()> commit;
    std::function<void()> revert;
};

// Settings > Editor. Humble view (ADR-0002): the font and the colours are
// read from and written back to `AppSettings`, which is where a default, a
// clamp or "empty means derived from the theme" is decided; this file only
// renders the controls and forwards what the user picked.
EditorPage buildEditorPage(QWidget *parent, AppSettings *appSettings, EditorTabs *editorTabs);

} // namespace ui_shell
