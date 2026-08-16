#pragma once

namespace ui_shell {

// Builds and shows the native Qt6 Widgets main window (menu bar per US-5)
// and runs the Qt event loop until the window is closed.
// Returns the process exit code (QApplication::exec()'s return value).
int run_app();

} // namespace ui_shell
