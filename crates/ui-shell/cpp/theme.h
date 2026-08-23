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

// The semantic colours every status and severity label in the product uses
// (`docs/design/language-platform-ui.md` section 1). Kept next to the
// stylesheets for the same reason `ThemeColors` is: a colour picked in code
// and a colour picked in QSS must not drift.
//
// Meaning is never carried by hue alone — every one of these is applied to a
// word (`Error`, `Running`, `Disabled after crash`), so a greyscale
// screenshot and a screen reader carry the same information. Each value
// clears WCAG AA 4.5:1 against its theme's list background.
struct SemanticColors
{
    QColor error;
    QColor warning;
    QColor info;
    QColor ok;
    QColor muted;
};

SemanticColors semanticColorsForTheme(const QString &themeName);

// The same, for whatever theme is active — what a widget building rows wants.
SemanticColors semanticColors();

// Chrome-wide stylesheets (menus, tabs, tree, scrollbars, splitter). Editor
// text colors are QPalette-driven, not QSS (A3) — kept separate here.
QString darculaStyleSheet();
QString lightStyleSheet();
QString vscodeDarkStyleSheet();

// Picks the matching stylesheet for a theme name from `app-config::Settings`
// (T2). Any name other than "light"/"vscode-dark" falls back to Darcula —
// the same default `Settings::theme_name()` already resolves an unset theme
// to, so an unrecognized value degrades to the default rather than an
// unstyled window.
QString styleSheetForTheme(const QString &themeName);

// The theme name last passed to applyTheme(). Widgets that pick colors in
// code rather than through QSS or the palette — today only the syntax
// highlighter's token colors — have no other route to the active theme.
QString activeThemeName();

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

// Nudges `base` away from itself so a band or column drawn in the result
// reads as a subtle tint on both dark and light editor backgrounds. Used by
// every widget that paints its own chrome against QPalette::Base — the
// editor's gutter and current-line band, and the hex viewer's columns.
QColor tinted(const QColor &base, int darkFactor, int lightFactor);

} // namespace ui_shell
