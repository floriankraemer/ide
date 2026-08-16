#pragma once

#include <QString>

namespace ui_shell {

// Chrome-wide stylesheet (menus, tabs, tree, scrollbars, splitter). Editor
// text colors are QPalette-driven, not QSS (A3) — kept separate here.
QString darculaStyleSheet();

} // namespace ui_shell
