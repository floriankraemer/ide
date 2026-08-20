#pragma once

#include <QColor>
#include <QPalette>
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

// The QSS above only reaches widgets we wrote selectors for. Qt Advanced
// Docking System ships its own stylesheet, applied to the CDockManager
// widget itself, and every color in it resolves through `palette(...)` roles
// — as does `find_bar.cpp`. A widget-level sheet beats the application one
// on equal specificity, so dock chrome can only be themed by giving the
// application a palette that matches the stylesheet (issue #11).
QPalette paletteForTheme(const QString &themeName);

// Applies both halves of a theme to the running QApplication. Callers should
// use this rather than setStyleSheet() alone, so palette and QSS can never
// drift apart.
void applyTheme(const QString &themeName);

} // namespace ui_shell
