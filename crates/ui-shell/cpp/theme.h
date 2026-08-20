#pragma once

#include <QColor>
#include <QString>

namespace ui_shell {

// The handful of theme colors a widget needs when it paints itself instead
// of being styled by QSS — today only the splash screen, which exists
// before the stylesheet can reach it. Kept next to the stylesheets so the
// two cannot drift apart.
struct ThemeColors
{
    QColor background;
    QColor foreground;
    QColor accent;
};

ThemeColors colorsForTheme(const QString &themeName);

// Chrome-wide stylesheets (menus, tabs, tree, scrollbars, splitter). Editor
// text colors are QPalette-driven, not QSS (A3) — kept separate here.
QString darculaStyleSheet();
QString lightStyleSheet();

// Picks the matching stylesheet for a theme name from `app-config::Settings`
// (T2). Any name other than "light" falls back to Darcula — the same
// default `Settings::theme_name()` already resolves an unset theme to, so
// an unrecognized value degrades to the default rather than an unstyled
// window.
QString styleSheetForTheme(const QString &themeName);

} // namespace ui_shell
