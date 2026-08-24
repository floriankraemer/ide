#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

class QWidget;

namespace ui_shell {

// Settings > Editing (F1-17): tab width, spaces-vs-tabs, trim-on-save and
// final-newline, plus one language's override at a time.
//
// Commits on OK, like Keymap and Language Servers: `EditingEditor::beginEdit`
// starts the draft this page edits, and every field change here is pushed
// into it immediately through `setGlobalRow`/`setLanguageRow` — the widgets
// hold no state of their own, `EditingDraft` does.
//
// Humble view (ADR-0002): which fields a language may override, what a
// nonsensical value is worth and what the resolved tab width for a preview
// would be are all `EditingEditor` calls into `settings-model`. This file
// only lays out the two rows and asks the questions.
QWidget *buildEditingPage(QWidget *parent, EditingEditor *editor);

// The dialog's OK handler: `problems()` first, since a setting that parses
// and then does nothing is worse than one refused, matching
// `commitAiProvidersPage`'s shape — the one other page that can say no.
bool commitEditingPage(QWidget *parent, EditingEditor *editor);

} // namespace ui_shell
