#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

class QWidget;

namespace ui_shell {

// F4-12: "Edit Configurations..." — a list of the project's run
// configurations on the left, a name/program/args/cwd/env form on the
// right, Add/Remove, and Save/Cancel.
//
// Same commit/discard convention `showSettingsDialog` uses for its pages
// (`KeymapEditor`, `LanguageServerEditor`, ...): `beginEdit()` loads the
// draft when the dialog opens, Save calls `validate()` then `commit()` (a
// validation refusal keeps the dialog open with the message shown, rather
// than closing on invalid data), Cancel calls `revert()`. Modal and
// standalone rather than a Settings page, since editing run configurations
// is an action reached from the Run menu, not a persistent preference.
void showRunConfigDialog(QWidget *parent, RunConfigEditor *editor);

} // namespace ui_shell
