#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <functional>

class QWidget;

namespace ui_shell {

// The widgets that carry an interface font scale of their own, rather than
// following the application font.
struct UiFontTargets
{
    QWidget *menuBar;
    QWidget *projectTree;
    QWidget *indexBar;
};

// One sink for all three scales, used at startup and by the Settings
// dialog's live preview and its Cancel path alike.
void applyUiFontScales(const FfiUiFontScales &scales, const UiFontTargets &targets);

// What the page has to tell the window behind it after changing something
// that is applied live. Each one is a repaint, never a decision.
struct AppearanceHooks
{
    // The colour theme changed, so the editors re-run their highlighting.
    std::function<void()> refreshEditors;
    // The icon art changed — a different pack, or the same pack's light
    // variants — so every cached QIcon is stale.
    std::function<void()> refreshIcons;
    // The dialog scaled under its own feet and has to re-measure itself.
    std::function<void()> relayout;
};

// Settings > Appearance: the colour theme, the icon theme (P7) and the three
// interface font scales.
//
// Everything on this page previews live, so the page owns both halves of
// what the dialog's buttons mean for it — `commit` persists what is already
// on screen, `revert` puts back what was in force when it opened. Keeping
// the pair next to the widgets is what stops a Cancel path from drifting out
// of step with the controls it is supposed to undo.
//
// Humble view (ADR-0002): the icon-theme combo is filled from
// `IconProvider::iconThemes()` and every switch is a call — which pack is
// offered, what an unknown id falls back to, and which art a colour theme
// wants are all decided in Rust.
struct AppearancePage
{
    QWidget *widget;
    std::function<void()> commit;
    std::function<void()> revert;
};

AppearancePage buildAppearancePage(QWidget *parent,
                                   AppSettings *appSettings,
                                   const UiFontTargets &targets,
                                   AppearanceHooks hooks);

} // namespace ui_shell
