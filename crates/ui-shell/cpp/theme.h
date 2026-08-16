#pragma once

#include <QString>

namespace ui_shell {

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
