#pragma once

#include <QColor>
#include <QPalette>
#include <QString>

class QWidget;

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

// The one definition of what a tab looks like. Every tab bar in the product
// renders from the rules `tabStyleSheet()` and `dockStyleSheet()` emit —
// the editor tab bar, the dock panel tabs and the Search Everywhere filter
// strip — so a metric changed there changes all of them at once. Only the
// colours differ per theme.
struct TabColors
{
    QColor bar;          // the strip behind the tabs
    QColor tab;          // an unselected tab's body
    QColor tabText;
    QColor selected;     // the selected tab's body
    QColor selectedText;
    QColor hover;        // an unselected tab under the mouse
    QColor hoverText;
    QColor accent;       // the marker on the selected tab's top edge
    QColor closeHover;   // the close button's hover square
    QColor pane;         // the page area below the strip
    QColor paneBorder;   // an invalid QColor means borderless
    QColor divider;      // the splitter handles between docked panes
};

TabColors tabColorsForTheme(const QString &themeName);

// The QTabBar/QTabWidget half, concatenated into each theme's sheet below.
QString tabStyleSheet(const TabColors &colors);

// The same look in the docking system's own selectors, plus the splitter
// handles between its panes. Qt gives a widget's own stylesheet priority over
// the application's however specific the latter is, and ADS installs one on
// its dock manager — so applyTheme() appends these to that sheet rather than
// to qApp's.
QString dockStyleSheet(const TabColors &colors);

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
// on equal specificity, so dock chrome is themed by giving the application a
// palette that matches the stylesheet (issue #11) — everything except the
// dock tabs and splitters, which applyTheme() reaches by appending
// dockStyleSheet() to that sheet.
QPalette paletteForTheme(const QString &themeName);

// Applies both halves of a theme to the running QApplication. Callers should
// use this rather than setStyleSheet() alone, so palette and QSS can never
// drift apart.
void applyTheme(const QString &themeName);

// Scales the whole application's default UI font to `percent` of the font
// Qt picked for the platform (100 = unchanged). Widgets that were never
// given a font of their own follow it; the two that were — see
// applyWidgetFontScale() — keep their own scale.
//
// Always relative to the font captured on the first call, so repeated live
// previews from the Settings dialog scale the original rather than
// compounding what the previous preview left behind.
void applyUiFontScale(int percent);

// Same scale, applied to one widget and its children (the menu bar and its
// popup menus, the project tree). An explicitly set font wins over the
// application one, which is exactly what keeps these two independent of
// applyUiFontScale().
void applyWidgetFontScale(QWidget *widget, int percent);

// Nudges `base` away from itself so a band or column drawn in the result
// reads as a subtle tint on both dark and light editor backgrounds. Used by
// every widget that paints its own chrome against QPalette::Base — the
// editor's gutter and current-line band, and the hex viewer's columns.
QColor tinted(const QColor &base, int darkFactor, int lightFactor);

} // namespace ui_shell
