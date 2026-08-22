#pragma once

#include "ui-shell/src/bridge.cxxqt.h"

#include <functional>

class QWidget;

namespace ui_shell {

// Settings > Languages (task G3): every language the editor knows, where it
// came from, and — for the ones that failed — a sentence saying what is
// wrong and a button that does something about it.
//
// Humble view (ADR-0002): which rows exist, which source each belongs to,
// what its status word is and what its failure means in English are all
// `LanguageCatalog` calls into `settings-model`, which maps
// `syntax_core::runtime::LoadErrorKind` to the wording. This file never
// renders a raw error string, because it never receives one.
//
// `openFile` opens a path in the editor behind the dialog — the one
// genuinely actionable button for a broken query or manifest.
//
// `languagesChanged` is called after a language is turned off or back on,
// so the editors behind the dialog re-resolve their language instead of
// waiting for a restart.
QWidget *buildLanguagesPage(QWidget *parent,
                            LanguageCatalog *catalog,
                            std::function<void(const QString &)> openFile,
                            std::function<void()> languagesChanged);

} // namespace ui_shell
