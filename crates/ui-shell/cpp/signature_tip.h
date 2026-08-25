#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QPoint>

class QWidget;

namespace ui_shell {

// F2-11: the signature-help popup driven by `(` and `,` while typing an
// argument list. Reuses `QToolTip::showText`/`hideText` at the exact
// placement hover already uses (`main_window.cpp`'s
// `hoverSignatureReady` handler) — a second frameless popup widget would
// just be a second set of placement bugs to chase, and `QToolTip` already
// supports the rich text (`<b>` around the active parameter) this needs.
void showSignatureTip(QWidget *editor, const QPoint &globalPos, const FfiSignatureHelp &help);

void hideSignatureTip();

} // namespace ui_shell
