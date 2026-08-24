#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QFont>

#include <functional>

class QWidget;

namespace ui_shell {

// Settings > Syntax Colors (task T4): the per-language token colour table
// and the control strip that edits it, built on the Keymap page's shape —
// grouped QTreeWidget, inert group headers, bold for "overridden here", a
// confirming QMessageBox for anything destructive.
//
// Humble view (ADR-0002): which rows exist, which family each belongs to,
// what its sample fragment is, where its value comes from and whether a
// reset would do anything are all `SyntaxColorEditor` calls into
// `settings-model`/`syntax-core`. This file paints them.
//
// `sampleFont` is the editor font, so each row's Sample cell previews the
// scope in the face it will actually be drawn in. `onChanged` is called
// after every edit — the page applies live, so the caller re-highlights the
// open editors behind the dialog.
QWidget *buildSyntaxColorsPage(QWidget *parent,
                               SyntaxColorEditor *editor,
                               const QFont &sampleFont,
                               std::function<void()> onChanged);

} // namespace ui_shell
